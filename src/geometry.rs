use crate::math::V3;
use crate::model::{Aperture, PreparedSurface, Shape};

pub const RAY_EPS_MM: f64 = 1e-8;

#[derive(Clone, Copy, Debug)]
pub struct Hit {
    pub distance_mm: f64,
    pub point: V3,
    /// Points from the minus medium towards the plus medium.
    pub normal: V3,
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

fn bounded_interval(surface: &PreparedSurface, p: V3, d: V3) -> Option<(f64, f64)> {
    let mut low: f64 = RAY_EPS_MM;
    let mut high: f64 = f64::INFINITY;
    for axis in 0..3 {
        let a = surface.local_bounds[0].0[axis];
        let b = surface.local_bounds[1].0[axis];
        if d.0[axis].abs() < 1e-15 {
            if p.0[axis] < a - 1e-12 || p.0[axis] > b + 1e-12 {
                return None;
            }
        } else {
            let ta = (a - p.0[axis]) / d.0[axis];
            let tb = (b - p.0[axis]) / d.0[axis];
            low = low.max(ta.min(tb));
            high = high.min(ta.max(tb));
        }
    }
    if let Aperture::Circle { radius_mm } = surface.spec.aperture {
        let a = d.0[0] * d.0[0] + d.0[1] * d.0[1];
        let b = p.0[0] * d.0[0] + p.0[1] * d.0[1];
        let c = p.0[0] * p.0[0] + p.0[1] * p.0[1] - radius_mm * radius_mm;
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

pub fn intersect(surface: &PreparedSurface, p_world: V3, d_world: V3) -> Result<Hit, Miss> {
    let p = surface.frame.local_point(p_world);
    let d = surface.frame.local_vector(d_world);
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
            distance_mm,
            local_point: point,
            point: surface.frame.world_point(point),
            normal: surface.frame.world_vector(normal).unit(),
        });
    }
    if let Some(mesh) = &surface.mesh {
        let (distance_mm, normal) = mesh.hit(p, d).ok_or(Miss::Geometry)?;
        let local_point = p + d * distance_mm;
        return Ok(Hit {
            distance_mm,
            local_point,
            point: surface.frame.world_point(local_point),
            normal: surface.frame.world_vector(normal).unit(),
        });
    }
    let t = match &surface.spec.shape {
        Shape::TriangleMesh { .. } | Shape::ConicalSide { .. } => unreachable!(),
        Shape::Plane => {
            if d.0[2].abs() < 1e-14 {
                return Err(Miss::Geometry);
            }
            -p.0[2] / d.0[2]
        }
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
        Shape::EvenAsphere { .. } => {
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
            if flo.abs() >= 1e-10 && fhi.abs() >= 1e-10 && flo.signum() == fhi.signum() {
                return Err(Miss::Geometry);
            }
            let mut t = if flo.abs() < 1e-10 {
                lo
            } else if fhi.abs() < 1e-10 {
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
                if residual.abs() < 1e-10 {
                    converged = true;
                    break;
                }
                if residual.signum() == flo.signum() {
                    lo = t;
                    flo = residual;
                } else {
                    hi = t;
                }
                let derivative = d.0[2] - gx * d.0[0] - gy * d.0[1];
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
        distance_mm: t,
        point: surface.frame.world_point(point),
        local_point: point,
        normal: surface
            .frame
            .world_vector(V3([-gx, -gy, 1.0]).unit())
            .unit(),
    })
}

/// Search from near to far; a global Lipschitz bound excludes root-free intervals.
/// This handles multiple intersections without assuming the sag residual is monotone.
fn oblique_asphere_hit(
    surface: &PreparedSurface,
    p: V3,
    d: V3,
    lo: f64,
    hi: f64,
) -> Result<Hit, Miss> {
    let value = |t| {
        let q = p + d * t;
        sag(&surface.spec.shape, q.0[0], q.0[1]).map(|v| q.0[2] - v.0)
    };
    let slope = d.0[2].abs() + surface.max_sag_slope * d.0[0].hypot(d.0[1]);
    let mut stack = [(0.0, 0.0); 64];
    let mut count = 1;
    stack[0] = (lo, hi);
    for _ in 0..4096 {
        if count == 0 {
            return Err(Miss::Geometry);
        }
        count -= 1;
        let (a, b) = stack[count];
        let mid = a + (b - a) * 0.5;
        let fm = value(mid).ok_or(Miss::Numerical)?;
        if fm.abs() > slope * (b - a) * 0.5 + 1e-12 {
            continue;
        }
        if b - a <= 1e-9 || mid == a || mid == b {
            let fa = value(a).ok_or(Miss::Numerical)?;
            let fb = value(b).ok_or(Miss::Numerical)?;
            let candidate = if fa * fb < 0.0 {
                a - fa * (b - a) / (fb - fa)
            } else {
                mid
            };
            let residual = value(candidate).ok_or(Miss::Numerical)?;
            if residual.abs() > 1e-9 {
                return Err(Miss::Numerical);
            }
            let local_point = p + d * candidate;
            let (_, gx, gy) = sag(&surface.spec.shape, local_point.0[0], local_point.0[1])
                .ok_or(Miss::Numerical)?;
            return Ok(Hit {
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
            reflectance: 0.0,
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
    let rs = ((n_before * cos_i - n_after * cos_t) / (n_before * cos_i + n_after * cos_t)).powi(2);
    let rp = ((n_before * cos_t - n_after * cos_i) / (n_before * cos_t + n_after * cos_i)).powi(2);
    InterfaceResult {
        reflected,
        transmitted: Some(transmitted),
        reflectance: (0.5 * (rs + rp)).clamp(0.0, 1.0),
    }
}
