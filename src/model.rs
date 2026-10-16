use crate::math::{Frame, Pose, V3};
pub use crate::sampling::Sampling;
use crate::sampling::{gaussian_quantile, uniform, Sobol};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum Material {
    Catalog {
        formula: usize,
        coefficients: Vec<f64>,
        valid_range_um: [f64; 2],
        #[serde(default)]
        absorption_per_mm: f64,
    },
    WithAbsorption {
        dispersion: Box<Material>,
        samples: Vec<[f64; 2]>,
    },
    Constant {
        n: f64,
        #[serde(default)]
        absorption_per_mm: f64,
    },
    /// n² = 1 + sum(B_i * lambda² / (lambda² - C_i)). C_i has units um².
    Sellmeier {
        b: [f64; 3],
        c_um2: [f64; 3],
        valid_range_um: [f64; 2],
        #[serde(default)]
        absorption_per_mm: f64,
    },
    /// n² = A0 + A1*l² + A2/l² + A3/l⁴ + A4/l⁶ + A5/l⁸ (AGF formula 1).
    Schott {
        coefficients: [f64; 6],
        valid_range_um: [f64; 2],
        #[serde(default)]
        absorption_per_mm: f64,
    },
    /// Sorted [wavelength_um, n] pairs. Linear interpolation, no extrapolation.
    Tabulated {
        samples: Vec<[f64; 2]>,
        #[serde(default)]
        absorption_per_mm: f64,
    },
}

impl Material {
    pub fn index(&self, wavelength_um: f64) -> Option<f64> {
        if !wavelength_um.is_finite() || wavelength_um <= 0.0 {
            return None;
        }
        let n = match self {
            Self::Catalog {
                formula,
                coefficients,
                valid_range_um,
                ..
            } => {
                if !(valid_range_um[0]..=valid_range_um[1]).contains(&wavelength_um) {
                    return None;
                }
                crate::dispersion::index(*formula, coefficients, wavelength_um)?
            }
            Self::WithAbsorption { dispersion, .. } => dispersion.index(wavelength_um)?,
            Self::Constant { n, .. } => *n,
            Self::Sellmeier {
                b,
                c_um2,
                valid_range_um,
                ..
            } => {
                if !(valid_range_um[0]..=valid_range_um[1]).contains(&wavelength_um) {
                    return None;
                }
                let l2 = wavelength_um * wavelength_um;
                (1.0 + (0..3).map(|i| b[i] * l2 / (l2 - c_um2[i])).sum::<f64>()).sqrt()
            }
            Self::Schott {
                coefficients: a,
                valid_range_um,
                ..
            } => {
                if !(valid_range_um[0]..=valid_range_um[1]).contains(&wavelength_um) {
                    return None;
                }
                let l2 = wavelength_um * wavelength_um;
                (a[0]
                    + a[1] * l2
                    + a[2] / l2
                    + a[3] / l2.powi(2)
                    + a[4] / l2.powi(3)
                    + a[5] / l2.powi(4))
                .sqrt()
            }
            Self::Tabulated { samples, .. } => {
                if samples.len() < 2
                    || wavelength_um < samples.first()?[0]
                    || wavelength_um > samples.last()?[0]
                {
                    return None;
                }
                let i = samples.partition_point(|s| s[0] < wavelength_um);
                if i == 0 {
                    samples[0][1]
                } else {
                    let [x0, y0] = samples[i - 1];
                    let [x1, y1] = samples[i];
                    y0 + (y1 - y0) * (wavelength_um - x0) / (x1 - x0)
                }
            }
        };
        (n.is_finite() && n > 0.0).then_some(n)
    }
    pub fn absorption(&self) -> f64 {
        match self {
            Self::WithAbsorption { .. } => 0.0,
            Self::Constant {
                absorption_per_mm, ..
            }
            | Self::Catalog {
                absorption_per_mm, ..
            }
            | Self::Sellmeier {
                absorption_per_mm, ..
            }
            | Self::Schott {
                absorption_per_mm, ..
            }
            | Self::Tabulated {
                absorption_per_mm, ..
            } => *absorption_per_mm,
        }
    }
    pub fn absorption_at(&self, wavelength_um: f64) -> Option<f64> {
        if let Self::WithAbsorption { samples, .. } = self {
            if samples.is_empty()
                || wavelength_um < samples.first()?[0]
                || wavelength_um > samples.last()?[0]
            {
                return None;
            }
            let i = samples.partition_point(|v| v[0] < wavelength_um);
            if i == 0 {
                return Some(samples[0][1]);
            }
            let [x0, y0] = samples[i - 1];
            let [x1, y1] = samples[i];
            Some(y0 + (y1 - y0) * (wavelength_um - x0) / (x1 - x0))
        } else {
            Some(self.absorption())
        }
    }
}

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
pub struct Source {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pupil: Option<PupilIllumination>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub spectrum: Option<crate::spectrum::Spectrum>,
    #[serde(default)]
    pub sampling: Sampling,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub diode: Option<Diode>,
    pub count: usize,
    pub origin_mm: V3,
    pub direction: V3,
    #[serde(default)]
    pub disk_radius_mm: f64,
    #[serde(default)]
    pub cone_half_angle_deg: f64,
    pub wavelength_um: f64,
    pub medium: usize,
    /// Arbitrary power units; the sum at launch defines the reference plane.
    pub total_weight: f64,
    #[serde(default)]
    pub seed: u64,
}
/// Uniform area samples on a pupil. Finite objects emit from a point; infinite
/// objects launch a parallel bundle whose intersections fill the pupil.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PupilIllumination {
    pub pose: Pose,
    pub aperture: Aperture,
    pub finite_object: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub surface: Option<usize>,
}
impl Source {
    pub fn wavelengths(&self) -> Vec<f64> {
        self.spectrum.as_ref().map_or_else(
            || vec![self.wavelength_um],
            |s| {
                s.samples
                    .iter()
                    .filter(|v| v[1] > 0.)
                    .map(|v| v[0])
                    .collect()
            },
        )
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Diode {
    pub rotation_deg: V3,
    pub divergence_deg: [f64; 2],
    pub array: [usize; 2],
    pub spacing_mm: [f64; 2],
    pub half_width_mm: [f64; 2],
    pub gaussian_width_mm: [f64; 2],
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

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub struct Ray {
    pub id: u64,
    pub position_mm: V3,
    pub direction: V3,
    pub weight: f64,
    pub wavelength_um: f64,
    pub medium: usize,
    #[serde(default)]
    pub optical_path_mm: f64,
}

impl Ray {
    fn validate_geometry(&mut self, scene: &PreparedScene) -> Result<(), String> {
        if !self.position_mm.finite()
            || !self.direction.finite()
            || self.direction.norm() < 1e-15
            || !self.weight.is_finite()
            || self.weight < 0.0
            || !self.wavelength_um.is_finite()
            || self.wavelength_um <= 0.0
            || self.medium >= scene.spec.materials.len()
            || !self.optical_path_mm.is_finite()
            || self.optical_path_mm < 0.0
        {
            return Err(format!("Invalid launch ray {}", self.id));
        }
        self.direction = self.direction.unit();
        Ok(())
    }
    pub fn validate(&mut self, scene: &PreparedScene) -> Result<(), String> {
        self.validate_geometry(scene)?;
        for (i, material) in scene.spec.materials.iter().enumerate() {
            if material.index(self.wavelength_um).is_none()
                || material.absorption_at(self.wavelength_um).is_none()
            {
                return Err(format!("Material {i} is undefined at {} um for launch {}; check the valid wavelength range", self.wavelength_um, self.id));
            }
        }
        for surface in &scene.surfaces {
            if surface
                .spec
                .coating
                .as_ref()
                .is_some_and(|c| !c.covers(self.wavelength_um))
            {
                return Err(format!(
                    "{}: コーティングの有効波長範囲に {} um が含まれません",
                    surface.spec.name, self.wavelength_um
                ));
            }
        }
        Ok(())
    }
}

fn positive(v: f64) -> bool {
    v.is_finite() && v > 0.0
}
fn fraction(v: f64) -> bool {
    v.is_finite() && (0.0..=1.0).contains(&v)
}
impl Scene {
    pub fn prepare(self) -> Result<PreparedScene, String> {
        if self.materials.is_empty() || self.surfaces.is_empty() {
            return Err("At least one material and surface are required".into());
        }
        let s = &self.settings;
        if s.max_depth == 0
            || s.max_events_per_launch == 0
            || !fraction(s.min_relative_branch_weight)
        {
            return Err("Invalid trace limits".into());
        }
        for m in &self.materials {
            if !m.absorption().is_finite() || m.absorption() < 0.0 {
                return Err("Absorption must be finite and nonnegative".into());
            }
            match m {
                Material::Catalog {
                    formula,
                    coefficients,
                    valid_range_um,
                    ..
                } => {
                    if crate::dispersion::coefficient_count(*formula)
                        .is_none_or(|n| coefficients.len() < n)
                        || !coefficients.iter().all(|v| v.is_finite())
                        || !positive(valid_range_um[0])
                        || !positive(valid_range_um[1])
                        || valid_range_um[0] >= valid_range_um[1]
                    {
                        return Err("Invalid AGF dispersion coefficients/range".into());
                    }
                }
                Material::WithAbsorption {
                    dispersion,
                    samples,
                } => {
                    if samples.is_empty()
                        || !samples
                            .iter()
                            .all(|s| positive(s[0]) && s[1].is_finite() && s[1] >= 0.0)
                        || !samples.windows(2).all(|w| w[0][0] < w[1][0])
                        || !samples.iter().any(|s| dispersion.index(s[0]).is_some())
                    {
                        return Err("Invalid catalog absorption/dispersion data".into());
                    }
                }
                Material::Constant { n, .. } if !positive(*n) => return Err("Invalid index".into()),
                Material::Sellmeier {
                    b,
                    c_um2,
                    valid_range_um,
                    ..
                } => {
                    if !b.iter().chain(c_um2).all(|v| v.is_finite())
                        || !positive(valid_range_um[0])
                        || !positive(valid_range_um[1])
                        || valid_range_um[0] >= valid_range_um[1]
                    {
                        return Err("Invalid Sellmeier data/range".into());
                    }
                }
                Material::Schott {
                    coefficients,
                    valid_range_um,
                    ..
                } => {
                    if !coefficients.iter().all(|v| v.is_finite())
                        || !positive(valid_range_um[0])
                        || !positive(valid_range_um[1])
                        || valid_range_um[0] >= valid_range_um[1]
                    {
                        return Err("Invalid Schott data/range".into());
                    }
                }
                Material::Tabulated { samples, .. }
                    if samples.len() < 2
                        || !samples.iter().all(|s| positive(s[0]) && positive(s[1]))
                        || !samples.windows(2).all(|s| s[0][0] < s[1][0]) =>
                {
                    return Err("Invalid index table".into());
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
                        return Err(format!(
                            "{}: コーティングには屈折界面、または透過率0のミラーを指定してください",
                            surface.name
                        ))
                    }
                }
            }
            if !names.insert(surface.name.clone()) {
                return Err("Surface names must be unique".into());
            }
            if !surface.pose.translation_mm.finite() || !surface.pose.rotation_deg.finite() {
                return Err(format!("Invalid pose: {}", surface.name));
            }
            let (hx, hy) = surface.aperture.bounds();
            if !positive(hx) || !positive(hy) {
                return Err("Invalid aperture".into());
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
                    return Err("Invalid lens barrel".into());
                }
                Shape::TriangleMesh {
                    vertices,
                    triangles,
                    normals,
                } => {
                    if vertices.is_empty()
                        || triangles.is_empty()
                        || triangles.len() > 1_000_000
                        || normals.len() != vertices.len()
                        || !vertices.iter().all(|p| p.finite())
                        || !normals.iter().all(|n| n.finite() && n.norm() > 1e-12)
                        || triangles
                            .iter()
                            .flatten()
                            .any(|&i| i as usize >= vertices.len())
                    {
                        return Err("Invalid triangle mesh".into());
                    }
                }
                Shape::Sphere { radius_mm }
                    if !positive(radius_mm.abs())
                        || surface.aperture.max_radius2() >= radius_mm * radius_mm =>
                {
                    return Err("Spherical sag cap requires aperture radius < |R|".into())
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
                    return Err("Invalid or singular even-asphere over aperture".into());
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
                        return Err("Invalid dielectric".into());
                    }
                    None
                }
                Interaction::Mirror { reflectance } if !fraction(reflectance) => {
                    return Err("Invalid mirror reflectance".into())
                }
                Interaction::Detector { pixels } => {
                    if !matches!(surface.shape, Shape::Plane) || pixels.contains(&0) {
                        return Err("Detector must be planar with nonzero pixel counts".into());
                    }
                    total_pixels = total_pixels
                        .checked_add(
                            pixels[0]
                                .checked_mul(pixels[1])
                                .ok_or("Pixel count overflow")?,
                        )
                        .ok_or("Pixel count overflow")?;
                    if total_pixels > 1_000_000 {
                        return Err("Prototype limit: 1 million detector pixels".into());
                    }
                    detector_surfaces.push(i);
                    Some(detector_surfaces.len() - 1)
                }
                _ => None,
            };
            let (local_bounds, max_sag_slope) =
                crate::geometry::bounds(&surface.shape, &surface.aperture)
                    .ok_or("Surface bounds overflow")?;
            surfaces.push(PreparedSurface {
                mesh: crate::mesh::MeshAccel::from_shape(&surface.shape),
                spec: surface.clone(),
                frame: Frame::new(&surface.pose),
                detector_index,
                local_bounds,
                max_sag_slope,
            });
        }
        for source in &self.sources {
            if let Some(pupil) = &source.pupil {
                let pose = if let Some(i) = pupil.surface {
                    &self.surfaces.get(i).ok_or("Invalid pupil surface")?.pose
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
                    return Err("Invalid pupil illumination: use a launch plane before the pupil and a forward direction".into());
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
                    return Err(format!(
                        "光源の {} um が材料・吸収表・コーティングの有効波長範囲に含まれません",
                        w
                    ));
                }
            }
            if let Some(d) = &source.diode {
                if !d.rotation_deg.finite()
                    || d.array.contains(&0)
                    || d.array.iter().any(|n| *n > 10000)
                    || d.divergence_deg
                        .iter()
                        .any(|v| !v.is_finite() || *v < 0.0 || *v >= 180.0)
                    || d.half_width_mm
                        .iter()
                        .chain(&d.gaussian_width_mm)
                        .any(|v| !v.is_finite() || *v < 0.0)
                    || !d.spacing_mm.iter().all(|v| v.is_finite())
                {
                    return Err("Invalid Source Diode parameters".into());
                }
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
                return Err("Invalid source".into());
            }
        }
        Ok(PreparedScene {
            spec: self,
            surfaces,
            detector_surfaces,
        })
    }
}

impl PreparedScene {
    pub fn generate_rays(&self) -> Result<Vec<Ray>, String> {
        let mut rays = Vec::new();
        for source in &self.spec.sources {
            if source.count as u128 > (1u128 << 52) {
                return Err("Sobol supports at most 2^52 rays per source".into());
            }
            let mut sobol = (source.sampling == Sampling::Sobol).then(|| Sobol::new(source.seed));
            let spectral_power = source
                .spectrum
                .as_ref()
                .map_or(0., |s| s.samples.iter().map(|v| v[1]).sum());
            let w = source.direction.unit();
            let helper = if w.0[2].abs() < 0.9 {
                V3([0.0, 0.0, 1.0])
            } else {
                V3([0.0, 1.0, 0.0])
            };
            let u = (helper - w * helper.dot(w)).unit();
            let v = V3([
                w.0[1] * u.0[2] - w.0[2] * u.0[1],
                w.0[2] * u.0[0] - w.0[0] * u.0[2],
                w.0[0] * u.0[1] - w.0[1] * u.0[0],
            ]);
            let pupil_frame = source
                .pupil
                .as_ref()
                .map(|p| Frame::new(p.surface.map_or(&p.pose, |i| &self.spec.surfaces[i].pose)));
            for j in 0..source.count {
                let mut state = source
                    .seed
                    .wrapping_add((j as u64).wrapping_mul(0xd1342543de82ef95));
                let sample = match &mut sobol {
                    Some(s) => s.next(),
                    None => [
                        uniform(&mut state),
                        uniform(&mut state),
                        uniform(&mut state),
                        uniform(&mut state),
                        0.0,
                        0.0,
                        0.0,
                    ],
                };
                let radius = source.disk_radius_mm * sample[0].sqrt();
                let phi = std::f64::consts::TAU * sample[1];
                let cos_theta =
                    1.0 - sample[2] * (1.0 - source.cone_half_angle_deg.to_radians().cos());
                let sin_theta = (1.0 - cos_theta * cos_theta).max(0.0).sqrt();
                let angle = std::f64::consts::TAU * sample[3];
                let mut ray = Ray {
                    id: rays.len() as u64,
                    position_mm: source.origin_mm
                        + u * (radius * phi.cos())
                        + v * (radius * phi.sin()),
                    direction: w * cos_theta
                        + u * (sin_theta * angle.cos())
                        + v * (sin_theta * angle.sin()),
                    weight: source.total_weight / source.count as f64,
                    wavelength_um: source.wavelength_um,
                    medium: source.medium,
                    optical_path_mm: 0.0,
                };
                if let Some((p, frame)) = source.pupil.as_ref().zip(pupil_frame.as_ref()) {
                    let xy = match p.aperture {
                        Aperture::Circle { radius_mm } => {
                            let r = radius_mm * sample[0].sqrt();
                            [r * phi.cos(), r * phi.sin()]
                        }
                        Aperture::Rectangle {
                            half_width_mm,
                            half_height_mm,
                        } => [
                            (2. * sample[0] - 1.) * half_width_mm,
                            (2. * sample[1] - 1.) * half_height_mm,
                        ],
                    };
                    let target = frame.world_point(V3([xy[0], xy[1], 0.]));
                    if p.finite_object {
                        ray.position_mm = source.origin_mm;
                        ray.direction = (target - source.origin_mm).unit();
                    } else {
                        let n = frame.world_vector(V3([0., 0., 1.]));
                        ray.position_mm =
                            target - w * ((target - source.origin_mm).dot(n) / w.dot(n));
                        ray.direction = w;
                    }
                }
                if let Some(diode) = &source.diode {
                    let frame = Frame::new(&Pose {
                        translation_mm: source.origin_mm,
                        rotation_deg: diode.rotation_deg,
                    });
                    let mut xy = [0.0; 2];
                    let mut angle = [0.0; 2];
                    for axis in 0..2 {
                        let index = ((if sobol.is_some() {
                            sample[4 + axis]
                        } else {
                            uniform(&mut state)
                        }) * diode.array[axis] as f64) as usize;
                        xy[axis] = (index as f64 - (diode.array[axis] - 1) as f64 / 2.0)
                            * diode.spacing_mm[axis];
                        if sobol.is_some() {
                            xy[axis] += gaussian_quantile(
                                sample[axis],
                                diode.gaussian_width_mm[axis] / 2.0,
                                diode.half_width_mm[axis],
                            );
                            angle[axis] = gaussian_quantile(
                                sample[2 + axis],
                                diode.divergence_deg[axis] / 2.0,
                                89.999999,
                            )
                            .to_radians();
                        } else {
                            xy[axis] += truncated_normal(
                                &mut state,
                                diode.gaussian_width_mm[axis] / 2.0,
                                diode.half_width_mm[axis],
                            );
                            angle[axis] = truncated_normal(
                                &mut state,
                                diode.divergence_deg[axis] / 2.0,
                                89.999999,
                            )
                            .to_radians();
                        }
                    }
                    ray.position_mm = frame.world_point(V3([xy[0], xy[1], 0.0]));
                    ray.direction = frame
                        .world_vector(V3([angle[0].tan(), angle[1].tan(), 1.0]).unit())
                        .unit();
                }
                if let Some(spectrum) = &source.spectrum {
                    ray.wavelength_um = spectrum.sample_with_total(
                        if sobol.is_some() {
                            sample[6]
                        } else {
                            uniform(&mut state)
                        },
                        spectral_power,
                    );
                }
                // prepare() has already checked every generated wavelength against
                // every material/coating; retain the geometric check and normalization.
                ray.validate_geometry(self)?;
                rays.push(ray);
            }
        }
        Ok(rays)
    }
}

fn truncated_normal(state: &mut u64, sigma: f64, half_width: f64) -> f64 {
    if sigma == 0.0 || half_width == 0.0 {
        return 0.0;
    }
    // Uniform rejection is efficient even when the physical window is narrow.
    if half_width < sigma {
        loop {
            let v = (2.0 * uniform(state) - 1.0) * half_width;
            if uniform(state) < (-0.5 * (v / sigma).powi(2)).exp() {
                return v;
            }
        }
    }
    loop {
        let a = uniform(state).max(1e-300);
        let b = uniform(state);
        let v = sigma * (-2.0 * a.ln()).sqrt() * (std::f64::consts::TAU * b).cos();
        if v.abs() <= half_width {
            return v;
        }
    }
}
