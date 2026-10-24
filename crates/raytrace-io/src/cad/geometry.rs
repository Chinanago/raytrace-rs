use super::{
    nurbs,
    step::{field, Table, Value},
};
use crate::Result;
use raytrace_core::math::V3;
use std::f64::consts::{FRAC_PI_2, TAU};

#[derive(Clone, Copy, Debug)]
pub(super) struct Transform {
    pub origin: V3,
    pub axes: [V3; 3],
}
impl Transform {
    pub fn identity() -> Self {
        Self {
            origin: V3::default(),
            axes: [V3([1., 0., 0.]), V3([0., 1., 0.]), V3([0., 0., 1.])],
        }
    }
    pub fn vector(&self, p: V3) -> V3 {
        self.axes[0] * p.0[0] + self.axes[1] * p.0[1] + self.axes[2] * p.0[2]
    }
    pub fn point(&self, p: V3) -> V3 {
        self.origin + self.vector(p)
    }
    pub fn local_vector(&self, p: V3) -> V3 {
        V3(self.axes.map(|a| a.dot(p)))
    }
    pub fn local_point(&self, p: V3) -> V3 {
        self.local_vector(p - self.origin)
    }
    pub fn compose(&self, child: Self) -> Self {
        Self {
            origin: self.point(child.origin),
            axes: child.axes.map(|a| self.vector(a)),
        }
    }
    pub fn inverse(&self) -> Result<Self> {
        let [a, b, c] = self.axes;
        let det = a.dot(b.cross(c));
        let scale = a.norm() * b.norm() * c.norm();
        if !det.is_finite() || !scale.is_finite() || scale == 0. || det.abs() < scale * 1e-12 {
            return Err(crate::Error::step("STEP: 配置変換が特異です".into()));
        }
        let rows = [b.cross(c) / det, c.cross(a) / det, a.cross(b) / det];
        let inverse = Self {
            origin: V3::default(),
            axes: std::array::from_fn(|i| V3(rows.map(|r| r.0[i]))),
        };
        Ok(Self {
            origin: -inverse.vector(self.origin),
            ..inverse
        })
    }
    pub fn normal(&self, n: V3) -> Result<V3> {
        let inverse = self.inverse()?;
        unit(V3(inverse.axes.map(|a| a.dot(n))))
    }
}
fn unit(v: V3) -> Result<V3> {
    let n = v.norm();
    if !v.finite() || !n.is_finite() || n < 1e-15 {
        return Err(crate::Error::step(
            "STEP: ゼロ長または不正な方向・法線です".into(),
        ));
    }
    Ok(v / n)
}
pub(super) fn point(table: &Table, id: usize) -> Result<V3> {
    let coords = field(table.args(id, "CARTESIAN_POINT")?, 1)?.numbers()?;
    match *coords.as_slice() {
        [x, y, z] => Ok(V3([x, y, z])),
        [x, y] => Ok(V3([x, y, 0.])),
        _ => Err(crate::Error::step(format!(
            "STEP #{id}: 点の座標数が不正です"
        ))),
    }
}
fn direction(table: &Table, id: usize) -> Result<V3> {
    let coords = field(table.args(id, "DIRECTION")?, 1)?.numbers()?;
    let v = match *coords.as_slice() {
        [x, y, z] => V3([x, y, z]),
        [x, y] => V3([x, y, 0.]),
        _ => {
            return Err(crate::Error::step(format!(
                "STEP #{id}: 方向の座標数が不正です"
            )))
        }
    };
    unit(v)
}
pub(super) fn frame(table: &Table, id: usize) -> Result<Transform> {
    let e = table.get(id)?;
    if let Some(a) = e.part("AXIS2_PLACEMENT_3D") {
        let origin = point(table, field(a, 1)?.reference()?)?;
        let z = match field(a, 2)? {
            Value::Empty => V3([0., 0., 1.]),
            v => direction(table, v.reference()?)?,
        };
        let x = match field(a, 3)? {
            Value::Empty => {
                if z.0[0].abs() < 0.9 {
                    V3([1., 0., 0.])
                } else {
                    V3([0., 1., 0.])
                }
            }
            v => direction(table, v.reference()?)?,
        };
        let x = unit(x - z * x.dot(z))?;
        return Ok(Transform {
            origin,
            axes: [x, z.cross(x), z],
        });
    }
    if let Some(a) = e.part("AXIS2_PLACEMENT_2D") {
        let origin = point(table, field(a, 1)?.reference()?)?;
        let x = match field(a, 2)? {
            Value::Empty => V3([1., 0., 0.]),
            v => direction(table, v.reference()?)?,
        };
        return Ok(Transform {
            origin,
            axes: [x, V3([-x.0[1], x.0[0], 0.]), V3([0., 0., 1.])],
        });
    }
    Err(crate::Error::step(format!(
        "STEP #{id}: 未対応の軸配置 {}",
        e.kind()
    )))
}

#[derive(Clone, Debug)]
pub(super) enum Curve {
    Line { origin: V3, direction: V3 },
    Ellipse { frame: Transform, a: f64, b: f64 },
    Spline(nurbs::Curve),
}
impl Curve {
    pub fn evaluate(&self, t: f64) -> (V3, V3) {
        match self {
            Self::Line { origin, direction } => (*origin + *direction * t, *direction),
            Self::Ellipse { frame, a, b } => {
                let (s, c) = t.sin_cos();
                (
                    frame.point(V3([a * c, b * s, 0.])),
                    frame.vector(V3([-a * s, b * c, 0.])),
                )
            }
            Self::Spline(c) => c.evaluate(t),
        }
    }
    pub fn periodic(&self) -> bool {
        matches!(self, Self::Ellipse { .. })
    }
    pub fn breaks(&self) -> Vec<f64> {
        match self {
            Self::Spline(c) => c.basis.breaks(),
            _ => vec![],
        }
    }
    pub fn parameter(&self, p: V3) -> f64 {
        match self {
            Self::Line { origin, direction } => {
                (p - *origin).dot(*direction) / direction.dot(*direction)
            }
            Self::Ellipse { frame, a, b } => {
                let p = frame.local_point(p);
                (p.0[1] / b).atan2(p.0[0] / a).rem_euclid(TAU)
            }
            Self::Spline(c) => {
                let [a, b] = c.basis.range();
                let mut best = (a, f64::INFINITY);
                for t in c
                    .basis
                    .breaks()
                    .into_iter()
                    .chain((0..=24).map(|i| a + (b - a) * i as f64 / 24.))
                {
                    let error = (c.evaluate(t).0 - p).norm();
                    if error < best.1 {
                        best = (t, error);
                    }
                }
                let mut t = best.0;
                for _ in 0..40 {
                    let (q, d) = c.evaluate(t);
                    let error = (q - p).norm();
                    if error < best.1 {
                        best = (t, error);
                    }
                    let denom = d.dot(d);
                    if denom < 1e-30 || !denom.is_finite() {
                        break;
                    }
                    let step = (q - p).dot(d) / denom;
                    let mut accepted = false;
                    for k in 0..12 {
                        let candidate = (t - step * 0.5f64.powi(k)).clamp(a, b);
                        if (c.evaluate(candidate).0 - p).norm() < error {
                            t = candidate;
                            accepted = true;
                            break;
                        }
                    }
                    if !accepted {
                        break;
                    }
                }
                best.0
            }
        }
    }
}

#[derive(Clone, Debug)]
pub(super) enum Surface {
    Plane(Transform),
    Cylinder {
        frame: Transform,
        radius: f64,
    },
    Cone {
        frame: Transform,
        radius: f64,
        angle: f64,
    },
    Sphere {
        frame: Transform,
        radius: f64,
    },
    SpherePatch {
        frame: Transform,
        radius: f64,
    },
    Torus {
        frame: Transform,
        major: f64,
        minor: f64,
    },
    Spline(nurbs::Surface, Vec<([f64; 2], V3)>),
}
impl Surface {
    pub fn sample(&self, uv: [f64; 2]) -> (V3, V3, V3) {
        let [u, v] = uv;
        match self {
            Self::Plane(f) => (f.point(V3([u, v, 0.])), f.axes[0], f.axes[1]),
            Self::Cylinder {
                frame: f,
                radius: r,
            } => {
                let (s, c) = u.sin_cos();
                (
                    f.point(V3([r * c, r * s, v])),
                    f.vector(V3([-r * s, r * c, 0.])),
                    f.axes[2],
                )
            }
            Self::Cone {
                frame: f,
                radius: r,
                angle,
            } => {
                let (s, c) = u.sin_cos();
                let t = angle.tan();
                let radius = r + v * t;
                (
                    f.point(V3([radius * c, radius * s, v])),
                    f.vector(V3([-radius * s, radius * c, 0.])),
                    f.vector(V3([t * c, t * s, 1.])),
                )
            }
            Self::Sphere {
                frame: f,
                radius: r,
            } => {
                let (s, c) = u.sin_cos();
                let (sv, cv) = v.sin_cos();
                (
                    f.point(V3([r * cv * c, r * cv * s, r * sv])),
                    f.vector(V3([-r * cv * s, r * cv * c, 0.])),
                    f.vector(V3([-r * sv * c, -r * sv * s, r * cv])),
                )
            }
            Self::SpherePatch {
                frame: f,
                radius: r,
            } => {
                let z = (r * r - u * u - v * v).max(0.).sqrt();
                let denominator = z.max(r * 1e-12);
                (
                    f.point(V3([u, v, z])),
                    f.vector(V3([1., 0., -u / denominator])),
                    f.vector(V3([0., 1., -v / denominator])),
                )
            }
            Self::Torus {
                frame: f,
                major: r,
                minor: q,
            } => {
                let (s, c) = u.sin_cos();
                let (sv, cv) = v.sin_cos();
                let radius = r + q * cv;
                (
                    f.point(V3([radius * c, radius * s, q * sv])),
                    f.vector(V3([-radius * s, radius * c, 0.])),
                    f.vector(V3([-q * sv * c, -q * sv * s, q * cv])),
                )
            }
            Self::Spline(s, _) => {
                let ranges = [s.u.range(), s.v.range()];
                let uv = std::array::from_fn(|i| {
                    if s.closed[i] && (uv[i] < ranges[i][0] || uv[i] > ranges[i][1]) {
                        ranges[i][0]
                            + (uv[i] - ranges[i][0]).rem_euclid(ranges[i][1] - ranges[i][0])
                    } else {
                        uv[i]
                    }
                });
                s.evaluate(uv)
            }
        }
    }
    pub fn point(&self, uv: [f64; 2]) -> V3 {
        self.sample(uv).0
    }
    pub fn normal(&self, uv: [f64; 2]) -> Result<V3> {
        match self {
            Self::Sphere { frame, radius: _ } | Self::SpherePatch { frame, radius: _ } => {
                unit(self.point(uv) - frame.origin)
            }
            _ => {
                let (_, du, dv) = self.sample(uv);
                let singular = if let Self::Spline(s, _) = self {
                    let scale = s
                        .points
                        .iter()
                        .map(|p| (*p - s.points[0]).norm())
                        .fold(0., f64::max);
                    let ur = s.u.range();
                    let vr = s.v.range();
                    du.norm() < scale / (ur[1] - ur[0]) * 1e-9
                        || dv.norm() < scale / (vr[1] - vr[0]) * 1e-9
                } else {
                    false
                };
                if !singular {
                    if let Ok(n) = unit(du.cross(dv)) {
                        return Ok(n);
                    }
                }
                if let Self::Spline(s, _) = self {
                    let ranges = [s.u.range(), s.v.range()];
                    for axis in 0..2 {
                        let [a, b] = ranges[axis];
                        let mut q = uv;
                        q[axis] = if (uv[axis] - a).abs() < (uv[axis] - b).abs() {
                            a + (b - a) * 1e-6
                        } else {
                            b - (b - a) * 1e-6
                        };
                        let (_, du, dv) = self.sample(q);
                        if let Ok(n) = unit(du.cross(dv)) {
                            return Ok(n);
                        }
                    }
                }
                Err(crate::Error::step(
                    "STEP: 曲面の法線が退化しています".into(),
                ))
            }
        }
    }
    pub fn periods(&self) -> [Option<f64>; 2] {
        match self {
            Self::Cylinder { .. } | Self::Cone { .. } | Self::Sphere { .. } => [Some(TAU), None],
            Self::Torus { .. } => [Some(TAU), Some(TAU)],
            Self::Spline(s, _) => std::array::from_fn(|i| {
                if s.closed[i] {
                    let r = if i == 0 { s.u.range() } else { s.v.range() };
                    Some(r[1] - r[0])
                } else {
                    None
                }
            }),
            _ => [None, None],
        }
    }
    pub fn seam_opposite(
        &self,
        axis: usize,
        base: f64,
        previous: Option<[f64; 2]>,
        winding: f64,
    ) -> f64 {
        let period = self.periods()[axis].unwrap();
        let target = previous.map_or(base + period, |p| p[axis]);
        let left = (base - period - target).abs();
        let right = (base + period - target).abs();
        if (left - right).abs() < period * 1e-6 && winding.abs() > period * 0.5 {
            return base - period * winding.signum();
        }
        if left < right {
            base - period
        } else {
            base + period
        }
    }
    pub fn prepare_chart(&mut self, boundary: &[V3]) -> Result<()> {
        if let Self::Sphere { frame, radius } = self {
            let mut z = V3::default();
            for &p in boundary {
                z = z + unit(p - frame.origin)?;
            }
            let z = unit(z).map_err(|_| {
                crate::Error::step("STEP: 球面を一つの半球へ展開できません".to_string())
            })?;
            if boundary
                .iter()
                .any(|p| (*p - frame.origin).dot(z) < -radius.abs() * 1e-8)
            {
                return Err(crate::Error::step(
                    "STEP: 半球を超える球面の境界は未対応です".into(),
                ));
            }
            let a = if z.0[0].abs() < 0.9 {
                V3([1., 0., 0.])
            } else {
                V3([0., 1., 0.])
            };
            let x = unit(a - z * a.dot(z))?;
            *self = Self::SpherePatch {
                frame: Transform {
                    origin: frame.origin,
                    axes: [x, z.cross(x), z],
                },
                radius: *radius,
            };
        }
        Ok(())
    }
    pub fn project(&self, p: V3, seed: Option<[f64; 2]>, tolerance: f64) -> Result<[f64; 2]> {
        let mut uv = match self {
            Self::Plane(f) => {
                let p = f.local_point(p);
                [p.0[0], p.0[1]]
            }
            Self::Cylinder { frame, .. } | Self::Cone { frame, .. } => {
                let q = frame.local_point(p);
                [q.0[1].atan2(q.0[0]), q.0[2]]
            }
            Self::Sphere { frame, .. } => {
                let q = frame.local_point(p);
                [q.0[1].atan2(q.0[0]), q.0[2].atan2(q.0[0].hypot(q.0[1]))]
            }
            Self::SpherePatch { frame, .. } => {
                let q = frame.local_point(p);
                [q.0[0], q.0[1]]
            }
            Self::Torus { frame, major, .. } => {
                let q = frame.local_point(p);
                [
                    q.0[1].atan2(q.0[0]),
                    q.0[2].atan2(q.0[0].hypot(q.0[1]) - major),
                ]
            }
            Self::Spline(s, grid) => {
                let ranges = [s.u.range(), s.v.range()];
                let mut best = ([ranges[0][0], ranges[1][0]], f64::INFINITY);
                if let Some(seed) = seed {
                    let q = self.optimize_projection(p, seed, ranges, tolerance);
                    best = (q, (self.point(q) - p).norm());
                }
                if best.1 > (tolerance * 1e-5).max(1e-11) {
                    let mut candidates = vec![best];
                    for &(q, point) in grid {
                        let error = (point - p).norm();
                        candidates.push((q, error));
                        if error < best.1 {
                            best = (q, error);
                        }
                    }
                    candidates.sort_by(|a, b| a.1.total_cmp(&b.1));
                    for (seed, _) in candidates.into_iter().take(12) {
                        let q = self.optimize_projection(p, seed, ranges, tolerance);
                        let error = (self.point(q) - p).norm();
                        if error < best.1 {
                            best = (q, error);
                        }
                        if best.1 < (tolerance * 1e-5).max(1e-11) {
                            break;
                        }
                    }
                }
                best.0
            }
        };
        if let Self::Spline(s, _) = self {
            for (i, range) in [s.u.range(), s.v.range()].into_iter().enumerate() {
                for endpoint in range {
                    if (uv[i] - endpoint).abs() < (range[1] - range[0]) * 1e-9 {
                        uv[i] = endpoint;
                    }
                }
            }
        }
        if !uv.iter().all(|x| x.is_finite()) || (self.point(uv) - p).norm() > tolerance {
            return Err(crate::Error::step(format!(
                "STEP: 境界を曲面へ投影できません（誤差 {:.6e}、許容 {:.6e}）",
                (self.point(uv) - p).norm(),
                tolerance
            )));
        }
        Ok(uv)
    }
    fn optimize_projection(
        &self,
        p: V3,
        seed: [f64; 2],
        ranges: [[f64; 2]; 2],
        tol: f64,
    ) -> [f64; 2] {
        let periods = self.periods();
        let canonical = |value: f64, i: usize| {
            if let Some(period) = periods[i] {
                ranges[i][0] + (value - ranges[i][0]).rem_euclid(period)
            } else {
                value.clamp(ranges[i][0], ranges[i][1])
            }
        };
        let mut uv = std::array::from_fn(|i| canonical(seed[i], i));
        for _ in 0..40 {
            let (q, du, dv) = self.sample(uv);
            let residual = q - p;
            let error = residual.norm();
            if error < tol * 0.0001 {
                break;
            }
            let (a, b, c) = (du.dot(du), du.dot(dv), dv.dot(dv));
            let det = a * c - b * b;
            if !det.is_finite() || det <= 1e-24 * a * c {
                break;
            }
            let (x, y) = (du.dot(residual), dv.dot(residual));
            let step = [(c * x - b * y) / det, (a * y - b * x) / det];
            let mut accepted = false;
            for k in 0..14 {
                let factor = 0.5f64.powi(k);
                let candidate = std::array::from_fn(|i| canonical(uv[i] - step[i] * factor, i));
                if (self.point(candidate) - p).norm() < error {
                    uv = candidate;
                    accepted = true;
                    break;
                }
            }
            if !accepted {
                break;
            }
        }
        uv
    }
}

fn positive(value: f64) -> Result<f64> {
    if value > 0. && value.is_finite() {
        Ok(value)
    } else {
        Err(crate::Error::step(
            "STEP: 半径・寸法は正の有限値が必要です".into(),
        ))
    }
}
fn weights(values: Option<&Value>, count: usize) -> Result<Vec<f64>> {
    let mut w = if let Some(v) = values {
        v.numbers()?
    } else {
        vec![1.; count]
    };
    if w.len() != count || w.iter().any(|v| !v.is_finite() || *v <= 0.) {
        return Err(crate::Error::step("STEP: NURBSの重みが不正です".into()));
    }
    let max = w.iter().copied().fold(0., f64::max);
    for v in &mut w {
        *v /= max;
        if *v < 1e-100 {
            return Err(crate::Error::step(
                "STEP: NURBSの重みの比が大きすぎます".into(),
            ));
        }
    }
    Ok(w)
}
pub(super) fn read_curve(table: &Table, id: usize) -> Result<Curve> {
    read_curve_inner(table, id, 0)
}
fn read_curve_inner(table: &Table, id: usize, depth: usize) -> Result<Curve> {
    if depth > 16 {
        return Err(crate::Error::step(
            "STEP: 曲線の参照が循環しています".into(),
        ));
    }
    let e = table.get(id)?;
    for kind in ["SURFACE_CURVE", "SEAM_CURVE"] {
        if let Some(a) = e.part(kind) {
            return read_curve_inner(table, field(a, 1)?.reference()?, depth + 1);
        }
    }
    if let Some(a) = e.part("LINE") {
        let vector = table.args(field(a, 2)?.reference()?, "VECTOR")?;
        let d = direction(table, field(vector, 1)?.reference()?)?
            * positive(field(vector, 2)?.number()?)?;
        return Ok(Curve::Line {
            origin: point(table, field(a, 1)?.reference()?)?,
            direction: d,
        });
    }
    for kind in ["CIRCLE", "ELLIPSE"] {
        if let Some(a) = e.part(kind) {
            let radius = positive(field(a, 2)?.number()?)?;
            return Ok(Curve::Ellipse {
                frame: frame(table, field(a, 1)?.reference()?)?,
                a: radius,
                b: if kind == "CIRCLE" {
                    radius
                } else {
                    positive(field(a, 3)?.number()?)?
                },
            });
        }
    }
    if let Some(a) = e.part("POLYLINE") {
        let points = field(a, 1)?
            .refs()?
            .into_iter()
            .map(|id| point(table, id))
            .collect::<Result<Vec<_>, _>>()?;
        if points.len() < 2 {
            return Err(crate::Error::step(
                "STEP: POLYLINEの点が不足しています".into(),
            ));
        }
        let n = points.len();
        let knots = (0..n).map(|i| i as f64).collect::<Vec<_>>();
        let mut mults = vec![1.; n];
        mults[0] = 2.;
        mults[n - 1] = 2.;
        return Ok(Curve::Spline(nurbs::Curve {
            basis: nurbs::Basis::new(1, n, &mults, &knots)?,
            points,
            weights: vec![1.; n],
        }));
    }
    let (base, offset, knots_offset) = if let Some(a) = e.part("B_SPLINE_CURVE") {
        (a, 0, 0)
    } else if let Some(a) = e.part("B_SPLINE_CURVE_WITH_KNOTS") {
        (a, 1, 6)
    } else {
        return Err(crate::Error::step(format!(
            "STEP #{id}: 未対応の境界曲線 {}",
            e.kind()
        )));
    };
    let degree = field(base, offset)?.integer()?;
    let points = field(base, offset + 1)?
        .refs()?
        .into_iter()
        .map(|id| point(table, id))
        .collect::<Result<Vec<_>, _>>()?;
    let k = e.part("B_SPLINE_CURVE_WITH_KNOTS").ok_or_else(|| {
        crate::Error::step(format!("STEP #{id}: ノット付きB-spline曲線が必要です"))
    })?;
    let basis = nurbs::Basis::new(
        degree,
        points.len(),
        &field(k, knots_offset)?.numbers()?,
        &field(k, knots_offset + 1)?.numbers()?,
    )?;
    let c = nurbs::Curve {
        basis,
        weights: weights(
            e.part("RATIONAL_B_SPLINE_CURVE").and_then(|a| a.first()),
            points.len(),
        )?,
        points,
    };
    c.validate()?;
    Ok(Curve::Spline(c))
}

pub(super) fn read_surface(table: &Table, id: usize) -> Result<Surface> {
    let e = table.get(id)?;
    for kind in [
        "PLANE",
        "CYLINDRICAL_SURFACE",
        "CONICAL_SURFACE",
        "SPHERICAL_SURFACE",
        "TOROIDAL_SURFACE",
    ] {
        if let Some(a) = e.part(kind) {
            let f = frame(table, field(a, 1)?.reference()?)?;
            if kind == "PLANE" {
                return Ok(Surface::Plane(f));
            }
            let r = positive(field(a, 2)?.number()?)?;
            return Ok(match kind {
                "CYLINDRICAL_SURFACE" => Surface::Cylinder {
                    frame: f,
                    radius: r,
                },
                "SPHERICAL_SURFACE" => Surface::Sphere {
                    frame: f,
                    radius: r,
                },
                "TOROIDAL_SURFACE" => {
                    let minor = positive(field(a, 3)?.number()?)?;
                    if minor >= r {
                        return Err(crate::Error::step(
                            "STEP: 自己交差するトーラスは未対応です".into(),
                        ));
                    }
                    Surface::Torus {
                        frame: f,
                        major: r,
                        minor,
                    }
                }
                _ => {
                    let angle = field(a, 3)?.number()? * table.angle_scale_rad()?;
                    if angle.abs() >= FRAC_PI_2 || angle.abs() < 1e-12 {
                        return Err(crate::Error::step("STEP: 円錐の半角が不正です".into()));
                    }
                    Surface::Cone {
                        frame: f,
                        radius: r,
                        angle,
                    }
                }
            });
        }
    }
    let (base, offset, knots_offset) = if let Some(a) = e.part("B_SPLINE_SURFACE") {
        (a, 0, 0)
    } else if let Some(a) = e.part("B_SPLINE_SURFACE_WITH_KNOTS") {
        (a, 1, 8)
    } else {
        return Err(crate::Error::step(format!(
            "STEP #{id}: 未対応の面 {}",
            e.kind()
        )));
    };
    let (ud, vd) = (
        field(base, offset)?.integer()?,
        field(base, offset + 1)?.integer()?,
    );
    let rows = field(base, offset + 2)?.list()?;
    let points = rows
        .iter()
        .map(|r| {
            r.refs()?
                .into_iter()
                .map(|id| point(table, id))
                .collect::<Result<Vec<_>, _>>()
        })
        .collect::<Result<Vec<_>, _>>()?;
    let nv = points.first().map_or(0, Vec::len);
    if nv == 0 || points.iter().any(|r| r.len() != nv) {
        return Err(crate::Error::step(
            "STEP: 曲面の制御点が矩形配列ではありません".into(),
        ));
    }
    let k = e.part("B_SPLINE_SURFACE_WITH_KNOTS").ok_or_else(|| {
        crate::Error::step(format!("STEP #{id}: ノット付きB-spline曲面が必要です"))
    })?;
    let u = nurbs::Basis::new(
        ud,
        points.len(),
        &field(k, knots_offset)?.numbers()?,
        &field(k, knots_offset + 2)?.numbers()?,
    )?;
    let v = nurbs::Basis::new(
        vd,
        nv,
        &field(k, knots_offset + 1)?.numbers()?,
        &field(k, knots_offset + 3)?.numbers()?,
    )?;
    let n = points.len() * nv;
    let w = if let Some(a) = e.part("RATIONAL_B_SPLINE_SURFACE") {
        let w = field(a, 0)?.list()?;
        if w.len() != points.len()
            || w.iter()
                .any(|r| r.list().map_or(true, |row| row.len() != nv))
        {
            return Err(crate::Error::step("STEP: 曲面の重み配列が不正です".into()));
        }
        let flat = Value::List(
            w.iter()
                .flat_map(|row| row.list().unwrap().iter().cloned())
                .collect(),
        );
        weights(Some(&flat), n)?
    } else {
        vec![1.; n]
    };
    let closed =
        std::array::from_fn(|i| matches!(base.get(offset+4+i),Some(Value::Enum(s)) if s=="T"));
    let s = nurbs::Surface {
        u,
        v,
        points: points.into_iter().flatten().collect(),
        weights: w,
        closed,
    };
    s.validate()?;
    Ok(spline_surface(s))
}

fn spline_surface(s: nurbs::Surface) -> Surface {
    // Reuse the multistart positions without relaxing projection accuracy.
    // Near-end starts resolve distinct trim points beside collapsed edges.
    let ranges = [s.u.range(), s.v.range()];
    let ub = s.u.breaks();
    let vb = s.v.breaks();
    let mut grid = Vec::new();
    let near_u = [
        ranges[0][0] + (ranges[0][1] - ranges[0][0]) * 1e-6,
        ranges[0][1] - (ranges[0][1] - ranges[0][0]) * 1e-6,
    ];
    let near_v = [
        ranges[1][0] + (ranges[1][1] - ranges[1][0]) * 1e-6,
        ranges[1][1] - (ranges[1][1] - ranges[1][0]) * 1e-6,
    ];
    for u in ub
        .into_iter()
        .chain((0..=8).map(|i| ranges[0][0] + (ranges[0][1] - ranges[0][0]) * i as f64 / 8.))
        .chain(near_u)
    {
        for v in vb
            .iter()
            .copied()
            .chain((0..=16).map(|i| ranges[1][0] + (ranges[1][1] - ranges[1][0]) * i as f64 / 16.))
            .chain(near_v)
        {
            let uv = [u, v];
            grid.push((uv, s.evaluate(uv).0));
        }
    }
    Surface::Spline(s, grid)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn trim_projection_escapes_a_collapsed_parameter_edge() {
        let basis = || nurbs::Basis::new(1, 2, &[2., 2.], &[0., 1.]).unwrap();
        // S(u,v)=(u*(1-v),u*v,0). Every v at u=0 is the origin.
        let surface = spline_surface(nurbs::Surface {
            u: basis(),
            v: basis(),
            points: vec![
                V3::default(),
                V3::default(),
                V3([1., 0., 0.]),
                V3([0., 1., 0.]),
            ],
            weights: vec![1.; 4],
            closed: [false, false],
        });
        for p in [V3([0.0002, 0.0001, 0.]), V3([0.0001, 0.0002, 0.])] {
            let uv = surface.project(p, Some([0., 0.8]), 0.002).unwrap();
            // A point already within the outer rejection tolerance must still
            // recover its position; accepting the singular seed loses trims.
            assert!((surface.point(uv) - p).norm() < 2e-7, "{uv:?}");
            assert!(uv[0] > 1e-4);
        }
    }
}
