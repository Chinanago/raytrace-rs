//! Sequential entrance pupils and real-ray targeting through refractive optics.
use crate::{
    geometry,
    math::{Frame, Pose, V3},
    model::{Aperture, Interaction, PreparedScene, PreparedSurface, Shape, Source},
};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RayAiming {
    #[default]
    Off,
    Paraxial,
    Real,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OpticalPupil {
    pub first_surface: usize,
    pub primary_wavelength_um: f64,
    pub aiming: RayAiming,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub finite_field_angle_deg: Option<[f64; 2]>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Vignetting {
    #[serde(default)]
    pub decenter: [f64; 2],
    #[serde(default)]
    pub compression: [f64; 2],
    #[serde(default)]
    pub tangential_angle_deg: f64,
}
impl Vignetting {
    pub fn is_zero(&self) -> bool {
        self.decenter == [0.; 2] && self.compression == [0.; 2] && self.tangential_angle_deg == 0.
    }
    pub fn validate(&self) -> Result<(), String> {
        if self
            .decenter
            .iter()
            .chain(&self.compression)
            .any(|v| !v.is_finite())
            || !self.tangential_angle_deg.is_finite()
            || self.compression.iter().any(|v| *v > 1.)
        {
            return Err("視野の瞳補正は有限値、VCX/VCYは1以下で指定してください".into());
        }
        Ok(())
    }
    pub fn transform(&self, p: [f64; 2]) -> [f64; 2] {
        let x = self.decenter[0] + p[0] * (1. - self.compression[0]);
        let y = self.decenter[1] + p[1] * (1. - self.compression[1]);
        let (s, c) = self.tangential_angle_deg.to_radians().sin_cos();
        [x * c - y * s, x * s + y * c]
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PupilGeometry {
    pub entrance_pose: Pose,
    pub object_origin_mm: V3,
    pub entrance_radius_mm: f64,
    pub paraxial_stop_radius_mm: f64,
    pub target_stop_radius_mm: f64,
    pub primary_wavelength_um: f64,
    pub aiming: RayAiming,
}

pub(crate) struct PreparedPupil {
    pub geometry: PupilGeometry,
    frame: Frame,
    entrance_z: f64,
    object_origin: V3,
    direction: V3,
    finite: bool,
    surfaces: Vec<PreparedSurface>,
    materials: Vec<crate::model::Material>,
    initial_medium: usize,
}

/// Returns the pupil obtained from the current layout, including its primary
/// wavelength. This is also used by the GUI's saved calculation conditions.
pub fn pupil_geometry(
    scene: &PreparedScene,
    source_index: usize,
) -> Result<Option<PupilGeometry>, String> {
    let source = scene
        .spec
        .sources
        .get(source_index)
        .ok_or("光源番号が不正です")?;
    Ok(PreparedPupil::new(scene, source)?.map(|p| p.geometry))
}

/// Compute saved pupil conditions using only the relevant analytic prefix;
/// downstream CAD meshes do not need another acceleration structure.
pub fn pupil_geometry_from_scene(
    scene: &crate::model::Scene,
    source_index: usize,
) -> Result<Option<PupilGeometry>, String> {
    let mut source = scene
        .sources
        .get(source_index)
        .ok_or("光源番号が不正です")?
        .clone();
    let Some(p) = &mut source.pupil else {
        return Ok(None);
    };
    let Some(o) = &mut p.optical else {
        return Ok(None);
    };
    let first = o.first_surface;
    let stop = p.surface.ok_or("STOP面がありません")?;
    if first > stop || stop >= scene.surfaces.len() {
        return Err("入射瞳の参照面が不正です".into());
    }
    o.first_surface = 0;
    p.surface = Some(stop - first);
    let prefix = crate::model::Scene {
        name: scene.name.clone(),
        reference_plane: scene.reference_plane.clone(),
        mode: crate::model::Mode::Sequential,
        materials: scene.materials.clone(),
        surfaces: scene.surfaces[first..=stop].to_vec(),
        sources: vec![source],
        settings: scene.settings.clone(),
    };
    pupil_geometry(&prefix.prepare()?, 0)
}

impl PreparedPupil {
    pub(crate) fn new(scene: &PreparedScene, source: &Source) -> Result<Option<Self>, String> {
        let Some(p) = &source.pupil else {
            return Ok(None);
        };
        p.vignetting.validate()?;
        let Some(o) = &p.optical else {
            return Ok(None);
        };
        let stop = p.surface.ok_or("光学的な瞳にはSTOP面の参照が必要です")?;
        if o.first_surface > stop
            || stop >= scene.surfaces.len()
            || !o.primary_wavelength_um.is_finite()
            || o.primary_wavelength_um <= 0.
        {
            return Err("入射瞳の面番号・主波長が不正です".into());
        }
        let Aperture::Circle {
            radius_mm: entrance_radius,
        } = p.aperture
        else {
            return Err("光学的な入射瞳は円形のENPDに対応しています".into());
        };
        let frame = scene.surfaces[o.first_surface].frame.clone();
        let mut z = 0.;
        let mut medium = source.medium;
        let index = |id: usize| {
            scene
                .spec
                .materials
                .get(id)
                .and_then(|m| m.index(o.primary_wavelength_um))
                .ok_or("入射瞳の主波長が材料の有効範囲外です")
        };
        let n0 = index(medium)?;
        let (mut a, mut b, mut c, mut d) = (1., 0., 0., 1.);
        for i in o.first_surface..=stop {
            let surface = &scene.surfaces[i];
            let pos = frame.local_point(surface.spec.pose.translation_mm);
            let axis = frame.local_vector(surface.frame.world_vector(V3([0., 0., 1.])));
            if pos.0[0].hypot(pos.0[1]) > 1e-8
                || (axis - V3([0., 0., 1.])).norm() > 1e-10
                || pos.0[2] < z - 1e-9
            {
                return Err(
                    "入射瞳の計算は、STOPまでが同軸で正の面間隔を持つ屈折系に対応しています".into(),
                );
            }
            if !matches!(
                surface.spec.shape,
                Shape::Plane
                    | Shape::Sphere { .. }
                    | Shape::EvenAsphere { .. }
                    | Shape::FlatEvenAsphere { .. }
            ) {
                return Err("入射瞳の計算は平面・球面・偶数次非球面に対応しています".into());
            }
            let distance = pos.0[2] - z;
            let n = index(medium)?;
            a += distance / n * c;
            b += distance / n * d;
            z = pos.0[2];
            if i == stop {
                break;
            }
            let Interaction::Dielectric {
                minus_medium,
                plus_medium,
                ..
            } = surface.spec.interaction
            else {
                return Err("STOPより前の反射・吸収・検出面を含む入射瞳は未対応です".into());
            };
            if medium != minus_medium {
                return Err("入射瞳の媒質の連続性が不正です".into());
            }
            let curvature = match surface.spec.shape {
                Shape::Sphere { radius_mm } | Shape::EvenAsphere { radius_mm, .. } => {
                    1. / radius_mm
                }
                _ => 0.,
            };
            let power = -(index(plus_medium)? - n) * curvature;
            c += power * a;
            d += power * b;
            medium = plus_medium;
        }
        if !a.is_finite() || !b.is_finite() || a.abs() < 1e-12 {
            return Err(
                "入射瞳が無限遠またはSTOPが焦線上にある系は、現在の瞳計算では扱えません".into(),
            );
        }
        let entrance_z = n0 * b / a;
        let mut origin = frame.local_point(source.origin_mm);
        if let Some(angles) = o.finite_field_angle_deg {
            if !p.finite_object || angles.iter().any(|v| !v.is_finite() || v.abs() >= 90.) {
                return Err("有限物体のAngle視野が不正です".into());
            }
            origin.0[0] = -angles[0].to_radians().tan() * (entrance_z - origin.0[2]);
            origin.0[1] = -angles[1].to_radians().tan() * (entrance_z - origin.0[2]);
        }
        if p.finite_object && (entrance_z - origin.0[2]).abs() < 1e-10 {
            return Err("物体と入射瞳が共役で光線の方向を決められません".into());
        }
        let direction = frame.local_vector(source.direction.unit());
        if !p.finite_object && direction.0[2] <= 1e-10 {
            return Err("入射瞳へ向かう光束は前方方向で指定してください".into());
        }
        let mut surfaces = scene.surfaces[o.first_surface..=stop].to_vec();
        // Aiming follows the mathematical optical surfaces. Physical apertures
        // remain in the tracing scene, where their losses are accounted for.
        let field_extent = origin.0[0].hypot(origin.0[1])
            + (z - origin.0[2]).abs() * direction.0[0].hypot(direction.0[1])
                / direction.0[2].abs().max(1e-9);
        for s in &mut surfaces {
            if o.aiming == RayAiming::Off {
                break;
            }
            if matches!(s.spec.shape, Shape::Plane) {
                continue;
            }
            let r = s.spec.aperture.max_radius2().sqrt();
            let requested = (4. * r).max(8. * entrance_radius + 2. * field_extent);
            let domain = match s.spec.shape {
                Shape::Sphere { radius_mm } => radius_mm.abs(),
                Shape::EvenAsphere {
                    radius_mm, conic, ..
                } if conic > -1. => radius_mm.abs() / (1. + conic).sqrt(),
                _ => f64::INFINITY,
            };
            s.spec.aperture = Aperture::Circle {
                radius_mm: requested.min(domain * (1. - 1e-7)).max(r),
            };
            let (bounds, slope) = geometry::bounds(&s.spec.shape, &s.spec.aperture)
                .ok_or("Ray Aimingの面の探索範囲を準備できません")?;
            s.local_bounds = bounds;
            s.max_sag_slope = slope;
        }
        let paraxial_radius = a.abs() * entrance_radius;
        let mut result = Self {
            geometry: PupilGeometry {
                entrance_pose: Pose {
                    translation_mm: frame.world_point(V3([0., 0., entrance_z])),
                    rotation_deg: scene.surfaces[o.first_surface].spec.pose.rotation_deg,
                },
                object_origin_mm: frame.world_point(origin),
                entrance_radius_mm: entrance_radius,
                paraxial_stop_radius_mm: paraxial_radius,
                target_stop_radius_mm: paraxial_radius,
                primary_wavelength_um: o.primary_wavelength_um,
                aiming: o.aiming,
            },
            frame,
            entrance_z,
            object_origin: origin,
            direction,
            finite: p.finite_object,
            surfaces,
            materials: scene.spec.materials.clone(),
            initial_medium: source.medium,
        };
        if o.aiming == RayAiming::Real {
            let mut reference = result.object_origin;
            reference.0[0] = 0.;
            reference.0[1] = 0.;
            let q = result.guess([entrance_radius, 0.], reference, V3([0., 0., 1.]));
            let hit = result.stop_hit(q, o.primary_wavelength_um, reference, V3([0., 0., 1.]))?;
            result.geometry.target_stop_radius_mm = hit[0].hypot(hit[1]);
        }
        if !result.geometry.target_stop_radius_mm.is_finite()
            || result.geometry.target_stop_radius_mm <= 0.
        {
            return Err("STOPへの基準光線から有効な半径を求められません".into());
        }
        Ok(Some(result))
    }
    fn guess(&self, xy: [f64; 2], origin: V3, direction: V3) -> [f64; 2] {
        if self.finite {
            [
                (xy[0] - origin.0[0]) / (self.entrance_z - origin.0[2]),
                (xy[1] - origin.0[1]) / (self.entrance_z - origin.0[2]),
            ]
        } else {
            [
                xy[0] - direction.0[0] / direction.0[2] * (self.entrance_z - origin.0[2]),
                xy[1] - direction.0[1] / direction.0[2] * (self.entrance_z - origin.0[2]),
            ]
        }
    }
    fn ray(&self, q: [f64; 2], origin: V3, direction: V3) -> (V3, V3) {
        if self.finite {
            (
                self.frame.world_point(origin),
                self.frame.world_vector(V3([q[0], q[1], 1.]).unit()),
            )
        } else {
            (
                self.frame.world_point(V3([q[0], q[1], origin.0[2]])),
                self.frame.world_vector(direction).unit(),
            )
        }
    }
    fn stop_hit(
        &self,
        q: [f64; 2],
        wavelength: f64,
        origin: V3,
        direction: V3,
    ) -> Result<[f64; 2], String> {
        let (mut pos, mut dir) = self.ray(q, origin, direction);
        let mut medium = self.initial_medium;
        for (i, s) in self.surfaces.iter().enumerate() {
            let (point, local, normal) = if matches!(s.spec.shape, Shape::Plane) {
                let p = s.frame.local_point(pos);
                let d = s.frame.local_vector(dir);
                if d.0[2].abs() < 1e-12 {
                    return Err("Ray Aimingの光線が光学面と平行です".into());
                }
                let t = -p.0[2] / d.0[2];
                if t < -1e-8 {
                    return Err("Ray Aimingの光学面が光線の後方にあります".into());
                }
                let local = p + d * t;
                (
                    s.frame.world_point(local),
                    local,
                    s.frame.world_vector(V3([0., 0., 1.])),
                )
            } else {
                let h = geometry::intersect_ordered(s, pos, dir)
                    .map_err(|e| format!("Ray Aimingの面交差に失敗しました: {e:?}"))?;
                (h.point, h.local_point, h.normal)
            };
            if i + 1 == self.surfaces.len() {
                return Ok([local.0[0], local.0[1]]);
            }
            let Interaction::Dielectric {
                minus_medium,
                plus_medium,
                ..
            } = s.spec.interaction
            else {
                return Err("Ray Aimingの光学面が屈折面ではありません".into());
            };
            if medium != minus_medium {
                return Err("Ray Aimingの媒質が一致しません".into());
            }
            let n1 = self.materials[medium]
                .index(wavelength)
                .ok_or("Ray Aimingの波長が材料の有効範囲外です")?;
            let n2 = self.materials[plus_medium]
                .index(wavelength)
                .ok_or("Ray Aimingの波長が材料の有効範囲外です")?;
            dir = geometry::dielectric(dir, normal, n1, n2)
                .transmitted
                .ok_or("STOPまでのRay Aimingの光路で全反射します")?;
            pos = point;
            medium = plus_medium;
        }
        Err("STOPがありません".into())
    }
    pub(crate) fn launch(
        &self,
        source: &Source,
        wavelength: f64,
        u: [f64; 2],
    ) -> Result<(V3, V3), String> {
        let p = source.pupil.as_ref().unwrap();
        let phi = std::f64::consts::TAU * u[1];
        let r = u[0].sqrt();
        let xy = p.vignetting.transform([r * phi.cos(), r * phi.sin()]);
        let mut q = self.guess(
            xy.map(|v| v * self.geometry.entrance_radius_mm),
            self.object_origin,
            self.direction,
        );
        if self.geometry.aiming == RayAiming::Off {
            return Ok(self.ray(q, self.object_origin, self.direction));
        }
        let target = xy.map(|v| v * self.geometry.target_stop_radius_mm);
        let tolerance = 1e-9 + self.geometry.target_stop_radius_mm * 1e-10;
        for _ in 0..24 {
            let hit = self.stop_hit(q, wavelength, self.object_origin, self.direction)?;
            let error = [hit[0] - target[0], hit[1] - target[1]];
            let norm = error[0].hypot(error[1]);
            if norm <= tolerance {
                return Ok(self.ray(q, self.object_origin, self.direction));
            }
            let mut jac = [[0.; 2]; 2];
            for axis in 0..2 {
                let step = 1e-5 * (1. + q[axis].abs());
                let mut plus = q;
                plus[axis] += step;
                let mut minus = q;
                minus[axis] -= step;
                match (
                    self.stop_hit(plus, wavelength, self.object_origin, self.direction),
                    self.stop_hit(minus, wavelength, self.object_origin, self.direction),
                ) {
                    (Ok(a), Ok(b)) => {
                        for j in 0..2 {
                            jac[j][axis] = (a[j] - b[j]) / (2. * step);
                        }
                    }
                    (Ok(a), Err(_)) => {
                        for j in 0..2 {
                            jac[j][axis] = (a[j] - hit[j]) / step;
                        }
                    }
                    (Err(_), Ok(b)) => {
                        for j in 0..2 {
                            jac[j][axis] = (hit[j] - b[j]) / step;
                        }
                    }
                    _ => return Err("Ray Aimingの局所的な光路を求められません".into()),
                }
            }
            let det = jac[0][0] * jac[1][1] - jac[0][1] * jac[1][0];
            let scale = jac.iter().flatten().map(|v| v.abs()).fold(0., f64::max);
            if !det.is_finite() || det.abs() <= 1e-12 * scale * scale {
                return Err("STOPが焦線に近く、Ray Aimingの解が一意に定まりません".into());
            }
            let delta = [
                (jac[1][1] * error[0] - jac[0][1] * error[1]) / det,
                (-jac[1][0] * error[0] + jac[0][0] * error[1]) / det,
            ];
            let mut accepted = false;
            for backtrack in 0..12 {
                let rate = 0.5_f64.powi(backtrack);
                let next = [q[0] - rate * delta[0], q[1] - rate * delta[1]];
                if let Ok(h) = self.stop_hit(next, wavelength, self.object_origin, self.direction) {
                    if (h[0] - target[0]).hypot(h[1] - target[1]) < norm {
                        q = next;
                        accepted = true;
                        break;
                    }
                }
            }
            if !accepted {
                return Err(format!("Ray Aimingが収束しません（絞り上の誤差 {norm:.3e} mm）。瞳径・視野・STOP位置を確認してください"));
            }
        }
        Err("Ray Aimingが24回で収束しませんでした".into())
    }
}
