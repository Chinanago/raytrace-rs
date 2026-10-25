//! Four independent f64 rays in structure-of-arrays form. The scalar tracer
//! replays successful packets in launch order, preserving every accumulation.
#[cfg(target_arch = "x86_64")]
use crate::model::{Interaction, Shape};
use crate::{
    geometry::{Hit, InterfaceResult},
    model::{PreparedScene, Ray},
};

pub(crate) const WIDTH: usize = 4;

#[derive(Clone, Copy)]
pub(crate) struct Step {
    pub hit: Hit,
    pub interface: Option<InterfaceResult>,
}

pub(crate) struct Plan {
    surfaces: usize,
}

impl Plan {
    pub fn new(scene: &PreparedScene) -> Option<Self> {
        #[cfg(target_arch = "x86_64")]
        if std::arch::is_x86_feature_detected!("avx2") {
            let mut count = 0;
            let limit = scene
                .spec
                .settings
                .max_depth
                .min(scene.spec.settings.max_events_per_launch);
            for surface in &scene.surfaces {
                if !matches!(
                    surface.spec.shape,
                    Shape::Plane
                        | Shape::Sphere { .. }
                        | Shape::EvenAsphere { .. }
                        | Shape::FlatEvenAsphere { .. }
                ) || surface.mesh.is_some()
                {
                    return None;
                }
                count += 1;
                if count == limit
                    || matches!(
                        surface.spec.interaction,
                        Interaction::Absorb | Interaction::Detector { .. }
                    )
                {
                    break;
                }
            }
            return Some(Self { surfaces: count });
        }
        let _ = scene;
        None
    }

    /// A failed lookahead has no observable effect: replay those launches with
    /// the scalar solver, including all original misses, diagnostics and limits.
    pub fn build(
        &self,
        scene: &PreparedScene,
        rays: &[Ray; WIDTH],
        optics: [&[(f64, f64)]; WIDTH],
        steps: &mut Vec<[Step; WIDTH]>,
    ) -> bool {
        steps.clear();
        #[cfg(target_arch = "x86_64")]
        {
            // SAFETY: Plan is only constructed after the AVX2 runtime check.
            unsafe { avx::build(scene, self.surfaces, rays, optics, steps) }
        }
        #[cfg(not(target_arch = "x86_64"))]
        {
            let _ = (self.surfaces, scene, rays, optics, steps);
            false
        }
    }
}

#[cfg(target_arch = "x86_64")]
mod avx {
    use super::*;
    use crate::{
        geometry::RAY_EPS_MM,
        math::V3,
        model::{Aperture, PreparedSurface},
    };
    use std::{
        arch::x86_64::*,
        ops::{Add, Div, Mul, Neg, Sub},
    };

    // Invariant: this private type is constructed and used only in the AVX2
    // call tree entered by Plan::build. Helpers inline into that target function.
    #[derive(Clone, Copy)]
    struct F(__m256d);
    impl F {
        #[inline(always)]
        fn splat(value: f64) -> Self {
            unsafe { Self(_mm256_set1_pd(value)) }
        }
        #[inline(always)]
        fn load(values: [f64; WIDTH]) -> Self {
            unsafe { Self(_mm256_loadu_pd(values.as_ptr())) }
        }
        #[inline(always)]
        fn array(self) -> [f64; WIDTH] {
            let mut result = [0.; WIDTH];
            unsafe {
                _mm256_storeu_pd(result.as_mut_ptr(), self.0);
            }
            result
        }
        #[inline(always)]
        fn sqrt(self) -> Self {
            unsafe { Self(_mm256_sqrt_pd(self.0)) }
        }
        #[inline(always)]
        fn abs(self) -> Self {
            unsafe { Self(_mm256_andnot_pd(_mm256_set1_pd(-0.), self.0)) }
        }
        #[inline(always)]
        fn min(self, b: Self) -> Self {
            unsafe { Self(_mm256_min_pd(self.0, b.0)) }
        }
        #[inline(always)]
        fn max(self, b: Self) -> Self {
            unsafe { Self(_mm256_max_pd(self.0, b.0)) }
        }
        #[inline(always)]
        fn lt(self, b: Self) -> Self {
            unsafe { Self(_mm256_cmp_pd::<_CMP_LT_OQ>(self.0, b.0)) }
        }
        #[inline(always)]
        fn le(self, b: Self) -> Self {
            unsafe { Self(_mm256_cmp_pd::<_CMP_LE_OQ>(self.0, b.0)) }
        }
        #[inline(always)]
        fn eq(self, b: Self) -> Self {
            unsafe { Self(_mm256_cmp_pd::<_CMP_EQ_OQ>(self.0, b.0)) }
        }
        #[inline(always)]
        fn or(self, b: Self) -> Self {
            unsafe { Self(_mm256_or_pd(self.0, b.0)) }
        }
        #[inline(always)]
        fn and(self, b: Self) -> Self {
            unsafe { Self(_mm256_and_pd(self.0, b.0)) }
        }
        #[inline(always)]
        fn same_mask(self, b: Self) -> Self {
            unsafe { Self(_mm256_xor_pd(self.0, b.0)) }.not()
        }
        #[inline(always)]
        fn not(self) -> Self {
            self.eq(Self::splat(0.))
        }
        #[inline(always)]
        fn bits(self) -> i32 {
            unsafe { _mm256_movemask_pd(self.0) }
        }
        #[inline(always)]
        fn all(self) -> bool {
            self.bits() == 15
        }
        #[inline(always)]
        fn finite(self) -> bool {
            self.abs().le(Self::splat(f64::MAX)).all()
        }
        #[inline(always)]
        fn select(self, yes: Self, no: Self) -> Self {
            unsafe { Self(_mm256_blendv_pd(no.0, yes.0, self.0)) }
        }
    }
    impl Add for F {
        type Output = Self;
        #[inline(always)]
        fn add(self, b: Self) -> Self {
            unsafe { Self(_mm256_add_pd(self.0, b.0)) }
        }
    }
    impl Sub for F {
        type Output = Self;
        #[inline(always)]
        fn sub(self, b: Self) -> Self {
            unsafe { Self(_mm256_sub_pd(self.0, b.0)) }
        }
    }
    impl Mul for F {
        type Output = Self;
        #[inline(always)]
        fn mul(self, b: Self) -> Self {
            unsafe { Self(_mm256_mul_pd(self.0, b.0)) }
        }
    }
    impl Div for F {
        type Output = Self;
        #[inline(always)]
        fn div(self, b: Self) -> Self {
            unsafe { Self(_mm256_div_pd(self.0, b.0)) }
        }
    }
    impl Neg for F {
        type Output = Self;
        #[inline(always)]
        fn neg(self) -> Self {
            unsafe { Self(_mm256_xor_pd(self.0, _mm256_set1_pd(-0.))) }
        }
    }

    #[derive(Clone, Copy)]
    struct V([F; 3]);
    impl V {
        #[inline(always)]
        fn load(values: [V3; WIDTH]) -> Self {
            Self(std::array::from_fn(|axis| {
                F::load(values.map(|v| v.0[axis]))
            }))
        }
        #[inline(always)]
        fn splat(value: V3) -> Self {
            Self(value.0.map(F::splat))
        }
        #[inline(always)]
        fn array(self) -> [V3; WIDTH] {
            let components = self.0.map(F::array);
            std::array::from_fn(|lane| V3(components.map(|v| v[lane])))
        }
        #[inline(always)]
        fn dot(self, b: Self) -> F {
            self.0[0] * b.0[0] + self.0[1] * b.0[1] + self.0[2] * b.0[2]
        }
        #[inline(always)]
        fn unit(self) -> Self {
            self * (F::splat(1.) / self.dot(self).sqrt())
        }
        #[inline(always)]
        fn local(self, axes: [V3; 3]) -> Self {
            Self(axes.map(|a| self.dot(Self::splat(a))))
        }
        #[inline(always)]
        fn world(self, axes: [V3; 3]) -> Self {
            Self::splat(axes[0]) * self.0[0]
                + Self::splat(axes[1]) * self.0[1]
                + Self::splat(axes[2]) * self.0[2]
        }
    }
    impl Add for V {
        type Output = Self;
        #[inline(always)]
        fn add(self, b: Self) -> Self {
            Self(std::array::from_fn(|i| self.0[i] + b.0[i]))
        }
    }
    impl Sub for V {
        type Output = Self;
        #[inline(always)]
        fn sub(self, b: Self) -> Self {
            Self(std::array::from_fn(|i| self.0[i] - b.0[i]))
        }
    }
    impl Mul<F> for V {
        type Output = Self;
        #[inline(always)]
        fn mul(self, b: F) -> Self {
            Self(self.0.map(|v| v * b))
        }
    }

    #[inline(always)]
    fn contains(aperture: &Aperture, p: V) -> bool {
        match *aperture {
            Aperture::Circle { radius_mm } => (p.0[0] * p.0[0] + p.0[1] * p.0[1])
                .le(F::splat(radius_mm * radius_mm * (1. + 32. * f64::EPSILON)))
                .all(),
            Aperture::Rectangle {
                half_width_mm,
                half_height_mm,
            } => p.0[0]
                .abs()
                .le(F::splat(half_width_mm * (1. + 32. * f64::EPSILON)))
                .and(
                    p.0[1]
                        .abs()
                        .le(F::splat(half_height_mm * (1. + 32. * f64::EPSILON))),
                )
                .all(),
        }
    }

    #[inline(always)]
    fn bounded(surface: &PreparedSurface, p: V, d: V) -> Option<(F, F)> {
        let mut lo = F::splat(RAY_EPS_MM);
        let mut hi = F::splat(f64::INFINITY);
        let zero = F::splat(0.);
        for axis in 0..3 {
            let a = surface.local_bounds[0].0[axis];
            let b = surface.local_bounds[1].0[axis];
            let pad = F::splat(32. * f64::EPSILON)
                * (F::splat(1.) + F::splat(a.abs().max(b.abs())).max(p.0[axis].abs()));
            let parallel = d.0[axis].abs().lt(F::splat(1e-15));
            if parallel
                .and(
                    p.0[axis]
                        .lt(F::splat(a - 1e-12))
                        .or(F::splat(b + 1e-12).lt(p.0[axis])),
                )
                .bits()
                != 0
            {
                return None;
            }
            let ta = (F::splat(a) - pad - p.0[axis]) / d.0[axis];
            let tb = (F::splat(b) + pad - p.0[axis]) / d.0[axis];
            lo = parallel.select(lo, lo.max(ta.min(tb)));
            hi = parallel.select(hi, hi.min(ta.max(tb)));
        }
        if let Aperture::Circle { radius_mm } = surface.spec.aperture {
            let a = d.0[0] * d.0[0] + d.0[1] * d.0[1];
            let b = p.0[0] * d.0[0] + p.0[1] * d.0[1];
            let c = p.0[0] * p.0[0] + p.0[1] * p.0[1]
                - F::splat(radius_mm * radius_mm * (1. + 32. * f64::EPSILON));
            let parallel = a.lt(F::splat(1e-30));
            if parallel.and(zero.lt(c)).bits() != 0 {
                return None;
            }
            let disc = b * b - a * c;
            if parallel.not().and(disc.lt(zero)).bits() != 0 {
                return None;
            }
            let root = disc.sqrt();
            lo = parallel.select(lo, lo.max((-b - root) / a));
            hi = parallel.select(hi, hi.min((-b + root) / a));
        }
        (lo.le(hi + F::splat(1e-12))
            .and(F::splat(RAY_EPS_MM).lt(hi))
            .all())
        .then_some((lo, hi.max(lo)))
    }

    #[inline(always)]
    fn sag(shape: &Shape, p: V) -> Option<(F, F, F)> {
        let (c, conic, coefficients): (f64, f64, &[f64]) = match shape {
            Shape::Sphere { radius_mm } => (1. / radius_mm, 0., &[]),
            Shape::EvenAsphere {
                radius_mm,
                conic,
                coefficients,
            } => (1. / radius_mm, *conic, coefficients),
            Shape::FlatEvenAsphere { coefficients } => (0., 0., coefficients),
            _ => return None,
        };
        let r2 = p.0[0] * p.0[0] + p.0[1] * p.0[1];
        let radicand = F::splat(1.) - F::splat((1. + conic) * c * c) * r2;
        if !F::splat(1e-14).lt(radicand).all() {
            return None;
        }
        let root = radicand.sqrt();
        let mut z = F::splat(c) * r2 / (F::splat(1.) + root);
        let mut factor = F::splat(c) / root;
        let mut power = r2;
        for (i, a) in coefficients.iter().enumerate() {
            z = z + F::splat(*a) * power * r2;
            factor = factor + F::splat(2. * (i + 2) as f64 * a) * power;
            power = power * r2;
        }
        if !z.finite() || !factor.finite() {
            return None;
        }
        Some((z, factor * p.0[0], factor * p.0[1]))
    }

    #[inline(always)]
    fn intersection(
        surface: &PreparedSurface,
        origin: V,
        direction: V,
    ) -> Option<([Hit; WIDTH], V, V)> {
        let axes = surface.frame.axes();
        let frame_origin = V::splat(surface.frame.origin);
        let p = (origin - frame_origin).local(axes);
        let d = direction.local(axes);
        // Coincident sequential surfaces have a separate zero-distance rule.
        // Defer the packet if any lane might use it, before ordinary bounds.
        let op = origin.array();
        let lp = p.array();
        for lane in 0..WIDTH {
            let scale = op[lane]
                .0
                .into_iter()
                .chain(surface.frame.origin.0)
                .map(f64::abs)
                .fold(1., f64::max);
            let tol = 32. * f64::EPSILON * scale;
            if lp[lane].0[2] >= surface.local_bounds[0].0[2] - tol
                && lp[lane].0[2] <= surface.local_bounds[1].0[2] + tol
            {
                return None;
            }
        }
        let (lo, hi) = bounded(surface, p, d)?;
        let zero = F::splat(0.);
        let t = match surface.spec.shape {
            Shape::Plane => {
                if d.0[2].abs().lt(F::splat(1e-14)).bits() != 0 {
                    return None;
                }
                -p.0[2] / d.0[2]
            }
            Shape::Sphere { radius_mm } => {
                let q = p - V::splat(V3([0., 0., radius_mm]));
                let a = d.dot(d);
                let b = q.dot(d) / a;
                let closest = q - d * b;
                let discriminant = F::splat(radius_mm * radius_mm) - closest.dot(closest);
                if discriminant.lt(zero).bits() != 0 {
                    return None;
                }
                let root = (discriminant / a).sqrt();
                let first = -b - root;
                let second = -b + root;
                let valid = |t: F| {
                    F::splat(RAY_EPS_MM).lt(t).and(
                        ((p + d * t).0[2] - F::splat(radius_mm))
                            .mul(F::splat(radius_mm))
                            .le(zero),
                    )
                };
                let a_ok = valid(first);
                let b_ok = valid(second);
                if !a_ok.or(b_ok).all() {
                    return None;
                }
                a_ok.select(first, second)
            }
            Shape::EvenAsphere { .. } | Shape::FlatEvenAsphere { .. } => {
                // A conservative L1 bound avoids a scalar hypot call. Passing
                // this stricter test also selects the scalar monotone solver.
                if !(F::splat(surface.max_sag_slope) * (d.0[0].abs() + d.0[1].abs())
                    + F::splat(1e-14))
                .lt(d.0[2].abs())
                .all()
                {
                    return None;
                }
                let mut lo = lo;
                let mut hi = hi;
                let mut flo = (p + d * lo).0[2] - sag(&surface.spec.shape, p + d * lo)?.0;
                let fhi = (p + d * hi).0[2] - sag(&surface.spec.shape, p + d * hi)?.0;
                let endpoint = flo.eq(zero).or(fhi.eq(zero));
                if !endpoint
                    .or(flo.lt(zero).same_mask(fhi.lt(zero)).not())
                    .all()
                {
                    return None;
                }
                let initial = (-p.0[2] / d.0[2]).max(lo).min(hi);
                let mut t = flo.eq(zero).select(lo, fhi.eq(zero).select(hi, initial));
                let mut active = F::splat(1.).eq(F::splat(1.));
                for _ in 0..64 {
                    let point = p + d * t;
                    let (z, gx, gy) = sag(&surface.spec.shape, point)?;
                    let residual = point.0[2] - z;
                    let derivative = d.0[2] - gx * d.0[0] - gy * d.0[1];
                    let tolerance = F::splat(32. * f64::EPSILON)
                        * (F::splat(1.) + t.abs().max(point.dot(point).sqrt()));
                    let converged = residual
                        .eq(zero)
                        .or(residual.abs().le(derivative.abs() * tolerance))
                        .or((hi - lo).le(tolerance));
                    active = active.and(converged.not());
                    if active.bits() == 0 {
                        break;
                    }
                    if active.and(derivative.abs().lt(F::splat(1e-14))).bits() != 0 {
                        return None;
                    }
                    let same = residual.lt(zero).same_mask(flo.lt(zero));
                    lo = active.and(same).select(t, lo);
                    flo = active.and(same).select(residual, flo);
                    hi = active.and(same.not()).select(t, hi);
                    let candidate = t - residual / derivative;
                    let within = lo.lt(candidate).and(candidate.lt(hi));
                    let next = within.select(candidate, F::splat(0.5) * (lo + hi));
                    t = active.select(next, t);
                }
                if active.bits() != 0 {
                    return None;
                }
                t
            }
            _ => return None,
        };
        if !t.finite() || !F::splat(RAY_EPS_MM).lt(t).all() {
            return None;
        }
        let local = p + d * t;
        if !contains(&surface.spec.aperture, local) {
            return None;
        }
        let normal = if matches!(surface.spec.shape, Shape::Plane) {
            V::splat(surface.frame.plane_normal())
        } else {
            let (_, gx, gy) = sag(&surface.spec.shape, local)?;
            V([-gx, -gy, F::splat(1.)]).unit().world(axes).unit()
        };
        let world = frame_origin + local.world(axes);
        let distances = t.array();
        let points = world.array();
        let locals = local.array();
        let normals = normal.array();
        let hits = std::array::from_fn(|lane| Hit {
            distance_mm: distances[lane],
            point: points[lane],
            local_point: locals[lane],
            normal: normals[lane],
            geometric_normal: normals[lane],
        });
        Some((hits, world, normal))
    }

    #[inline(always)]
    fn dielectric(
        d: V,
        n: V,
        before: F,
        after: F,
        power: Option<f64>,
    ) -> Option<([InterfaceResult; WIDTH], V)> {
        let zero = F::splat(0.);
        let one = F::splat(1.);
        let facing = V(std::array::from_fn(|i| {
            zero.lt(d.dot(n)).select(-n.0[i], n.0[i])
        }));
        let ci = (-d.dot(facing)).max(zero).min(one);
        let reflected = (d + facing * (F::splat(2.) * ci)).unit();
        let equal = (before - after).abs().lt(F::splat(1e-15));
        let eta = before / after;
        let st2 = eta * eta * (one - ci * ci);
        if equal.not().and(one.le(st2)).bits() != 0 {
            return None;
        }
        let ct = (one - st2).sqrt();
        let refracted = (d * eta + facing * (eta * ci - ct)).unit();
        let transmitted = V(std::array::from_fn(|i| {
            equal.select(d.0[i], refracted.0[i])
        }));
        let reflectance = if let Some(p) = power {
            F::splat(p)
        } else {
            let rs = (before * ci - after * ct) / (before * ci + after * ct);
            let rp = (before * ct - after * ci) / (before * ct + after * ci);
            equal.select(
                zero,
                (F::splat(0.5) * (rs * rs + rp * rp)).max(zero).min(one),
            )
        };
        let r = reflectance.array();
        let rd = reflected.array();
        let td = transmitted.array();
        Some((
            std::array::from_fn(|lane| InterfaceResult {
                reflected: rd[lane],
                transmitted: Some(td[lane]),
                reflectance: r[lane],
            }),
            transmitted,
        ))
    }

    #[target_feature(enable = "avx2")]
    pub(super) unsafe fn build(
        scene: &PreparedScene,
        count: usize,
        rays: &[Ray; WIDTH],
        optics: [&[(f64, f64)]; WIDTH],
        steps: &mut Vec<[Step; WIDTH]>,
    ) -> bool {
        let mut p = V::load(rays.map(|r| r.position_mm));
        let mut d = V::load(rays.map(|r| r.direction));
        let mut media = rays.map(|r| r.medium);
        for surface in &scene.surfaces[..count] {
            let Some((hits, point, normal)) = intersection(surface, p, d) else {
                return false;
            };
            p = point;
            let interface = match surface.spec.interaction {
                Interaction::Dielectric {
                    minus_medium,
                    plus_medium,
                    reflectance,
                } => {
                    let incoming = d.dot(normal).array();
                    // At grazing incidence, signed zero and root conditioning
                    // belong to the scalar edge-case rules, including clamp.
                    if incoming.iter().any(|v| !v.is_finite() || v.abs() < 1e-12) {
                        return false;
                    }
                    let expected =
                        incoming.map(|v| if v > 0. { minus_medium } else { plus_medium });
                    if media != expected {
                        return false;
                    }
                    let after = incoming.map(|v| if v > 0. { plus_medium } else { minus_medium });
                    let nb = F::load(std::array::from_fn(|i| optics[i][media[i]].0));
                    let na = F::load(std::array::from_fn(|i| optics[i][after[i]].0));
                    let power = if surface.spec.coating.is_some() {
                        Some(0.)
                    } else {
                        reflectance
                    };
                    let Some((interfaces, transmitted)) = dielectric(d, normal, nb, na, power)
                    else {
                        return false;
                    };
                    for lane in 0..WIDTH {
                        let n = hits[lane].normal;
                        if !n.finite()
                            || interfaces[lane].reflected.dot(n) * incoming[lane] > 1e-14
                            || interfaces[lane]
                                .transmitted
                                .is_some_and(|t| t.dot(n) * incoming[lane] < -1e-14)
                        {
                            return false;
                        }
                    }
                    d = transmitted;
                    media = after;
                    Some(interfaces)
                }
                Interaction::Mirror { .. } => {
                    d = (d - normal * (F::splat(2.) * d.dot(normal))).unit();
                    None
                }
                _ => None,
            };
            steps.push(std::array::from_fn(|lane| Step {
                hit: hits[lane],
                interface: interface.map(|r| r[lane]),
            }));
            if matches!(
                surface.spec.interaction,
                Interaction::Absorb | Interaction::Detector { .. }
            ) {
                break;
            }
        }
        true
    }
}
