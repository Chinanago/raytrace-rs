use super::queue::{Traversal, Work, WorkQueue};
use super::{IntersectionContext, PathEvent, Report, TraceOptions};
use crate::geometry::{Miss, dielectric_with_reflectance, intersect_ordered};
use crate::model::{Interaction, PreparedScene, Ray};

// The cached path shares the state machine to retain scalar accumulation order.
#[allow(clippy::too_many_arguments)]
pub(super) fn trace_one<const SEQUENTIAL: bool, const CACHED: bool>(
    scene: &PreparedScene,
    launch: Ray,
    report: &mut Report,
    options: TraceOptions,
    record_paths: bool,
    optics: &[(f64, f64)],
    traversal: &mut Traversal<'_>,
    packet: Option<(&[[crate::packet::Step; crate::packet::WIDTH]], usize)>,
) {
    // The sequential path has at most one successor. Keep its state local so
    // LLVM can scalarize it; only NSC rays need a reusable priority queue.
    let mut sequential = WorkQueue::Sequential(None);
    let stack = if SEQUENTIAL {
        &mut sequential
    } else {
        &mut traversal.stack
    };
    report.budget.launched += launch.weight;
    // Resolve the strongest pending NSC branches first. A launch-wide event
    // allowance must not strand a strong reflection behind many weak descendants.
    stack.start(Work {
        ray: launch,
        next_surface: 0,
        depth: 0,
    });
    let mut events = 0;
    let mut path_events = 0;
    while let Some(work) = stack.pop() {
        let Work {
            mut ray,
            next_surface,
            depth,
        } = work;
        if ray.weight == 0.0 {
            continue;
        }
        if ray.weight < launch.weight * scene.spec.settings.min_relative_branch_weight {
            report.budget.branch_cutoff += ray.weight;
            let previous = next_surface.checked_sub(1);
            report.diagnose(ray, "branch_cutoff", previous, depth, None, None);
            report.terminal(
                ray,
                "branch_cutoff",
                previous,
                depth,
                options.keep_terminals,
            );
            continue;
        }
        if depth >= scene.spec.settings.max_depth
            || events >= scene.spec.settings.max_events_per_launch
        {
            report.budget.interaction_limit += ray.weight;
            let previous = next_surface.checked_sub(1);
            report.diagnose(ray, "interaction_limit", previous, depth, None, None);
            report.terminal(
                ray,
                "interaction_limit",
                previous,
                depth,
                options.keep_terminals,
            );
            continue;
        }
        let selection = if SEQUENTIAL {
            match scene.surfaces.get(next_surface) {
                Some(s) => (if CACHED {
                    let (steps, lane) = packet.expect("cached packet");
                    Ok(steps[next_surface][lane].hit)
                } else {
                    intersect_ordered(s, ray.position_mm, ray.direction)
                })
                .map(|h| Some((next_surface, h)))
                .map_err(|e| (e, Some(next_surface))),
                None => Ok(None),
            }
        } else {
            crate::acceleration::nearest(
                &scene.surfaces,
                traversal.surface_index,
                ray.position_mm,
                ray.direction,
            )
        };
        let (index, hit) = match selection {
            Ok(Some(value)) => value,
            Ok(None) => {
                report.budget.escaped += ray.weight;
                report.terminal(ray, "escaped", None, depth, options.keep_terminals);
                continue;
            }
            Err((Miss::Geometry, _)) => {
                report.budget.geometric_miss += ray.weight;
                report.surfaces[next_surface].geometric_miss_weight += ray.weight;
                report.terminal(
                    ray,
                    "geometric_miss",
                    Some(next_surface),
                    depth,
                    options.keep_terminals,
                );
                continue;
            }
            Err((Miss::Numerical, surface)) => {
                report.budget.numerical_failure += ray.weight;
                report.diagnose(ray, "numerical_failure", surface, depth, None, None);
                report.terminal(
                    ray,
                    "numerical_failure",
                    surface,
                    depth,
                    options.keep_terminals,
                );
                continue;
            }
        };
        events += 1;
        report.surface_intersections += 1;
        if record_paths && path_events < 256 {
            report.paths.push(PathEvent {
                id: ray.id,
                depth,
                surface: index,
                start_mm: ray.position_mm,
                end_mm: hit.point,
                incident_weight: ray.weight,
            });
            path_events += 1;
        }
        let (n_before, absorption) = optics[ray.medium];
        let origin_mm = ray.position_mm;
        let before_bulk = ray.weight;
        if absorption != 0.0 {
            ray.weight *= (-absorption * hit.distance_mm).exp();
        }
        report.budget.bulk_absorbed += before_bulk - ray.weight;
        ray.optical_path_mm += n_before * hit.distance_mm;
        ray.position_mm = hit.point;
        let surface = &scene.surfaces[index];
        report.surfaces[index].encounters += 1;
        report.surfaces[index].incident_weight += ray.weight;
        match surface.spec.interaction {
            Interaction::Absorb => {
                report.budget.surface_absorbed += ray.weight;
                report.surfaces[index].absorbed_weight += ray.weight;
                report.terminal(
                    ray,
                    "absorbed",
                    Some(index),
                    depth + 1,
                    options.keep_terminals,
                );
            }
            Interaction::Detector { pixels } => {
                let detector = &mut report.detectors[surface.detector_index.unwrap()];
                let [hx, hy] = detector.half_extent_mm;
                let x = (((hit.local_point.0[0] / hx + 1.0) * 0.5 * pixels[0] as f64) as usize)
                    .min(pixels[0] - 1);
                let y = (((hit.local_point.0[1] / hy + 1.0) * 0.5 * pixels[1] as f64) as usize)
                    .min(pixels[1] - 1);
                detector.weights[y * pixels[0] + x] += ray.weight;
                report.budget.detected += ray.weight;
                report.surfaces[index].detected_weight += ray.weight;
                report.terminal(
                    ray,
                    "detected",
                    Some(index),
                    depth + 1,
                    options.keep_terminals,
                );
            }
            Interaction::Mirror { reflectance } => {
                let mut normal = hit.normal;
                if !normal.finite()
                    || ray.direction.dot(normal) * ray.direction.dot(hit.geometric_normal) < 0.
                    || (ray.direction.dot(hit.geometric_normal)
                        - 2. * ray.direction.dot(normal) * normal.dot(hit.geometric_normal))
                        * ray.direction.dot(hit.geometric_normal)
                        > 1e-14
                {
                    normal = hit.geometric_normal;
                    report.surfaces[index].normal_fallbacks += 1;
                }
                let reflectance = surface.spec.coating.as_ref().map_or(reflectance, |c| {
                    c.side(ray.direction.dot(hit.geometric_normal) > 0.0)
                        .evaluate(
                            ray.wavelength_um,
                            ray.direction.dot(normal).abs(),
                            n_before,
                            n_before,
                            false,
                        )[0]
                });
                let absorbed = ray.weight * (1.0 - reflectance);
                report.budget.surface_absorbed += absorbed;
                report.surfaces[index].absorbed_weight += absorbed;
                report.surfaces[index].reflected_weight += ray.weight * reflectance;
                ray.weight *= reflectance;
                ray.direction = (ray.direction - normal * (2.0 * ray.direction.dot(normal))).unit();
                stack.push(Work {
                    ray,
                    next_surface: index + 1,
                    depth: depth + 1,
                });
            }
            Interaction::Dielectric {
                minus_medium,
                plus_medium,
                reflectance,
            } => {
                let (expected, after) = if ray.direction.dot(hit.geometric_normal) > 0.0 {
                    (minus_medium, plus_medium)
                } else {
                    (plus_medium, minus_medium)
                };
                if ray.medium != expected {
                    report.budget.medium_mismatch += ray.weight;
                    report.diagnose(
                        ray,
                        "medium_mismatch",
                        Some(index),
                        depth + 1,
                        Some(expected),
                        Some(IntersectionContext {
                            origin_mm,
                            distance_mm: hit.distance_mm,
                            normal: hit.normal,
                            geometric_normal: hit.geometric_normal,
                        }),
                    );
                    report.terminal(
                        ray,
                        "medium_mismatch",
                        Some(index),
                        depth + 1,
                        options.keep_terminals,
                    );
                    continue;
                }
                let n_after = optics[after].0;
                let mut normal = hit.normal;
                let bare_power = if surface.spec.coating.is_some() {
                    Some(0.0)
                } else {
                    reflectance
                };
                let mut result = if CACHED {
                    let (steps, lane) = packet.expect("cached packet");
                    steps[index][lane].interface.expect("cached dielectric")
                } else {
                    dielectric_with_reflectance(
                        ray.direction,
                        normal,
                        n_before,
                        n_after,
                        bare_power,
                    )
                };
                let incoming = ray.direction.dot(hit.geometric_normal);
                if !normal.finite()
                    || ray.direction.dot(normal) * incoming < 0.
                    || result.reflected.dot(hit.geometric_normal) * incoming > 1e-14
                    || result
                        .transmitted
                        .is_some_and(|t| t.dot(hit.geometric_normal) * incoming < -1e-14)
                {
                    normal = hit.geometric_normal;
                    result = dielectric_with_reflectance(
                        ray.direction,
                        normal,
                        n_before,
                        n_after,
                        bare_power,
                    );
                    report.surfaces[index].normal_fallbacks += 1;
                }
                let r = if result.transmitted.is_none() {
                    1.0
                } else {
                    reflectance.unwrap_or(result.reflectance)
                };
                let reflected_weight = ray.weight * r;
                let (reflected_weight, transmitted_weight) = if let Some(c) = &surface.spec.coating
                {
                    let [r, t] = c.side(incoming > 0.0).evaluate(
                        ray.wavelength_um,
                        ray.direction.dot(normal).abs(),
                        n_before,
                        n_after,
                        result.transmitted.is_none(),
                    );
                    let rw = ray.weight * r;
                    let tw = (ray.weight * t).min(ray.weight - rw);
                    let absorbed = (ray.weight - rw - tw).max(0.0);
                    report.budget.surface_absorbed += absorbed;
                    report.surfaces[index].absorbed_weight += absorbed;
                    (rw, tw)
                } else {
                    (reflected_weight, ray.weight - reflected_weight)
                };
                report.surfaces[index].reflected_weight += reflected_weight;
                report.surfaces[index].transmitted_weight += transmitted_weight;
                if result.transmitted.is_none() {
                    report.total_internal_reflections += 1;
                }
                if SEQUENTIAL && result.transmitted.is_none() {
                    ray.weight = reflected_weight;
                    report.budget.sequential_tir += ray.weight;
                    report.diagnose(ray, "sequential_tir", Some(index), depth + 1, None, None);
                    report.terminal(
                        ray,
                        "sequential_tir",
                        Some(index),
                        depth + 1,
                        options.keep_terminals,
                    );
                } else if !SEQUENTIAL {
                    if reflected_weight > 0.0 {
                        stack.push(Work {
                            ray: Ray {
                                direction: result.reflected,
                                weight: reflected_weight,
                                ..ray
                            },
                            next_surface: index + 1,
                            depth: depth + 1,
                        });
                    }
                } else {
                    report.budget.sequential_reflection_not_followed += reflected_weight;
                }
                if let Some(direction) = result.transmitted
                    && transmitted_weight > 0.0
                {
                    stack.push(Work {
                        ray: Ray {
                            direction,
                            medium: after,
                            weight: transmitted_weight,
                            ..ray
                        },
                        next_surface: index + 1,
                        depth: depth + 1,
                    });
                }
            }
        }
    }
}
