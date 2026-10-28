use crate::math::V3;
use crate::model::{Aperture, PreparedSurface, Shape};

pub const RAY_EPS_MM: f64 = 1e-8;

#[derive(Clone, Copy, Debug)]
pub struct Hit {
    pub distance_mm: f64,
    pub point: V3,
    /// Points from the minus medium towards the plus medium.
    pub normal: V3,
    /// Boundary orientation for medium selection; mesh normals may be smoothed.
    pub geometric_normal: V3,
    pub local_point: V3,
}
#[derive(Clone, Copy, Debug)]
pub enum Miss {
    Geometry,
    Numerical,
}

/// Conservative bounds include the full sag cap, including polynomial terms.
pub fn bounds(shape: &Shape, aperture: &Aperture) -> Option<([V3; 2], f64)> {
    if let Shape::ConicalSide {
        radius_start_mm,
        radius_end_mm,
        length_mm,
    } = shape
    {
        let r = radius_start_mm.max(*radius_end_mm);
        return Some(([V3([-r, -r, 0.0]), V3([r, r, *length_mm])], 0.0));
    }
    if let Shape::TriangleMesh { vertices, .. } = shape {
        if vertices.is_empty() {
            return None;
        }
        let low = V3(std::array::from_fn(|a| {
            vertices
                .iter()
                .map(|v| v.0[a])
                .fold(f64::INFINITY, f64::min)
        }));
        let high = V3(std::array::from_fn(|a| {
            vertices
                .iter()
                .map(|v| v.0[a])
                .fold(f64::NEG_INFINITY, f64::max)
        }));
        return Some(([low, high], 0.0));
    }
    let (hx, hy) = aperture.bounds();
    let (radius, conic, coefficients): (f64, f64, &[f64]) = match shape {
        Shape::TriangleMesh { .. } | Shape::ConicalSide { .. } => unreachable!(),
        Shape::Plane => return Some(([V3([-hx, -hy, 0.0]), V3([hx, hy, 0.0])], 0.0)),
        Shape::Sphere { radius_mm } => (*radius_mm, 0.0, &[]),
        Shape::FlatEvenAsphere { coefficients } => (f64::INFINITY, 0.0, coefficients),
        Shape::EvenAsphere {
            radius_mm,
            conic,
            coefficients,
        } => (*radius_mm, *conic, coefficients),
    };
    let r2 = aperture.max_radius2();
    let c = 1.0 / radius;
    let radicand = 1.0 - (1.0 + conic) * c * c * r2;
    let conic_end = c * r2 / (1.0 + radicand.sqrt());
    let mut low = conic_end.min(0.0);
    let mut high = conic_end.max(0.0);
    let mut factor_bound = c.abs() / radicand.min(1.0).sqrt();
    let mut power = r2;
    for (i, a) in coefficients.iter().enumerate() {
        let term = a * power * r2;
        low += term.min(0.0);
        high += term.max(0.0);
        factor_bound += 2.0 * (i + 2) as f64 * a.abs() * power;
        power *= r2;
    }
    let slope = factor_bound * r2.sqrt();
    (low.is_finite() && high.is_finite() && slope.is_finite())
        .then_some(([V3([-hx, -hy, low]), V3([hx, hy, high])], slope))
}

/// Lower bound of the surface search interval along a world-space ray.
pub fn search_start(surface: &PreparedSurface, p_world: V3, d_world: V3) -> Option<f64> {
    bounded_interval(
        surface,
        surface.frame.local_point(p_world),
        surface.frame.local_vector(d_world),
    )
    .map(|(low, _)| low)
}

fn bounded_interval(surface: &PreparedSurface, p: V3, d: V3) -> Option<(f64, f64)> {
    let mut low: f64 = RAY_EPS_MM;
    let mut high: f64 = f64::INFINITY;
    for axis in 0..3 {
        let a = surface.local_bounds[0].0[axis];
        let b = surface.local_bounds[1].0[axis];
        let padding = 32. * f64::EPSILON * (1. + a.abs().max(b.abs()).max(p.0[axis].abs()));
        if d.0[axis].abs() < 1e-15 {
            if p.0[axis] < a - 1e-12 || p.0[axis] > b + 1e-12 {
                return None;
            }
        } else {
            let ta = (a - padding - p.0[axis]) / d.0[axis];
            let tb = (b + padding - p.0[axis]) / d.0[axis];
            low = low.max(ta.min(tb));
            high = high.min(ta.max(tb));
        }
    }
    if let Aperture::Circle { radius_mm } = surface.spec.aperture {
        let a = d.0[0] * d.0[0] + d.0[1] * d.0[1];
        let b = p.0[0] * d.0[0] + p.0[1] * d.0[1];
        let c =
            p.0[0] * p.0[0] + p.0[1] * p.0[1] - radius_mm * radius_mm * (1. + 32. * f64::EPSILON);
        if a < 1e-30 {
            if c > 0.0 {
                return None;
            }
        } else {
            let disc = b * b - a * c;
            if disc < 0.0 {
                return None;
            }
            low = low.max((-b - disc.sqrt()) / a);
            high = high.min((-b + disc.sqrt()) / a);
        }
    }
    (low <= high + 1e-12 && high > RAY_EPS_MM).then_some((low, high.max(low)))
}

/// Sag and derivatives with respect to x and y.
pub fn sag(shape: &Shape, x: f64, y: f64) -> Option<(f64, f64, f64)> {
    let (radius, conic, coefficients): (f64, f64, &[f64]) = match shape {
        Shape::TriangleMesh { .. } | Shape::ConicalSide { .. } => return None,
        Shape::Plane => return Some((0.0, 0.0, 0.0)),
        Shape::Sphere { radius_mm } => (*radius_mm, 0.0, &[]),
        Shape::FlatEvenAsphere { coefficients } => (f64::INFINITY, 0.0, coefficients),
        Shape::EvenAsphere {
            radius_mm,
            conic,
            coefficients,
        } => (*radius_mm, *conic, coefficients),
    };
    let c = 1.0 / radius;
    let r2 = x * x + y * y;
    let radicand = 1.0 - (1.0 + conic) * c * c * r2;
    if radicand <= 1e-14 {
        return None;
    }
    let root = radicand.sqrt();
    let mut z = c * r2 / (1.0 + root);
    let mut factor = c / root;
    let mut power = r2;
    for (i, a) in coefficients.iter().enumerate() {
        z += a * power * r2;
        factor += 2.0 * (i + 2) as f64 * a * power;
        power *= r2;
    }
    (z.is_finite() && factor.is_finite()).then_some((z, factor * x, factor * y))
}

/// An ordered sequence may contain a stop, interface and image at the same
/// position. Each is visited once, so a zero-distance hit cannot self-loop.
pub fn intersect_ordered(surface: &PreparedSurface, p_world: V3, d_world: V3) -> Result<Hit, Miss> {
    let p = surface.frame.local_point(p_world);
    let d = surface.frame.local_vector(d_world);
    if !matches!(
        surface.spec.shape,
        Shape::TriangleMesh { .. } | Shape::ConicalSide { .. }
    ) {
        let scale = p_world
            .0
            .into_iter()
            .chain(surface.frame.origin.0)
            .map(f64::abs)
            .fold(1.0, f64::max);
        let tolerance = 32.0 * f64::EPSILON * scale;
        if p.0[2] >= surface.local_bounds[0].0[2] - tolerance
            && p.0[2] <= surface.local_bounds[1].0[2] + tolerance
            && surface.spec.aperture.contains(p.0[0], p.0[1])
            && let Some((z, gx, gy)) = sag(&surface.spec.shape, p.0[0], p.0[1])
        {
            let normal = if matches!(surface.spec.shape, Shape::Plane) {
                surface.frame.plane_normal()
            } else {
                surface
                    .frame
                    .world_vector(V3([-gx, -gy, 1.0]).unit())
                    .unit()
            };
            if (p.0[2] - z).abs() <= tolerance && d_world.dot(normal).abs() > 1e-14 {
                return Ok(Hit {
                    geometric_normal: normal,
                    distance_mm: 0.0,
                    point: p_world,
                    local_point: p,
                    normal,
                });
            }
        }
    }
    intersect_local(surface, p, d)
}

pub fn intersect(surface: &PreparedSurface, p_world: V3, d_world: V3) -> Result<Hit, Miss> {
    let p = surface.frame.local_point(p_world);
    let d = surface.frame.local_vector(d_world);
    intersect_local(surface, p, d)
}

fn intersect_local(surface: &PreparedSurface, p: V3, d: V3) -> Result<Hit, Miss> {
    if matches!(surface.spec.shape, Shape::Plane) {
        if d.0[2].abs() < 1e-14 {
            return Err(Miss::Geometry);
        }
        let t = -p.0[2] / d.0[2];
        if !t.is_finite() || t <= RAY_EPS_MM {
            return Err(Miss::Geometry);
        }
        let point = p + d * t;
        if !surface.spec.aperture.contains(point.0[0], point.0[1]) {
            return Err(Miss::Geometry);
        }
        // Keep the original bounding predicate for all possible hits, including
        // grazing circular apertures. Reject ordinary misses before that work.
        bounded_interval(surface, p, d).ok_or(Miss::Geometry)?;
        let normal = surface.frame.plane_normal();
        return Ok(Hit {
            distance_mm: t,
            point: surface.frame.world_point(point),
            local_point: point,
            normal,
            geometric_normal: normal,
        });
    }
    let (t_low, t_high) = bounded_interval(surface, p, d).ok_or(Miss::Geometry)?;
    if let Shape::ConicalSide {
        radius_start_mm,
        radius_end_mm,
        length_mm,
    } = surface.spec.shape
    {
        let k = (radius_end_mm - radius_start_mm) / length_mm;
        let r = radius_start_mm + k * p.0[2];
        let a = d.0[0].powi(2) + d.0[1].powi(2) - (k * d.0[2]).powi(2);
        let b = 2.0 * (p.0[0] * d.0[0] + p.0[1] * d.0[1] - r * k * d.0[2]);
        let c = p.0[0].powi(2) + p.0[1].powi(2) - r * r;
        let mut candidates = if a.abs() < 1e-15 {
            if b.abs() < 1e-15 {
                return Err(Miss::Geometry);
            }
            [-c / b, f64::INFINITY]
        } else {
            let discriminant = b * b - 4.0 * a * c;
            if discriminant < 0.0 {
                return Err(Miss::Geometry);
            }
            let q = -0.5 * (b + discriminant.sqrt().copysign(b));
            if q == 0.0 {
                [-b / (2.0 * a), f64::INFINITY]
            } else {
                [q / a, c / q]
            }
        };
        candidates.sort_by(f64::total_cmp);
        let distance_mm = candidates
            .into_iter()
            .find(|t| t.is_finite() && *t > RAY_EPS_MM && *t >= t_low - 1e-9 && *t <= t_high + 1e-9)
            .ok_or(Miss::Geometry)?;
        let point = p + d * distance_mm;
        let normal = V3([
            point.0[0],
            point.0[1],
            -k * (radius_start_mm + k * point.0[2]),
        ])
        .unit();
        return Ok(Hit {
            geometric_normal: surface.frame.world_vector(normal).unit(),
            distance_mm,
            local_point: point,
            point: surface.frame.world_point(point),
            normal: surface.frame.world_vector(normal).unit(),
        });
    }
    if let Some(mesh) = &surface.mesh {
        let h = mesh.hit_with_normals(p, d).ok_or(Miss::Geometry)?;
        let distance_mm = h.distance_mm;
        let local_point = p + d * distance_mm;
        return Ok(Hit {
            distance_mm,
            local_point,
            point: surface.frame.world_point(local_point),
            normal: surface.frame.world_vector(h.normal).unit(),
            geometric_normal: surface.frame.world_vector(h.geometric_normal).unit(),
        });
    }
    let t = match &surface.spec.shape {
        Shape::TriangleMesh { .. } | Shape::ConicalSide { .. } => unreachable!(),
        Shape::Plane => unreachable!(),
        Shape::Sphere { radius_mm } => {
            let q = p - V3([0.0, 0.0, *radius_mm]);
            let a = d.dot(d);
            let b = q.dot(d) / a;
            let closest = q - d * b;
            let discriminant = radius_mm * radius_mm - closest.dot(closest);
            if discriminant < 0.0 {
                return Err(Miss::Geometry);
            }
            let root = (discriminant / a).sqrt();
            [-b - root, -b + root]
                .into_iter()
                .filter(|t| *t > RAY_EPS_MM)
                .find(|t| ((p + d * *t).0[2] - radius_mm) * radius_mm <= 0.0)
                .ok_or(Miss::Geometry)?
        }
        Shape::EvenAsphere { .. } | Shape::FlatEvenAsphere { .. } => {
            // Use the fast solver when monotonicity proves a unique intersection;
            // otherwise search bounded intervals in nearest-first order.
            let transverse = d.0[0].hypot(d.0[1]);
            if d.0[2].abs() <= surface.max_sag_slope * transverse + 1e-14 {
                return oblique_asphere_hit(surface, p, d, t_low, t_high);
            }
            let residual_at = |t: f64| -> Option<f64> {
                let q = p + d * t;
                Some(q.0[2] - sag(&surface.spec.shape, q.0[0], q.0[1])?.0)
            };
            let mut lo = t_low;
            let mut hi = t_high;
            let mut flo = residual_at(lo).ok_or(Miss::Numerical)?;
            let fhi = residual_at(hi).ok_or(Miss::Numerical)?;
            if flo != 0. && fhi != 0. && flo.signum() == fhi.signum() {
                return Err(Miss::Geometry);
            }
            let mut t = if flo == 0. {
                lo
            } else if fhi == 0. {
                hi
            } else {
                (-p.0[2] / d.0[2]).clamp(lo, hi)
            };
            let mut converged = false;
            for _ in 0..64 {
                let point = p + d * t;
                let (z, gx, gy) =
                    sag(&surface.spec.shape, point.0[0], point.0[1]).ok_or(Miss::Numerical)?;
                let residual = point.0[2] - z;
                let derivative = d.0[2] - gx * d.0[0] - gy * d.0[1];
                let tolerance = 32. * f64::EPSILON * (1. + t.abs().max(point.norm()));
                // A sign-changing bracket can reach machine precision before
                // the rounded sag residual meets a near-tangent error estimate.
                if residual == 0.
                    || residual.abs() <= derivative.abs() * tolerance
                    || hi - lo <= tolerance
                {
                    converged = true;
                    break;
                }
                if residual.signum() == flo.signum() {
                    lo = t;
                    flo = residual;
                } else {
                    hi = t;
                }
                if derivative.abs() < 1e-14 {
                    return Err(Miss::Numerical);
                }
                let candidate = t - residual / derivative;
                t = if candidate > lo && candidate < hi {
                    candidate
                } else {
                    0.5 * (lo + hi)
                };
            }
            if !converged {
                return Err(Miss::Numerical);
            }
            t
        }
    };
    if !t.is_finite() || t <= RAY_EPS_MM {
        return Err(Miss::Geometry);
    }
    let point = p + d * t;
    if !surface.spec.aperture.contains(point.0[0], point.0[1]) {
        return Err(Miss::Geometry);
    }
    let (_, gx, gy) = sag(&surface.spec.shape, point.0[0], point.0[1]).ok_or(Miss::Numerical)?;
    Ok(Hit {
        geometric_normal: surface.frame.world_vector(V3([-gx, -gy, 1.]).unit()).unit(),
        distance_mm: t,
        point: surface.frame.world_point(point),
        local_point: point,
        normal: surface
            .frame
            .world_vector(V3([-gx, -gy, 1.0]).unit())
            .unit(),
    })
}

// Bound the spectral norm of the sag Hessian over the aperture. Together with
// the derivative at an interval midpoint this certifies monotonicity locally.
fn sag_curvature_bound(shape: &Shape, r2: f64) -> f64 {
    let (c, conic, coefficients) = match shape {
        Shape::EvenAsphere {
            radius_mm,
            conic,
            coefficients,
        } => (1. / radius_mm, *conic, coefficients),
        Shape::FlatEvenAsphere { coefficients } => (0., 0., coefficients),
        _ => unreachable!(),
    };
    let q = (1. - (1. + conic) * c * c * r2).min(1.);
    let mut bound = c.abs() / q.sqrt() + (1. + conic).abs() * c.abs().powi(3) * r2 / q.powf(1.5);
    let mut power = r2;
    for (i, a) in coefficients.iter().enumerate() {
        let m = (i + 2) as f64;
        bound += 2. * m * (2. * m - 1.) * a.abs() * power;
        power *= r2;
    }
    bound
}

/// Search near to far. A second-derivative bound excludes empty intervals and
/// certifies unique roots; bracketed roots are refined to floating-point scale.
/// A small residual alone does not establish an intersection near a tangent.
fn oblique_asphere_hit(
    surface: &PreparedSurface,
    p: V3,
    d: V3,
    lo: f64,
    hi: f64,
) -> Result<Hit, Miss> {
    let value = |t| {
        let q = p + d * t;
        sag(&surface.spec.shape, q.0[0], q.0[1])
            .map(|v| (q.0[2] - v.0, d.0[2] - v.1 * d.0[0] - v.2 * d.0[1]))
    };
    let transverse2 = d.0[0] * d.0[0] + d.0[1] * d.0[1];
    let mut stack = [(0.0, 0.0); 64];
    let mut count = 1;
    stack[0] = (lo, hi);
    for _ in 0..4096 {
        if count == 0 {
            return Err(Miss::Geometry);
        }
        count -= 1;
        let (a, b) = stack[count];
        // Squared radial distance is convex along the ray. Endpoint radii
        // bound this interval more tightly than the full optical aperture.
        let radius2 = [a, b]
            .into_iter()
            .map(|t| {
                let q = p + d * t;
                q.0[0] * q.0[0] + q.0[1] * q.0[1]
            })
            .fold(0., f64::max)
            .min(surface.spec.aperture.max_radius2());
        let curvature = sag_curvature_bound(&surface.spec.shape, radius2) * transverse2;
        let mid = a + (b - a) * 0.5;
        let (fm, df) = value(mid).ok_or(Miss::Numerical)?;
        let half = (b - a) * 0.5;
        let roundoff = 32. * f64::EPSILON * (1. + p.0[2].abs() + (d.0[2] * mid).abs());
        if fm.abs() > df.abs() * half + 0.5 * curvature * half * half + roundoff {
            continue;
        }
        let tolerance = 32. * f64::EPSILON * (1. + a.abs().max(b.abs()).max(p.norm()));
        let monotone = df.abs() > curvature * half + roundoff;
        let terminal = b - a <= tolerance || mid == a || mid == b;
        if monotone || terminal {
            let (mut fa, _) = value(a).ok_or(Miss::Numerical)?;
            let (mut fb, _) = value(b).ok_or(Miss::Numerical)?;
            let mut left = a;
            if fa != 0. && fb != 0. && fa.signum() == fb.signum() {
                if monotone {
                    continue;
                }
                // At floating-point scale a stationary zero represents a tangent.
                // Ordinary near-misses are already excluded by monotonicity/Taylor bounds.
                if terminal && fm.abs() <= roundoff && df.abs() <= curvature * half + roundoff {
                    left = mid;
                    fa = 0.;
                } else {
                    return Err(Miss::Numerical);
                }
            }
            let mut right = b;
            for _ in 0..80 {
                if fa == 0. || fb == 0. || right - left <= tolerance {
                    break;
                }
                let t = left + (right - left) * 0.5;
                let (f, _) = value(t).ok_or(Miss::Numerical)?;
                if f.signum() == fa.signum() {
                    left = t;
                    fa = f;
                } else {
                    right = t;
                    fb = f;
                }
            }
            let candidate = if fa.abs() < fb.abs() { left } else { right };
            if candidate <= RAY_EPS_MM {
                continue;
            }
            let local_point = p + d * candidate;
            if !surface
                .spec
                .aperture
                .contains(local_point.0[0], local_point.0[1])
            {
                continue;
            }
            let (_, gx, gy) = sag(&surface.spec.shape, local_point.0[0], local_point.0[1])
                .ok_or(Miss::Numerical)?;
            return Ok(Hit {
                geometric_normal: surface.frame.world_vector(V3([-gx, -gy, 1.]).unit()).unit(),
                distance_mm: candidate,
                local_point,
                point: surface.frame.world_point(local_point),
                normal: surface
                    .frame
                    .world_vector(V3([-gx, -gy, 1.0]).unit())
                    .unit(),
            });
        }
        if count + 2 > stack.len() {
            return Err(Miss::Numerical);
        }
        stack[count] = (mid, b);
        stack[count + 1] = (a, mid);
        count += 2;
    }
    Err(Miss::Numerical)
}

#[derive(Clone, Copy, Debug)]
pub struct InterfaceResult {
    pub reflected: V3,
    pub transmitted: Option<V3>,
    pub reflectance: f64,
}

/// Scalar, unpolarized Fresnel power coefficients for real refractive indices.
pub fn dielectric(direction: V3, normal: V3, n_before: f64, n_after: f64) -> InterfaceResult {
    dielectric_with_reflectance(direction, normal, n_before, n_after, None)
}

/// A supplied power coefficient still preserves Snell refraction and TIR. Coated
/// interfaces compute their power separately and can skip the unused bare Fresnel work.
#[inline]
pub(crate) fn dielectric_with_reflectance(
    direction: V3,
    normal: V3,
    n_before: f64,
    n_after: f64,
    power: Option<f64>,
) -> InterfaceResult {
    let facing = if direction.dot(normal) > 0.0 {
        -normal
    } else {
        normal
    };
    let cos_i = (-direction.dot(facing)).clamp(0.0, 1.0);
    let reflected = (direction + facing * (2.0 * cos_i)).unit();
    if (n_before - n_after).abs() < 1e-15 {
        return InterfaceResult {
            reflected,
            transmitted: Some(direction),
            reflectance: power.unwrap_or(0.0),
        };
    }
    let eta = n_before / n_after;
    let sin2_t = eta * eta * (1.0 - cos_i * cos_i);
    if sin2_t >= 1.0 {
        return InterfaceResult {
            reflected,
            transmitted: None,
            reflectance: 1.0,
        };
    }
    let cos_t = (1.0 - sin2_t).sqrt();
    let transmitted = (direction * eta + facing * (eta * cos_i - cos_t)).unit();
    let reflectance = power.unwrap_or_else(|| {
        let rs =
            ((n_before * cos_i - n_after * cos_t) / (n_before * cos_i + n_after * cos_t)).powi(2);
        let rp =
            ((n_before * cos_t - n_after * cos_i) / (n_before * cos_t + n_after * cos_i)).powi(2);
        (0.5 * (rs + rp)).clamp(0.0, 1.0)
    });
    InterfaceResult {
        reflected,
        transmitted: Some(transmitted),
        reflectance,
    }
}
