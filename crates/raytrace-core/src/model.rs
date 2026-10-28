use crate::Result;
use crate::math::{Frame, Pose, V3};
pub use crate::sampling::Sampling;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

mod material;
mod source;

pub use material::Material;
pub use source::{Diode, PupilIllumination, Ray, RayGenerator, Source};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum Shape {
    /// Side of a closed lens barrel: z in [0,length], outward-pointing normal.
    ConicalSide {
        radius_start_mm: f64,
        radius_end_mm: f64,
        length_mm: f64,
    },
    TriangleMesh {
        vertices: Vec<V3>,
        triangles: Vec<[u32; 3]>,
        normals: Vec<V3>,
    },
    Plane,
    /// Coefficients are A4, A6, A8, ... in increasing order.
    FlatEvenAsphere {
        #[serde(default)]
        coefficients: Vec<f64>,
    },
    /// The vertex-centred sag cap; radius is signed. This is not a full sphere.
    Sphere {
        radius_mm: f64,
    },
    /// Coefficients are A4, A6, A8, ... in increasing order.
    EvenAsphere {
        radius_mm: f64,
        conic: f64,
        #[serde(default)]
        coefficients: Vec<f64>,
    },
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum Aperture {
    Circle {
        radius_mm: f64,
    },
    Rectangle {
        half_width_mm: f64,
        half_height_mm: f64,
    },
}
impl Aperture {
    pub fn contains(&self, x: f64, y: f64) -> bool {
        match *self {
            Self::Circle { radius_mm } => {
                x * x + y * y <= radius_mm * radius_mm * (1. + 32. * f64::EPSILON)
            }
            Self::Rectangle {
                half_width_mm,
                half_height_mm,
            } => {
                x.abs() <= half_width_mm * (1. + 32. * f64::EPSILON)
                    && y.abs() <= half_height_mm * (1. + 32. * f64::EPSILON)
            }
        }
    }
    pub fn bounds(&self) -> (f64, f64) {
        match *self {
            Self::Circle { radius_mm } => (radius_mm, radius_mm),
            Self::Rectangle {
                half_width_mm,
                half_height_mm,
            } => (half_width_mm, half_height_mm),
        }
    }
    pub fn max_radius2(&self) -> f64 {
        match *self {
            Self::Circle { radius_mm } => radius_mm * radius_mm,
            Self::Rectangle {
                half_width_mm,
                half_height_mm,
            } => half_width_mm * half_width_mm + half_height_mm * half_height_mm,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum Interaction {
    /// The minus/plus medium is on the -/+ side of the local sag normal.
    Dielectric {
        minus_medium: usize,
        plus_medium: usize,
        reflectance: Option<f64>,
    },
    Mirror {
        reflectance: f64,
    },
    Absorb,
    Detector {
        pixels: [usize; 2],
    },
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Surface {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub coating: Option<crate::coating::Coating>,
    #[serde(default)]
    pub pose: Pose,
    pub shape: Shape,
    pub aperture: Aperture,
    pub interaction: Interaction,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum Mode {
    Sequential,
    NonSequential,
}

fn default_depth() -> usize {
    64
}
fn default_events() -> usize {
    4096
}
fn default_cutoff() -> f64 {
    1e-7
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Settings {
    #[serde(default = "default_depth")]
    pub max_depth: usize,
    #[serde(default = "default_events")]
    pub max_events_per_launch: usize,
    #[serde(default = "default_cutoff")]
    pub min_relative_branch_weight: f64,
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            max_depth: default_depth(),
            max_events_per_launch: default_events(),
            min_relative_branch_weight: default_cutoff(),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Scene {
    pub name: String,
    pub reference_plane: String,
    pub mode: Mode,
    pub materials: Vec<Material>,
    pub surfaces: Vec<Surface>,
    #[serde(default)]
    pub settings: Settings,
    #[serde(default)]
    pub sources: Vec<Source>,
}

#[derive(Clone, Debug)]
pub struct PreparedSurface {
    pub mesh: Option<crate::mesh::MeshAccel>,
    pub spec: Surface,
    pub frame: Frame,
    pub detector_index: Option<usize>,
    pub local_bounds: [V3; 2],
    pub max_sag_slope: f64,
}
#[derive(Clone, Debug)]
pub struct PreparedScene {
    pub spec: Scene,
    pub surfaces: Vec<PreparedSurface>,
    pub detector_surfaces: Vec<usize>,
}

fn positive(v: f64) -> bool {
    v.is_finite() && v > 0.0
}
fn fraction(v: f64) -> bool {
    v.is_finite() && (0.0..=1.0).contains(&v)
}
impl Scene {
    pub fn prepare(self) -> Result<PreparedScene> {
        if self.materials.is_empty() || self.surfaces.is_empty() {
            return Err(crate::Error::InvalidScene(
                "材料と面をそれぞれ1つ以上指定してください".into(),
            ));
        }
        let s = &self.settings;
        if s.max_depth == 0
            || s.max_events_per_launch == 0
            || !fraction(s.min_relative_branch_weight)
        {
            return Err(crate::Error::InvalidScene(
                "追跡の上限設定が不正です".into(),
            ));
        }
        for m in &self.materials {
            if !m.absorption().is_finite() || m.absorption() < 0.0 {
                return Err(crate::Error::InvalidScene(
                    "吸収係数は非負の有限値で指定してください".into(),
                ));
            }
            match m {
                Material::Catalog {
                    formula,
                    coefficients,
                    valid_range_um,
                    ..
                } if crate::dispersion::coefficient_count(*formula)
                    .is_none_or(|n| coefficients.len() < n)
                    || !coefficients.iter().all(|v| v.is_finite())
                    || !positive(valid_range_um[0])
                    || !positive(valid_range_um[1])
                    || valid_range_um[0] >= valid_range_um[1] =>
                {
                    return Err(crate::Error::InvalidScene(
                        "AGFの分散係数・有効波長範囲が不正です".into(),
                    ));
                }
                Material::WithAbsorption {
                    dispersion,
                    samples,
                } if samples.is_empty()
                    || !samples
                        .iter()
                        .all(|s| positive(s[0]) && s[1].is_finite() && s[1] >= 0.0)
                    || !samples.windows(2).all(|w| w[0][0] < w[1][0])
                    || !samples.iter().any(|s| dispersion.index(s[0]).is_some()) =>
                {
                    return Err(crate::Error::InvalidScene(
                        "カタログの吸収・分散データが不正です".into(),
                    ));
                }
                Material::Constant { n, .. } if !positive(*n) => {
                    return Err(crate::Error::InvalidScene("屈折率が不正です".into()));
                }
                Material::Sellmeier {
                    b,
                    c_um2,
                    valid_range_um,
                    ..
                } if !b.iter().chain(c_um2).all(|v| v.is_finite())
                    || !positive(valid_range_um[0])
                    || !positive(valid_range_um[1])
                    || valid_range_um[0] >= valid_range_um[1] =>
                {
                    return Err(crate::Error::InvalidScene(
                        "Sellmeierの係数・有効波長範囲が不正です".into(),
                    ));
                }
                Material::Schott {
                    coefficients,
                    valid_range_um,
                    ..
                } if !coefficients.iter().all(|v| v.is_finite())
                    || !positive(valid_range_um[0])
                    || !positive(valid_range_um[1])
                    || valid_range_um[0] >= valid_range_um[1] =>
                {
                    return Err(crate::Error::InvalidScene(
                        "Schottの係数・有効波長範囲が不正です".into(),
                    ));
                }
                Material::Tabulated { samples, .. }
                    if samples.len() < 2
                        || !samples.iter().all(|s| positive(s[0]) && positive(s[1]))
                        || !samples.windows(2).all(|s| s[0][0] < s[1][0]) =>
                {
                    return Err(crate::Error::InvalidScene("屈折率の表が不正です".into()));
                }
                _ => (),
            }
        }
        let mut names = HashSet::new();
        let mut detector_surfaces = Vec::new();
        let mut surfaces = Vec::new();
        let mut total_pixels = 0usize;
        for (i, surface) in self.surfaces.iter().enumerate() {
            if let Some(coating) = &surface.coating {
                coating.validate()?;
                match surface.interaction {
                    Interaction::Dielectric {
                        reflectance: None, ..
                    } => (),
                    Interaction::Mirror { .. }
                        if coating.front.opaque()
                            && coating.back.as_ref().is_none_or(|b| b.opaque()) => {}
                    _ => {
                        return Err(crate::Error::InvalidScene(format!(
                            "{}: コーティングには屈折界面、または透過率0のミラーを指定してください",
                            surface.name
                        )));
                    }
                }
            }
            if !names.insert(surface.name.clone()) {
                return Err(crate::Error::InvalidScene(
                    "面の名前が重複しています".into(),
                ));
            }
            if !surface.pose.translation_mm.finite() || !surface.pose.rotation_deg.finite() {
                return Err(crate::Error::InvalidScene(format!(
                    "位置・角度が不正です: {}",
                    surface.name
                )));
            }
            let (hx, hy) = surface.aperture.bounds();
            if !positive(hx) || !positive(hy) {
                return Err(crate::Error::InvalidScene("開口の寸法が不正です".into()));
            }
            match &surface.shape {
                Shape::ConicalSide {
                    radius_start_mm,
                    radius_end_mm,
                    length_mm,
                } if !positive(*radius_start_mm)
                    || !positive(*radius_end_mm)
                    || !positive(*length_mm) =>
                {
                    return Err(crate::Error::InvalidScene(
                        "レンズ側面の寸法が不正です".into(),
                    ));
                }
                Shape::TriangleMesh {
                    vertices,
                    triangles,
                    normals,
                } if vertices.is_empty()
                    || triangles.is_empty()
                    || triangles.len() > 1_000_000
                    || normals.len() != vertices.len()
                    || !vertices.iter().all(|p| p.finite())
                    || !normals.iter().all(|n| n.finite() && n.norm() > 1e-12)
                    || triangles
                        .iter()
                        .flatten()
                        .any(|&i| i as usize >= vertices.len()) =>
                {
                    return Err(crate::Error::InvalidScene(
                        "三角形メッシュが不正です".into(),
                    ));
                }
                Shape::Sphere { radius_mm }
                    if !positive(radius_mm.abs())
                        || surface.aperture.max_radius2() >= radius_mm * radius_mm =>
                {
                    return Err(crate::Error::InvalidScene(
                        "球面の開口半径は曲率半径の絶対値より小さくしてください".into(),
                    ));
                }
                Shape::FlatEvenAsphere { coefficients }
                    if coefficients.len() > 8 || !coefficients.iter().all(|v| v.is_finite()) =>
                {
                    return Err(crate::Error::InvalidScene(
                        "偶数次非球面が不正、または開口内に特異点があります".into(),
                    ));
                }
                Shape::EvenAsphere {
                    radius_mm,
                    conic,
                    coefficients,
                } if !positive(radius_mm.abs())
                    || !conic.is_finite()
                    || coefficients.len() > 8
                    || !coefficients.iter().all(|v| v.is_finite())
                    || 1.0 - (1.0 + conic) * surface.aperture.max_radius2() / radius_mm.powi(2)
                        <= 1e-12 =>
                {
                    return Err(crate::Error::InvalidScene(
                        "偶数次非球面が不正、または開口内に特異点があります".into(),
                    ));
                }
                _ => (),
            }
            let detector_index = match surface.interaction {
                Interaction::Dielectric {
                    minus_medium,
                    plus_medium,
                    reflectance,
                } => {
                    if minus_medium >= self.materials.len()
                        || plus_medium >= self.materials.len()
                        || reflectance.is_some_and(|r| !fraction(r))
                    {
                        return Err(crate::Error::InvalidScene(
                            "屈折界面の媒質・反射率が不正です".into(),
                        ));
                    }
                    None
                }
                Interaction::Mirror { reflectance } if !fraction(reflectance) => {
                    return Err(crate::Error::InvalidScene(
                        "ミラーの反射率が不正です".into(),
                    ));
                }
                Interaction::Detector { pixels } => {
                    if !matches!(surface.shape, Shape::Plane) || pixels.contains(&0) {
                        return Err(crate::Error::InvalidScene(
                            "検出面は平面とし、画素数は正の値で指定してください".into(),
                        ));
                    }
                    total_pixels = total_pixels
                        .checked_add(pixels[0].checked_mul(pixels[1]).ok_or_else(|| {
                            crate::Error::Limit("画素数が整数の上限を超えています".into())
                        })?)
                        .ok_or_else(|| {
                            crate::Error::Limit("画素数が整数の上限を超えています".into())
                        })?;
                    if total_pixels > 1_000_000 {
                        return Err(crate::Error::Limit(
                            "試作版の検出面は合計100万画素までです".into(),
                        ));
                    }
                    detector_surfaces.push(i);
                    Some(detector_surfaces.len() - 1)
                }
                _ => None,
            };
            let (local_bounds, max_sag_slope) =
                crate::geometry::bounds(&surface.shape, &surface.aperture).ok_or_else(|| {
                    crate::Error::InvalidScene("面の境界が計算範囲を超えています".into())
                })?;
            surfaces.push(PreparedSurface {
                mesh: crate::mesh::MeshAccel::from_shape(&surface.shape),
                spec: surface.clone(),
                frame: Frame::new(&surface.pose),
                detector_index,
                local_bounds,
                max_sag_slope,
            });
        }
        for (source_index, source) in self.sources.iter().enumerate() {
            (|| -> Result<()> {
            if let Some(pupil) = &source.pupil {
                pupil.vignetting.validate()?;
                let pose = if let Some(i) = pupil.surface {
                    &self.surfaces.get(i).ok_or_else(|| crate::Error::InvalidScene("瞳の参照面が不正です".into()))?.pose
                } else {
                    &pupil.pose
                };
                let frame = Frame::new(pose);
                let distance = frame.local_point(source.origin_mm).0[2];
                let (hx, hy) = pupil.aperture.bounds();
                if !pose.translation_mm.finite()
                    || !pose.rotation_deg.finite()
                    || !positive(hx)
                    || !positive(hy)
                    || source.diode.is_some()
                    || source.disk_radius_mm != 0.
                    || source.cone_half_angle_deg != 0.
                    || distance >= -1e-9
                    || !pupil.finite_object
                        && frame.local_vector(source.direction.unit()).0[2] <= 1e-9
                {
                    return Err(crate::Error::InvalidScene("瞳照明が不正です。発射面は瞳の前方に置き、光線は前向きに指定してください".into()));
                }
            }
            if let Some(spectrum) = &source.spectrum {
                spectrum.validate()?;
            }
            for w in source.wavelengths() {
                if self
                    .materials
                    .iter()
                    .any(|m| m.index(w).is_none() || m.absorption_at(w).is_none())
                    || self
                        .surfaces
                        .iter()
                        .any(|s| s.coating.as_ref().is_some_and(|c| !c.covers(w)))
                {
                    return Err(crate::Error::InvalidScene(format!(
                        "光源の {} um が材料・吸収表・コーティングの有効波長範囲に含まれません",
                        w
                    )));
                }
            }
            if let Some(d) = &source.diode {
                d.validate()?;
            }
            if source.count == 0
                || !source.origin_mm.finite()
                || !source.direction.finite()
                || source.direction.norm() < 1e-15
                || !source.disk_radius_mm.is_finite()
                || source.disk_radius_mm < 0.0
                || !source.cone_half_angle_deg.is_finite()
                || !(0.0..90.0).contains(&source.cone_half_angle_deg)
                || !positive(source.wavelength_um)
                || !positive(source.total_weight)
                || source.medium >= self.materials.len()
            {
                return Err(crate::Error::InvalidScene("光源の設定が不正です".into()));
            }
                Ok(())
            })().map_err(|error| crate::Error::InvalidSource {
                source_index: source_index + 1,
                detail: error.to_string(),
            })?;
        }
        let prepared = PreparedScene {
            spec: self,
            surfaces,
            detector_surfaces,
        };
        for (source_index, source) in prepared.spec.sources.iter().enumerate() {
            crate::illumination::PreparedPupil::new(&prepared, source).map_err(|error| {
                crate::Error::InvalidSource {
                    source_index: source_index + 1,
                    detail: error.to_string(),
                }
            })?;
        }
        Ok(prepared)
    }
}
