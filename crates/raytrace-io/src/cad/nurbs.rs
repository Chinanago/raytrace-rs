//! Local B-spline basis and rational derivatives; no modelling operations.
use crate::Result;
use raytrace_core::math::V3;

#[derive(Clone, Debug)]
pub(super) struct Basis {
    pub degree: usize,
    pub knots: Vec<f64>,
    pub count: usize,
}
impl Basis {
    pub fn new(degree: usize, count: usize, multiplicities: &[f64], knots: &[f64]) -> Result<Self> {
        if !(1..=16).contains(&degree)
            || count <= degree
            || count > 16_384
            || multiplicities.len() != knots.len()
            || knots.len() < 2
            || knots.len() > count + degree + 1
            || knots.windows(2).any(|k| k[0] >= k[1])
        {
            return Err(crate::Error::step(
                "STEP: B-splineの次数・制御点・ノット列が不正です".into(),
            ));
        }
        let mut expanded = Vec::new();
        for (&n, &value) in multiplicities.iter().zip(knots) {
            if !n.is_finite()
                || n < 1.
                || n > (degree + 1) as f64
                || n.fract() != 0.
                || !value.is_finite()
            {
                return Err(crate::Error::step(
                    "STEP: B-splineのノット重複度が不正です".into(),
                ));
            }
            expanded.extend(std::iter::repeat_n(value, n as usize));
        }
        if expanded.len() != count + degree + 1
            || expanded[degree] >= expanded[count]
            || !(expanded[count] - expanded[degree]).is_finite()
        {
            return Err(crate::Error::step(
                "STEP: B-splineのノット数・パラメーター範囲が不正です".into(),
            ));
        }
        Ok(Self {
            degree,
            knots: expanded,
            count,
        })
    }
    pub fn range(&self) -> [f64; 2] {
        [self.knots[self.degree], self.knots[self.count]]
    }
    pub fn breaks(&self) -> Vec<f64> {
        let [a, b] = self.range();
        let mut v: Vec<_> = self
            .knots
            .iter()
            .copied()
            .filter(|&t| t >= a && t <= b)
            .collect();
        v.dedup();
        v
    }
    fn values_at_span(&self, span: usize, degree: usize, t: f64) -> [f64; 17] {
        let mut values = [0.; 17];
        let mut left = [0.; 17];
        let mut right = [0.; 17];
        values[0] = 1.;
        for j in 1..=degree {
            left[j] = t - self.knots[span + 1 - j];
            right[j] = self.knots[span + j] - t;
            let mut saved = 0.;
            for r in 0..j {
                let denominator = right[r + 1] + left[j - r];
                let temp = if denominator > 0. {
                    values[r] / denominator
                } else {
                    0.
                };
                values[r] = saved + right[r + 1] * temp;
                saved = left[j - r] * temp;
            }
            values[j] = saved;
        }
        values
    }
    pub fn evaluate(&self, t: f64) -> (usize, [f64; 17], [f64; 17]) {
        let [a, b] = self.range();
        let t = t.clamp(a, b);
        let span = (self.knots.partition_point(|&x| x <= t).saturating_sub(1))
            .clamp(self.degree, self.count - 1);
        let values = self.values_at_span(span, self.degree, t);
        let lower = self.values_at_span(span, self.degree - 1, t);
        let mut derivative = [0.; 17];
        for (j, d) in derivative.iter_mut().enumerate().take(self.degree + 1) {
            let i = span - self.degree + j;
            if j > 0 {
                let denominator = self.knots[i + self.degree] - self.knots[i];
                if denominator > 0. {
                    *d += self.degree as f64 * lower[j - 1] / denominator;
                }
            }
            if j < self.degree {
                let denominator = self.knots[i + self.degree + 1] - self.knots[i + 1];
                if denominator > 0. {
                    *d -= self.degree as f64 * lower[j] / denominator;
                }
            }
        }
        (span - self.degree, values, derivative)
    }
}

#[derive(Clone, Debug)]
pub(super) struct Curve {
    pub basis: Basis,
    pub points: Vec<V3>,
    pub weights: Vec<f64>,
}
impl Curve {
    pub fn validate(&self) -> Result<()> {
        if self.points.len() != self.basis.count
            || self.weights.len() != self.points.len()
            || self.points.iter().any(|p| !p.finite())
            || self.weights.iter().any(|w| !w.is_finite() || *w <= 0.)
        {
            return Err(crate::Error::step(
                "STEP: 曲線の制御点・重みが不正です".into(),
            ));
        }
        Ok(())
    }
    pub fn evaluate(&self, t: f64) -> (V3, V3) {
        let (start, n, d) = self.basis.evaluate(t);
        let (mut p, mut derivative, mut w, mut dw) = (V3::default(), V3::default(), 0., 0.);
        for j in 0..=self.basis.degree {
            let weight = self.weights[start + j];
            p = p + self.points[start + j] * (n[j] * weight);
            derivative = derivative + self.points[start + j] * (d[j] * weight);
            w += n[j] * weight;
            dw += d[j] * weight;
        }
        p = p / w;
        (p, (derivative - p * dw) / w)
    }
}

#[derive(Clone, Debug)]
pub(super) struct Surface {
    pub u: Basis,
    pub v: Basis,
    pub points: Vec<V3>,
    pub weights: Vec<f64>,
    pub closed: [bool; 2],
}
impl Surface {
    pub fn validate(&self) -> Result<()> {
        if self
            .u
            .count
            .checked_mul(self.v.count)
            .is_none_or(|n| n > 65_536 || self.points.len() != n)
            || self.weights.len() != self.points.len()
            || self.points.iter().any(|p| !p.finite())
            || self.weights.iter().any(|w| !w.is_finite() || *w <= 0.)
        {
            return Err(crate::Error::step(
                "STEP: 曲面の制御点・重みが不正です".into(),
            ));
        }
        Ok(())
    }
    pub fn evaluate(&self, uv: [f64; 2]) -> (V3, V3, V3) {
        let (i0, u, du) = self.u.evaluate(uv[0]);
        let (j0, v, dv) = self.v.evaluate(uv[1]);
        let (mut p, mut pu, mut pv, mut w, mut wu, mut wv) =
            (V3::default(), V3::default(), V3::default(), 0., 0., 0.);
        for i in 0..=self.u.degree {
            for j in 0..=self.v.degree {
                let k = (i0 + i) * self.v.count + j0 + j;
                let weight = self.weights[k];
                let point = self.points[k];
                let (n, nu, nv) = (
                    u[i] * v[j] * weight,
                    du[i] * v[j] * weight,
                    u[i] * dv[j] * weight,
                );
                p = p + point * n;
                pu = pu + point * nu;
                pv = pv + point * nv;
                w += n;
                wu += nu;
                wv += nv;
            }
        }
        p = p / w;
        (p, (pu - p * wu) / w, (pv - p * wv) / w)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rational_circle_and_bilinear_surface_have_analytic_derivatives() {
        let c = Curve {
            basis: Basis::new(2, 3, &[3., 3.], &[0., 1.]).unwrap(),
            points: vec![V3([1., 0., 0.]), V3([1., 1., 0.]), V3([0., 1., 0.])],
            weights: vec![1., 0.5f64.sqrt(), 1.],
        };
        c.validate().unwrap();
        for i in 0..=100 {
            let (p, d) = c.evaluate(i as f64 / 100.);
            assert!((p.norm() - 1.).abs() < 2e-15);
            assert!(p.dot(d).abs() < 2e-15);
            assert!(d.0[0] <= 0. && d.0[1] >= 0.);
        }
        let b = Basis::new(1, 2, &[2., 2.], &[0., 1.]).unwrap();
        let s = Surface {
            u: b.clone(),
            v: b,
            points: vec![
                V3([0., 0., 0.]),
                V3([0., 1., 0.]),
                V3([1., 0., 0.]),
                V3([1., 1., 1.]),
            ],
            weights: vec![1.; 4],
            closed: [false; 2],
        };
        s.validate().unwrap();
        for uv in [[0., 0.], [1., 1.], [0.3, 0.7]] {
            let (p, du, dv) = s.evaluate(uv);
            assert!((p - V3([uv[0], uv[1], uv[0] * uv[1]])).norm() < 1e-14);
            assert!((du - V3([1., 0., uv[1]])).norm() < 1e-14);
            assert!((dv - V3([0., 1., uv[0]])).norm() < 1e-14);
        }
    }
}
