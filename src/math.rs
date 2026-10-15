use serde::{Deserialize, Serialize};
use std::ops::{Add, Div, Mul, Neg, Sub};

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(transparent)]
pub struct V3(pub [f64; 3]);

impl V3 {
    pub fn cross(self, r: Self) -> Self {
        Self([
            self.0[1] * r.0[2] - self.0[2] * r.0[1],
            self.0[2] * r.0[0] - self.0[0] * r.0[2],
            self.0[0] * r.0[1] - self.0[1] * r.0[0],
        ])
    }
    pub fn dot(self, other: Self) -> f64 {
        self.0[0] * other.0[0] + self.0[1] * other.0[1] + self.0[2] * other.0[2]
    }
    pub fn norm(self) -> f64 {
        self.dot(self).sqrt()
    }
    pub fn unit(self) -> Self {
        self / self.norm()
    }
    pub fn finite(self) -> bool {
        self.0.iter().all(|v| v.is_finite())
    }
}

impl Add for V3 {
    type Output = Self;
    fn add(self, r: Self) -> Self {
        Self(std::array::from_fn(|i| self.0[i] + r.0[i]))
    }
}
impl Sub for V3 {
    type Output = Self;
    fn sub(self, r: Self) -> Self {
        Self(std::array::from_fn(|i| self.0[i] - r.0[i]))
    }
}
impl Mul<f64> for V3 {
    type Output = Self;
    fn mul(self, r: f64) -> Self {
        Self(self.0.map(|v| v * r))
    }
}
impl Div<f64> for V3 {
    type Output = Self;
    fn div(self, r: f64) -> Self {
        self * (1.0 / r)
    }
}
impl Neg for V3 {
    type Output = Self;
    fn neg(self) -> Self {
        self * -1.0
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Pose {
    #[serde(default)]
    pub translation_mm: V3,
    /// Active rotations: Rx, then Ry, then Rz, in the world frame.
    #[serde(default)]
    pub rotation_deg: V3,
}

#[derive(Clone, Debug)]
pub struct Frame {
    pub origin: V3,
    axes: [V3; 3],
}

impl Frame {
    pub fn new(pose: &Pose) -> Self {
        let [x, y, z] = pose.rotation_deg.0.map(f64::to_radians);
        let (sx, cx) = x.sin_cos();
        let (sy, cy) = y.sin_cos();
        let (sz, cz) = z.sin_cos();
        Self {
            origin: pose.translation_mm,
            axes: [
                V3([cz * cy, sz * cy, -sy]),
                V3([cz * sy * sx - sz * cx, sz * sy * sx + cz * cx, cy * sx]),
                V3([cz * sy * cx + sz * sx, sz * sy * cx - cz * sx, cy * cx]),
            ],
        }
    }
    pub fn local_vector(&self, v: V3) -> V3 {
        V3(self.axes.map(|axis| axis.dot(v)))
    }
    pub fn local_point(&self, p: V3) -> V3 {
        self.local_vector(p - self.origin)
    }
    pub fn world_vector(&self, v: V3) -> V3 {
        self.axes[0] * v.0[0] + self.axes[1] * v.0[1] + self.axes[2] * v.0[2]
    }
    pub fn world_point(&self, p: V3) -> V3 {
        self.origin + self.world_vector(p)
    }
}
