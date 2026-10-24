use super::{Aperture, PreparedScene, Sampling};
use crate::math::{Frame, Pose, V3};
use crate::sampling::{gaussian_quantile, uniform, Sobol, SuperGaussian};
use serde::{Deserialize, Serialize};
use std::ops::Range;

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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub optical: Option<crate::illumination::OpticalPupil>,
    #[serde(
        default,
        skip_serializing_if = "crate::illumination::Vignetting::is_zero"
    )]
    pub vignetting: crate::illumination::Vignetting,
}
impl Source {
    #[doc(hidden)]
    pub fn extended_illumination(&self) -> bool {
        self.diode.as_ref().is_some_and(|d| {
            d.astigmatism_mm != 0.
                || d.angular_supergaussian != [1.; 2]
                || d.spatial_supergaussian != [1.; 2]
        }) || self
            .pupil
            .as_ref()
            .is_some_and(|p| p.optical.is_some() || !p.vignetting.is_zero())
    }
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
    #[serde(default)]
    pub astigmatism_mm: f64,
    #[serde(default = "unit_pair")]
    pub angular_supergaussian: [f64; 2],
    #[serde(default = "unit_pair")]
    pub spatial_supergaussian: [f64; 2],
    pub rotation_deg: V3,
    pub divergence_deg: [f64; 2],
    pub array: [usize; 2],
    pub spacing_mm: [f64; 2],
    pub half_width_mm: [f64; 2],
    pub gaussian_width_mm: [f64; 2],
}
fn unit_pair() -> [f64; 2] {
    [1.; 2]
}
impl Diode {
    #[doc(hidden)]
    pub fn validate(&self) -> Result<(), String> {
        if !self.rotation_deg.finite()
            || !self.astigmatism_mm.is_finite()
            || self.astigmatism_mm < 0.
            || self
                .angular_supergaussian
                .iter()
                .any(|v| !v.is_finite() || !(0.01..=50.).contains(v))
            || self.array.iter().any(|n| *n == 0 || *n > 10000)
            || self
                .divergence_deg
                .iter()
                .any(|v| !v.is_finite() || *v < 0.)
            || self.astigmatism_mm == 0.
                && (self
                    .half_width_mm
                    .iter()
                    .chain(&self.gaussian_width_mm)
                    .any(|v| !v.is_finite() || *v < 0.)
                    || self
                        .spatial_supergaussian
                        .iter()
                        .any(|v| !v.is_finite() || *v < 0.01))
            || self.spacing_mm.iter().any(|v| !v.is_finite())
        {
            return Err("Source Diode: 非点収差は0以上、角度の超Gaussian係数は0.01〜50、発光面の係数は0.01以上、配列数は1〜10,000で指定してください".into());
        }
        Ok(())
    }
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

/// Prepared source data shared by independently generated launch ranges.
pub struct RayGenerator<'a> {
    scene: &'a PreparedScene,
    sources: Vec<Result<PreparedSource<'a>, String>>,
    offsets: Vec<usize>,
}

struct PreparedSource<'a> {
    source: &'a Source,
    spectral_power: f64,
    w: V3,
    u: V3,
    v: V3,
    pupil_frame: Option<Frame>,
    optical_pupil: Option<crate::illumination::PreparedPupil>,
    profiles: Option<[Option<SuperGaussian>; 4]>,
    diode_frame: Option<Frame>,
}

impl PreparedScene {
    pub fn ray_generator(&self) -> Result<RayGenerator<'_>, String> {
        let mut sources = Vec::new();
        let mut offsets = vec![0usize];
        for source in &self.spec.sources {
            // Defer source errors to their launch offset to retain error order.
            sources.push(PreparedSource::new(self, source));
            offsets.push(
                offsets
                    .last()
                    .unwrap()
                    .checked_add(source.count)
                    .ok_or("Source ray count exceeds usize")?,
            );
        }
        Ok(RayGenerator {
            scene: self,
            sources,
            offsets,
        })
    }

    pub fn generate_rays(&self) -> Result<Vec<Ray>, String> {
        let generator = self.ray_generator()?;
        generator.rays(0..generator.len()).collect()
    }
}

impl RayGenerator<'_> {
    pub fn len(&self) -> usize {
        *self.offsets.last().unwrap()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Source preparation errors are yielded in launch order along with ray errors.
    pub fn rays(&self, range: Range<usize>) -> impl Iterator<Item = Result<Ray, String>> + '_ {
        assert!(range.start <= range.end && range.end <= self.len());
        let first = self.offsets[1..].partition_point(|&end| end <= range.start);
        let last = self.offsets[..self.sources.len()].partition_point(|&start| start < range.end);
        (first..last).flat_map(move |source_index| {
            let source = &self.sources[source_index];
            let offset = self.offsets[source_index];
            let start = range.start.max(offset) - offset;
            let end = range.end.min(self.offsets[source_index + 1]) - offset;
            let mut sobol = source
                .as_ref()
                .ok()
                .filter(|s| s.source.sampling == Sampling::Sobol && start < end)
                .map(|source| {
                    let mut s = Sobol::new(source.source.seed);
                    s.seek(start as u64);
                    s
                });
            (start..end).map(move |j| match source {
                Ok(source) => source.ray(self.scene, source_index, j, offset + j, &mut sobol),
                Err(error) => Err(error.clone()),
            })
        })
    }
}

impl<'a> PreparedSource<'a> {
    fn new(scene: &PreparedScene, source: &'a Source) -> Result<Self, String> {
        if source.count as u128 > (1u128 << 52) {
            return Err("Sobol supports at most 2^52 rays per source".into());
        }
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
        let v = w.cross(u);
        let pupil_frame = source
            .pupil
            .as_ref()
            .map(|p| Frame::new(p.surface.map_or(&p.pose, |i| &scene.spec.surfaces[i].pose)));
        let optical_pupil = crate::illumination::PreparedPupil::new(scene, source)?;
        let profiles: Option<[Option<SuperGaussian>; 4]> = source.diode.as_ref().map(|d| {
            std::array::from_fn(|i| {
                let axis = i % 2;
                let (width, factor, limit) = if i < 2 {
                    (
                        d.gaussian_width_mm[axis],
                        d.spatial_supergaussian[axis],
                        d.half_width_mm[axis],
                    )
                } else {
                    (
                        d.divergence_deg[axis],
                        d.angular_supergaussian[axis],
                        89.999999,
                    )
                };
                (factor != 1. && !(i < 2 && d.astigmatism_mm > 0.))
                    .then(|| SuperGaussian::new(width, factor, limit))
            })
        });
        let diode_frame = source.diode.as_ref().map(|diode| {
            Frame::new(&Pose {
                translation_mm: source.origin_mm,
                rotation_deg: diode.rotation_deg,
            })
        });
        Ok(Self {
            source,
            spectral_power,
            w,
            u,
            v,
            pupil_frame,
            optical_pupil,
            profiles,
            diode_frame,
        })
    }

    fn ray(
        &self,
        scene: &PreparedScene,
        source_index: usize,
        j: usize,
        id: usize,
        sobol: &mut Option<Sobol>,
    ) -> Result<Ray, String> {
        let Self {
            source,
            spectral_power,
            w,
            u,
            v,
            pupil_frame,
            optical_pupil,
            profiles,
            ..
        } = self;
        let (w, u, v, spectral_power) = (*w, *u, *v, *spectral_power);
        let mut state = source
            .seed
            .wrapping_add((j as u64).wrapping_mul(0xd1342543de82ef95));
        let sample = match sobol {
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
        let cos_theta = 1.0 - sample[2] * (1.0 - source.cone_half_angle_deg.to_radians().cos());
        let sin_theta = (1.0 - cos_theta * cos_theta).max(0.0).sqrt();
        let angle = std::f64::consts::TAU * sample[3];
        let mut ray = Ray {
            id: id as u64,
            position_mm: source.origin_mm + u * (radius * phi.cos()) + v * (radius * phi.sin()),
            direction: w * cos_theta
                + u * (sin_theta * angle.cos())
                + v * (sin_theta * angle.sin()),
            weight: source.total_weight / source.count as f64,
            wavelength_um: source.wavelength_um,
            medium: source.medium,
            optical_path_mm: 0.0,
        };
        if let Some((p, frame)) = source.pupil.as_ref().zip(pupil_frame.as_ref()) {
            if p.optical.is_none() {
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
                let (hx, hy) = p.aperture.bounds();
                let xy = if p.vignetting.is_zero() {
                    xy
                } else {
                    let normalized = p.vignetting.transform([xy[0] / hx, xy[1] / hy]);
                    [normalized[0] * hx, normalized[1] * hy]
                };
                let target = frame.world_point(V3([xy[0], xy[1], 0.]));
                if p.finite_object {
                    ray.position_mm = source.origin_mm;
                    ray.direction = (target - source.origin_mm).unit();
                } else {
                    let n = frame.world_vector(V3([0., 0., 1.]));
                    ray.position_mm = target - w * ((target - source.origin_mm).dot(n) / w.dot(n));
                    ray.direction = w;
                }
            }
        }
        if let Some(diode) = &source.diode {
            let frame = self.diode_frame.as_ref().unwrap();
            let mut xy = [0.0; 2];
            let mut angle = [0.0; 2];
            for axis in 0..2 {
                let index = ((if sobol.is_some() {
                    sample[4 + axis]
                } else {
                    uniform(&mut state)
                }) * diode.array[axis] as f64) as usize;
                xy[axis] =
                    (index as f64 - (diode.array[axis] - 1) as f64 / 2.0) * diode.spacing_mm[axis];
                let spatial_profile = profiles.as_ref().and_then(|p| p[axis].as_ref());
                let angular_profile = profiles.as_ref().and_then(|p| p[2 + axis].as_ref());
                if sobol.is_some() {
                    if diode.astigmatism_mm == 0. {
                        xy[axis] += spatial_profile.map_or_else(
                            || {
                                gaussian_quantile(
                                    sample[axis],
                                    diode.gaussian_width_mm[axis] / 2.0,
                                    diode.half_width_mm[axis],
                                )
                            },
                            |p| p.quantile(sample[axis]),
                        );
                    }
                    angle[axis] = angular_profile
                        .map_or_else(
                            || {
                                gaussian_quantile(
                                    sample[2 + axis],
                                    diode.divergence_deg[axis] / 2.0,
                                    89.999999,
                                )
                            },
                            |p| p.quantile(sample[2 + axis]),
                        )
                        .to_radians();
                } else {
                    if diode.astigmatism_mm == 0. {
                        xy[axis] += match spatial_profile {
                            Some(p) => p.quantile(uniform(&mut state)),
                            None => truncated_normal(
                                &mut state,
                                diode.gaussian_width_mm[axis] / 2.0,
                                diode.half_width_mm[axis],
                            ),
                        };
                    }
                    angle[axis] = match angular_profile {
                        Some(p) => p.quantile(uniform(&mut state)),
                        None => truncated_normal(
                            &mut state,
                            diode.divergence_deg[axis] / 2.0,
                            89.999999,
                        ),
                    }
                    .to_radians();
                }
            }
            if diode.astigmatism_mm > 0. {
                xy[0] += diode.astigmatism_mm * angle[0].tan();
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
        if let Some(pupil) = &optical_pupil {
            let (position, direction) = pupil
                .launch(source, ray.wavelength_um, [sample[0], sample[1]])
                .map_err(|e| {
                    format!(
                        "光源 {} / 光線 {} / 波長 {} um: {e}",
                        source_index + 1,
                        ray.id,
                        ray.wavelength_um
                    )
                })?;
            ray.position_mm = position;
            ray.direction = direction;
        }
        // prepare() has already checked every generated wavelength against
        // every material/coating; retain the geometric check and normalization.
        ray.validate_geometry(scene)?;
        Ok(ray)
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
