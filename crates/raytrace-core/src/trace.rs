use crate::Result;
use crate::acceleration::SurfaceIndex;
use crate::model::{Mode, PreparedScene, Ray};
use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

mod interaction;
mod optics;
mod queue;
mod report;

use interaction::trace_one;
use optics::OpticalCache;
use queue::{Traversal, WorkQueue};
pub use report::{
    Budget, DetectorMap, DiagnosticOverflow, IntersectionContext, PathEvent, Report, SurfaceStats,
    Terminal, TraceDiagnostic,
};

#[derive(Default)]
pub struct TraceControl {
    pub cancel: AtomicBool,
    pub completed: AtomicUsize,
    pub total: AtomicUsize,
}

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize)]
pub struct TraceOptions {
    pub keep_terminals: bool,
    /// Store every segment belonging to the first N launch rays in input order, including branches.
    pub path_launch_count: u64,
}

/// Rays must pass Ray::validate or originate from PreparedScene::generate_rays.
pub fn trace(scene: &PreparedScene, rays: &[Ray], options: TraceOptions) -> Report {
    trace_impl(
        scene,
        rays.len(),
        |range| rays[range].iter().copied().map(Ok),
        options,
        None,
    )
    .expect("explicit rays cannot fail generation")
    .expect("uncontrolled tracing cannot be cancelled")
}

/// A cancelled calculation returns no partial report; callers retain completed runs.
/// Progress advances after complete launch rays, including all their branches.
pub fn trace_controlled(
    scene: &PreparedScene,
    rays: &[Ray],
    options: TraceOptions,
    control: &TraceControl,
) -> Option<Report> {
    control.completed.store(0, Ordering::Relaxed);
    control.total.store(rays.len(), Ordering::Relaxed);
    trace_impl(
        scene,
        rays.len(),
        |range| rays[range].iter().copied().map(Ok),
        options,
        Some(control),
    )
    .expect("explicit rays cannot fail generation")
}

/// Trace source rays without retaining the launch array.
pub fn trace_generated(scene: &PreparedScene, options: TraceOptions) -> Result<Report> {
    generated_impl(scene, options, None)
        .map(|r| r.expect("uncontrolled tracing cannot be cancelled"))
}

pub fn trace_generated_controlled(
    scene: &PreparedScene,
    options: TraceOptions,
    control: &TraceControl,
) -> Result<Option<Report>> {
    generated_impl(scene, options, Some(control))
}

fn generated_impl(
    scene: &PreparedScene,
    options: TraceOptions,
    control: Option<&TraceControl>,
) -> Result<Option<Report>> {
    let generator = scene.ray_generator()?;
    if let Some(c) = control {
        c.completed.store(0, Ordering::Relaxed);
        c.total.store(generator.len(), Ordering::Relaxed);
        if c.cancel.load(Ordering::Relaxed) {
            return Ok(None);
        }
    }
    trace_impl(
        scene,
        generator.len(),
        |range| generator.rays(range),
        options,
        control,
    )
}

fn trace_impl<I: Iterator<Item = Result<Ray>>>(
    scene: &PreparedScene,
    len: usize,
    rays: impl Fn(std::ops::Range<usize>) -> I + Sync,
    options: TraceOptions,
    control: Option<&TraceControl>,
) -> Result<Option<Report>> {
    if scene.spec.mode == Mode::Sequential {
        trace_mode::<I, true, true>(scene, len, rays, options, control)
    } else {
        trace_mode::<I, false, false>(scene, len, rays, options, control)
    }
}

fn trace_mode<I: Iterator<Item = Result<Ray>>, const SEQUENTIAL: bool, const PACKETS: bool>(
    scene: &PreparedScene,
    len: usize,
    rays: impl Fn(std::ops::Range<usize>) -> I + Sync,
    options: TraceOptions,
    control: Option<&TraceControl>,
) -> Result<Option<Report>> {
    let packet_plan = if PACKETS {
        crate::packet::Plan::new(scene)
    } else {
        None
    };
    let surface_index = if SEQUENTIAL {
        None
    } else {
        SurfaceIndex::new(&scene.surfaces)
    };
    // Fixed indexed chunks give a fixed reduction order independent of worker count.
    // At most 32 dense detector-map copies exist, including the final reduction.
    let chunk_size = len.div_ceil(31).max(1);
    let reports: Vec<_> = (0..len.div_ceil(chunk_size))
        .into_par_iter()
        .map(|chunk_index| -> Result<Option<Report>> {
            let start = chunk_index * chunk_size;
            let chunk = rays(start..start.saturating_add(chunk_size).min(len));
            let mut report = Report::new(scene);
            let mut cache = OpticalCache::new(scene);
            let mut traversal = Traversal {
                stack: WorkQueue::new(if SEQUENTIAL {
                    Mode::Sequential
                } else {
                    Mode::NonSequential
                }),
                surface_index: surface_index.as_ref(),
            };
            let mut completed = 0;
            // Keep the original tight loop for NSC, unsupported shapes and CPUs
            // without AVX2. There is no packet gather/replay cost on this path.
            if packet_plan.is_none() {
                for (position, ray) in chunk.enumerate() {
                    if control.is_some_and(|c| c.cancel.load(Ordering::Relaxed)) {
                        return Ok(None);
                    }
                    let ray = ray?;
                    let record_paths =
                        ((chunk_index * chunk_size + position) as u64) < options.path_launch_count;
                    let optics = cache.get(scene, ray.wavelength_um);
                    trace_one::<SEQUENTIAL, false>(
                        scene,
                        ray,
                        &mut report,
                        options,
                        record_paths,
                        optics,
                        &mut traversal,
                        None,
                    );
                    if let Some(c) = control {
                        completed += 1;
                        if completed == 256 {
                            c.completed.fetch_add(completed, Ordering::Relaxed);
                            completed = 0;
                        }
                    }
                }
                if let Some(c) = control {
                    c.completed.fetch_add(completed, Ordering::Relaxed);
                }
                return Ok(Some(report));
            }
            let mut chunk = chunk.enumerate();
            let mut packet_steps = Vec::new();
            let mut packet_optics: [Vec<(f64, f64)>; crate::packet::WIDTH] = Default::default();
            let mut failed_packets = 0;
            let mut scalar_cooldown: usize = 0;
            let mut cooldown_window: usize = 256;
            while let Some((position, ray)) = chunk.next() {
                if control.is_some_and(|c| c.cancel.load(Ordering::Relaxed)) {
                    return Ok(None);
                }
                let ray = ray?;
                if scalar_cooldown > 0 {
                    // No SoA gather or packet bookkeeping in an incoherent run.
                    let record_paths =
                        ((chunk_index * chunk_size + position) as u64) < options.path_launch_count;
                    let optics = cache.get(scene, ray.wavelength_um);
                    trace_one::<SEQUENTIAL, false>(
                        scene,
                        ray,
                        &mut report,
                        options,
                        record_paths,
                        optics,
                        &mut traversal,
                        None,
                    );
                    if let Some(c) = control {
                        completed += 1;
                        if completed == 256 {
                            c.completed.fetch_add(completed, Ordering::Relaxed);
                            completed = 0;
                        }
                    }
                    scalar_cooldown -= 1;
                    continue;
                }
                let mut launches = [ray; crate::packet::WIDTH];
                let mut count = 1;
                for launch in &mut launches[1..] {
                    if let Some((_, next)) = chunk.next() {
                        *launch = next?;
                        count += 1;
                    } else {
                        break;
                    }
                }
                let mixed = launches[..count]
                    .iter()
                    .any(|r| r.wavelength_um.to_bits() != ray.wavelength_um.to_bits());
                let prepared_optics = count == crate::packet::WIDTH && mixed;
                let cached = if let Some(plan) = &packet_plan {
                    if count == crate::packet::WIDTH {
                        if mixed {
                            for (launch, material) in launches.iter().zip(&mut packet_optics) {
                                material.clear();
                                material.extend_from_slice(cache.get(scene, launch.wavelength_um));
                            }
                            plan.build(
                                scene,
                                &launches,
                                packet_optics.each_ref().map(|v| v.as_slice()),
                                &mut packet_steps,
                            )
                        } else {
                            let optics = cache.get(scene, ray.wavelength_um);
                            plan.build(
                                scene,
                                &launches,
                                [optics; crate::packet::WIDTH],
                                &mut packet_steps,
                            )
                        }
                    } else {
                        false
                    }
                } else {
                    false
                };
                if count == crate::packet::WIDTH {
                    if cached {
                        failed_packets = 0;
                        cooldown_window = 256;
                    } else {
                        failed_packets += 1;
                        // Repeated aperture misses or exceptional geometry are
                        // cheaper on the scalar path. Back off repeated probes
                        // while allowing a later coherent source to recover.
                        if failed_packets == 4 {
                            scalar_cooldown = cooldown_window;
                            cooldown_window = (cooldown_window * 4).min(4096);
                            failed_packets = 0;
                        }
                    }
                }
                for (lane, ray) in launches[..count].iter().copied().enumerate() {
                    if control.is_some_and(|c| c.cancel.load(Ordering::Relaxed)) {
                        return Ok(None);
                    }
                    let record_paths = ((chunk_index * chunk_size + position + lane) as u64)
                        < options.path_launch_count;
                    let optics = if prepared_optics {
                        &packet_optics[lane]
                    } else {
                        cache.get(scene, ray.wavelength_um)
                    };
                    if cached {
                        trace_one::<SEQUENTIAL, true>(
                            scene,
                            ray,
                            &mut report,
                            options,
                            record_paths,
                            optics,
                            &mut traversal,
                            Some((&packet_steps, lane)),
                        );
                    } else {
                        trace_one::<SEQUENTIAL, false>(
                            scene,
                            ray,
                            &mut report,
                            options,
                            record_paths,
                            optics,
                            &mut traversal,
                            None,
                        );
                    }
                    if let Some(c) = control {
                        completed += 1;
                        if completed == 256 {
                            c.completed.fetch_add(completed, Ordering::Relaxed);
                            completed = 0;
                        }
                    }
                }
            }
            if let Some(c) = control {
                c.completed.fetch_add(completed, Ordering::Relaxed);
            }
            Ok(Some(report))
        })
        .collect();
    let mut report = Some(Report::new(scene));
    // Inspect every chunk in launch order so the earliest generation error wins.
    for next in reports {
        let next = next?;
        report = report.zip(next).map(|(a, b)| a.merge(b));
    }
    Ok(report)
}

#[cfg(test)]
mod packet_tests {
    use super::optics::evaluate_optics;
    use super::*;
    use crate::{
        math::{Frame, Pose, V3},
        model::{Aperture, Interaction, Material, Scene, Shape},
    };

    fn compare(scene: &PreparedScene, rays: &[Ray], options: TraceOptions) {
        let scalar = trace_mode::<_, true, false>(
            scene,
            rays.len(),
            |range| rays[range].iter().copied().map(Ok),
            options,
            None,
        )
        .unwrap()
        .unwrap();
        let expected = serde_json::to_vec(&scalar).unwrap();
        for workers in [1, 4] {
            let pool = rayon::ThreadPoolBuilder::new()
                .num_threads(workers)
                .build()
                .unwrap();
            let actual = pool.install(|| trace(scene, rays, options));
            let encoded = serde_json::to_vec(&actual).unwrap();
            if encoded != expected {
                let offset = encoded
                    .iter()
                    .zip(&expected)
                    .position(|(a, b)| a != b)
                    .unwrap_or(encoded.len().min(expected.len()));
                panic!(
                    "workers={workers}, first differing byte={offset}, scalar={}, packet={}",
                    String::from_utf8_lossy(
                        &expected[offset.saturating_sub(80)..(offset + 160).min(expected.len())]
                    ),
                    String::from_utf8_lossy(
                        &encoded[offset.saturating_sub(80)..(offset + 160).min(encoded.len())]
                    )
                );
            }
        }
        let control = TraceControl::default();
        let controlled = trace_controlled(scene, rays, options, &control).unwrap();
        assert_eq!(serde_json::to_vec(&controlled).unwrap(), expected);
        assert_eq!(control.completed.load(Ordering::Relaxed), rays.len());
        control.cancel.store(true, Ordering::Relaxed);
        if !rays.is_empty() {
            assert!(trace_controlled(scene, rays, options, &control).is_none());
        }
    }

    fn lens() -> Scene {
        let mut scene: Scene =
            serde_json::from_str(include_str!("../../../examples/sequential_lens.json")).unwrap();
        for source in &mut scene.sources {
            source.count = 1027;
        }
        scene
    }

    #[test]
    fn packets_keep_scalar_reports_for_shapes_poses_spectra_absorption_and_paths() {
        for shape in [
            Shape::Plane,
            Shape::Sphere { radius_mm: 25. },
            Shape::EvenAsphere {
                radius_mm: 25.,
                conic: -0.6,
                coefficients: vec![1e-6, 1e-10],
            },
            Shape::FlatEvenAsphere {
                coefficients: vec![1e-5, -1e-8],
            },
        ] {
            for rotated in [false, true] {
                let mut scene = lens();
                scene.surfaces[0].shape = shape.clone();
                scene.surfaces[2].aperture = Aperture::Rectangle {
                    half_width_mm: 20.,
                    half_height_mm: 20.,
                };
                scene.materials[1] = Material::WithAbsorption {
                    dispersion: Box::new(scene.materials[1].clone()),
                    samples: vec![[0.4, 0.0001], [0.7, 0.0003]],
                };
                if rotated {
                    let frame = Frame::new(&Pose {
                        translation_mm: V3([8., -4., 3.]),
                        rotation_deg: V3([17., -23., 8.]),
                    });
                    for surface in &mut scene.surfaces {
                        surface.pose = frame.transform_pose(&surface.pose);
                    }
                    for source in &mut scene.sources {
                        source.origin_mm = frame.world_point(source.origin_mm);
                        source.direction = frame.world_vector(source.direction);
                    }
                }
                let prepared = scene.prepare().unwrap();
                let mut rays = prepared.generate_rays().unwrap();
                for spectral in [false, true] {
                    if spectral {
                        for (i, ray) in rays.iter_mut().enumerate() {
                            ray.wavelength_um = 0.4 + (i % 400) as f64 * 0.0007;
                        }
                    }
                    if let Some(plan) = crate::packet::Plan::new(&prepared) {
                        let first: [Ray; 4] = rays[..4].try_into().unwrap();
                        let materials: [Vec<_>; 4] = std::array::from_fn(|i| {
                            evaluate_optics(&prepared, first[i].wavelength_um).collect()
                        });
                        let mut steps = Vec::new();
                        assert!(
                            plan.build(
                                &prepared,
                                &first,
                                materials.each_ref().map(|v| v.as_slice()),
                                &mut steps
                            ),
                            "eligible packet was not exercised: {shape:?}, rotated={rotated}, spectral={spectral}"
                        );
                        assert_eq!(steps.len(), prepared.surfaces.len());
                    }
                    compare(&prepared, &rays, TraceOptions::default());
                    compare(
                        &prepared,
                        &rays,
                        TraceOptions {
                            keep_terminals: true,
                            path_launch_count: 64,
                        },
                    );
                }
            }
        }
    }

    #[test]
    fn packets_preserve_misses_limits_cutoffs_tir_and_zero_distance_interfaces() {
        for case in 0..8 {
            let mut scene = lens();
            match case {
                0 => scene.surfaces[0].aperture = Aperture::Circle { radius_mm: 0.7 },
                1 => scene.settings.max_depth = 1,
                2 => scene.settings.max_events_per_launch = 1,
                3 => {
                    scene.settings.min_relative_branch_weight = 0.999;
                    if let Interaction::Dielectric { reflectance, .. } =
                        &mut scene.surfaces[0].interaction
                    {
                        *reflectance = Some(0.1);
                    }
                }
                4 => {
                    scene.surfaces[0].shape = Shape::Plane;
                    scene.surfaces[1].shape = Shape::Plane;
                    scene.surfaces[1].pose = scene.surfaces[0].pose.clone();
                }
                5 => scene.surfaces[0].interaction = Interaction::Mirror { reflectance: 0.95 },
                6 => {
                    scene.surfaces[0].shape = Shape::Plane;
                    scene.surfaces[0].interaction = Interaction::Dielectric {
                        minus_medium: 1,
                        plus_medium: 0,
                        reflectance: None,
                    };
                }
                _ => {
                    scene.surfaces[0].aperture = Aperture::Rectangle {
                        half_width_mm: 0.5,
                        half_height_mm: 0.5,
                    }
                }
            }
            let prepared = scene.prepare().unwrap();
            let mut rays = prepared.generate_rays().unwrap();
            if case == 6 {
                for ray in &mut rays {
                    ray.medium = 1;
                    ray.direction = V3([0.9, 0., 0.4358898943540673]).unit();
                    ray.position_mm = V3([0., 0., 9.9]);
                }
            }
            compare(
                &prepared,
                &rays,
                TraceOptions {
                    keep_terminals: true,
                    path_launch_count: 64,
                },
            );
        }
        let scene = lens().prepare().unwrap();
        let rays = scene.generate_rays().unwrap();
        for count in [0, 1, 3, 31, 32, 123, 124, 127] {
            compare(&scene, &rays[..count], TraceOptions::default());
        }
    }

    #[test]
    fn packets_keep_coatings_equal_indices_reverse_traversal_and_folded_mirrors() {
        use crate::coating::{Coating, CoatingModel};
        for equal_index in [false, true] {
            for reverse in [false, true] {
                let mut scene = lens();
                if equal_index {
                    scene.materials[1] = Material::Constant {
                        n: 1.,
                        absorption_per_mm: 0.,
                    };
                }
                scene.surfaces[2].aperture = Aperture::Rectangle {
                    half_width_mm: 20.,
                    half_height_mm: 20.,
                };
                scene.surfaces[0].coating = Some(Coating {
                    name: "owned table".into(),
                    front: CoatingModel::Table {
                        wavelengths_um: vec![0.4, 0.7],
                        angles_deg: vec![0., 60.],
                        coefficients: vec![[0.01, 0.98], [0.02, 0.96], [0.03, 0.95], [0.04, 0.94]],
                        low_index_angle: true,
                        tir_reflectance: 0.9,
                    },
                    back: Some(CoatingModel::Ideal {
                        reflectance: 0.02,
                        transmittance: 0.97,
                        tir_reflectance: 0.9,
                    }),
                    zemax_definition: None,
                });
                if reverse {
                    scene.surfaces.swap(0, 1);
                    scene.surfaces[2].pose.translation_mm = V3([0., 0., -10.]);
                    for source in &mut scene.sources {
                        source.origin_mm = V3([0., 0., 30.]);
                        source.direction = V3([0., 0., -1.]);
                    }
                }
                let prepared = scene.prepare().unwrap();
                let rays = prepared.generate_rays().unwrap();
                compare(
                    &prepared,
                    &rays,
                    TraceOptions {
                        keep_terminals: true,
                        path_launch_count: 64,
                    },
                );
            }
        }
        let mut scene: Scene =
            serde_json::from_str(include_str!("../../../examples/folded_mirror.json")).unwrap();
        scene.mode = Mode::Sequential;
        for source in &mut scene.sources {
            source.count = 1027;
        }
        let prepared = scene.prepare().unwrap();
        compare(
            &prepared,
            &prepared.generate_rays().unwrap(),
            TraceOptions {
                keep_terminals: true,
                path_launch_count: 64,
            },
        );
    }

    #[test]
    fn packets_preserve_grazing_roots_and_distant_coordinate_roundoff() {
        let prepared = lens().prepare().unwrap();
        let mut rays = prepared.generate_rays().unwrap();
        for (i, ray) in rays.iter_mut().enumerate() {
            let x = (i % 20) as f64 * 0.4;
            let root = (2500. - x * x).sqrt();
            let z = 50. - root;
            let slope = x / root;
            let direction = V3([1., 0., slope + ((i % 3) as f64 - 1.) * 1e-12]).unit();
            ray.position_mm = V3([x, 0., 10. + z]) - direction * 20.;
            ray.direction = direction;
        }
        compare(
            &prepared,
            &rays,
            TraceOptions {
                keep_terminals: true,
                path_launch_count: 64,
            },
        );
        let mut scene = lens();
        scene.surfaces[0].shape = Shape::EvenAsphere {
            radius_mm: 25.,
            conic: -0.6,
            coefficients: vec![1e-6, 1e-10],
        };
        scene.surfaces[2].aperture = Aperture::Rectangle {
            half_width_mm: 20.,
            half_height_mm: 20.,
        };
        let frame = Frame::new(&Pose {
            translation_mm: V3([1e12, -1e12, 1e12]),
            rotation_deg: V3([17., -23., 8.]),
        });
        for surface in &mut scene.surfaces {
            surface.pose = frame.transform_pose(&surface.pose);
        }
        for source in &mut scene.sources {
            source.origin_mm = frame.world_point(source.origin_mm);
            source.direction = frame.world_vector(source.direction);
        }
        let prepared = scene.prepare().unwrap();
        compare(
            &prepared,
            &prepared.generate_rays().unwrap(),
            TraceOptions {
                keep_terminals: true,
                path_launch_count: 64,
            },
        );
    }
}
