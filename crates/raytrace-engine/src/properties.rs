use crate::{Error, PropertyChange as P, Result};
use raytrace_core::{
    math::V3,
    model::{Aperture, Interaction, Source, Surface},
};
use raytrace_io::project::Project;
use serde_json::{Value, json};

#[derive(Clone, Copy)]
pub(crate) enum Target {
    Project,
    Component(usize),
    Source(Option<usize>, usize),
    Surface(Option<usize>, usize),
}

pub(crate) fn locate(p: &Project, id: &str) -> Result<Target> {
    if id == "project" {
        return Ok(Target::Project);
    }
    let (local, component) = if let Some((parent, local)) = id.split_once('/') {
        let i = p
            .assembly
            .as_ref()
            .and_then(|a| a.components.iter().position(|c| c.id.0 == parent))
            .ok_or_else(|| Error::new("not_found", format!("部品 {parent} がありません")))?;
        (local, Some(i))
    } else {
        if let Some(i) = p
            .assembly
            .as_ref()
            .and_then(|a| a.components.iter().position(|c| c.id.0 == id))
        {
            return Ok(Target::Component(i));
        }
        (id, None)
    };
    let q = component.map_or(p, |i| &p.assembly.as_ref().unwrap().components[i].project);
    if component.is_none() && p.assembly.is_some() {
        return Err(Error::new("not_found", id));
    }
    if let Some(i) = q.identity.sources.iter().position(|x| x.0 == local) {
        return Ok(Target::Source(component, i));
    }
    if let Some(i) = q.identity.surfaces.iter().position(|x| x.0 == local) {
        return Ok(Target::Surface(component, i));
    }
    Err(Error::new(
        "not_found",
        format!("編集対象 {id} がありません"),
    ))
}
fn positive(v: f64) -> Result<()> {
    if v.is_finite() && v > 0. && v <= 1e12 {
        Ok(())
    } else {
        Err(Error::new(
            "invalid_value",
            "正の有限値を指定してください（上限1e12）",
        ))
    }
}
fn finite(values: &[f64]) -> Result<()> {
    if values.iter().all(|v| v.is_finite() && v.abs() <= 1e12) {
        Ok(())
    } else {
        Err(Error::new(
            "invalid_value",
            "有限値を指定してください（絶対値の上限1e12）",
        ))
    }
}
pub(crate) fn dimensions(values: &[f64]) -> Result<()> {
    for &v in values {
        positive(v)?;
    }
    Ok(())
}
pub(crate) fn pixels(values: [usize; 2]) -> Result<()> {
    if values.contains(&0)
        || values[0]
            .checked_mul(values[1])
            .is_none_or(|n| n > 1_000_000)
    {
        Err(Error::new(
            "invalid_value",
            "画素数は合計1〜1,000,000で指定してください",
        ))
    } else {
        Ok(())
    }
}

fn source_set(s: &mut Source, value: &P) -> Result<()> {
    match value {
        P::PositionMm(v) => {
            finite(v)?;
            s.origin_mm = V3(*v);
        }
        P::Direction(v) => {
            finite(v)?;
            if V3(*v).norm() < 1e-12 {
                return Err(Error::new(
                    "invalid_value",
                    "光線方向の長さを確認してください",
                ));
            }
            s.direction = V3(*v).unit();
        }
        P::Rays(n) if (1..=2_000_000).contains(n) => s.count = *n,
        P::Seed(n) => s.seed = *n,
        P::WavelengthUm(v) => {
            positive(*v)?;
            if s.spectrum.is_some() {
                return Err(Error::new(
                    "spectrum_active",
                    "スペクトルを持つ光源の単色計算にはproject.monochromatic_override_umを指定してください",
                ));
            }
            s.wavelength_um = *v;
        }
        P::TotalWeight(v) if *v >= 0. => {
            finite(&[*v])?;
            s.total_weight = *v;
        }
        P::DiskRadiusMm(v) if *v >= 0. => {
            finite(&[*v])?;
            s.disk_radius_mm = *v;
        }
        P::ConeHalfAngleDeg(v) if (0.0..90.0).contains(v) => s.cone_half_angle_deg = *v,
        P::Sampling(mode) => s.sampling = (*mode).into(),
        P::Spectrum(samples) => {
            let spectrum = samples
                .as_ref()
                .map(|v| raytrace_core::spectrum::Spectrum { samples: v.clone() });
            if let Some(spectrum) = &spectrum {
                spectrum.validate()?;
            }
            s.spectrum = spectrum;
        }
        P::DiodeDivergenceDeg(_)
        | P::DiodeArray(_)
        | P::DiodeSpacingMm(_)
        | P::DiodeHalfWidthMm(_)
        | P::DiodeGaussianWidthMm(_)
        | P::DiodeAngularSupergaussian(_)
        | P::DiodeSpatialSupergaussian(_)
        | P::DiodeAstigmatismMm(_)
        | P::DiodeRotationDeg(_) => {
            match value {
                P::DiodeDivergenceDeg(v)
                | P::DiodeSpacingMm(v)
                | P::DiodeHalfWidthMm(v)
                | P::DiodeGaussianWidthMm(v)
                | P::DiodeAngularSupergaussian(v)
                | P::DiodeSpatialSupergaussian(v) => finite(v)?,
                P::DiodeAstigmatismMm(v) => finite(&[*v])?,
                P::DiodeRotationDeg(v) => finite(v)?,
                _ => (),
            }
            let d = s.diode.as_mut().ok_or_else(|| {
                Error::new("invalid_property", "Source DiodeのIDを指定してください")
            })?;
            match value {
                P::DiodeDivergenceDeg(v) => d.divergence_deg = *v,
                P::DiodeArray(v) => d.array = *v,
                P::DiodeSpacingMm(v) => d.spacing_mm = *v,
                P::DiodeHalfWidthMm(v) => d.half_width_mm = *v,
                P::DiodeGaussianWidthMm(v) => d.gaussian_width_mm = *v,
                P::DiodeAngularSupergaussian(v) => d.angular_supergaussian = *v,
                P::DiodeSpatialSupergaussian(v) => d.spatial_supergaussian = *v,
                P::DiodeAstigmatismMm(v) => d.astigmatism_mm = *v,
                P::DiodeRotationDeg(v) => d.rotation_deg = V3(*v),
                _ => unreachable!(),
            }
            d.validate()?;
        }
        _ => {
            return Err(Error::new(
                "invalid_property",
                "この光源の項目・値を確認してください",
            ));
        }
    }
    Ok(())
}
fn surface_set(s: &mut Surface, value: &P) -> Result<()> {
    match value {
        P::Name(name) => s.name = name.clone(),
        P::PositionMm(v) => {
            finite(v)?;
            s.pose.translation_mm = V3(*v);
        }
        P::RotationDeg(v) => {
            finite(v)?;
            s.pose.rotation_deg = V3(*v);
        }
        P::DetectorSizeMm(v) if matches!(s.interaction, Interaction::Detector { .. }) => {
            dimensions(v)?;
            s.aperture = Aperture::Rectangle {
                half_width_mm: v[0] / 2.,
                half_height_mm: v[1] / 2.,
            };
        }
        P::Pixels(v) => {
            pixels(*v)?;
            let Interaction::Detector { pixels } = &mut s.interaction else {
                return Err(Error::new(
                    "invalid_property",
                    "検出器のIDを指定してください",
                ));
            };
            *pixels = *v;
        }
        P::RadiusMm(v) => {
            finite(&[*v])?;
            match &mut s.shape {
                raytrace_core::model::Shape::Sphere { radius_mm }
                | raytrace_core::model::Shape::EvenAsphere { radius_mm, .. } => *radius_mm = *v,
                _ => {
                    return Err(Error::new(
                        "invalid_property",
                        "球面・非球面のIDを指定してください",
                    ));
                }
            }
            validate_surface(s)?;
        }
        P::Conic(v) => {
            finite(&[*v])?;
            let raytrace_core::model::Shape::EvenAsphere { conic, .. } = &mut s.shape else {
                return Err(Error::new(
                    "invalid_property",
                    "非球面のIDを指定してください",
                ));
            };
            *conic = *v;
            validate_surface(s)?;
        }
        P::SurfaceMedia(v) => {
            let Interaction::Dielectric {
                minus_medium,
                plus_medium,
                ..
            } = &mut s.interaction
            else {
                return Err(Error::new(
                    "invalid_property",
                    "屈折面のIDを指定してください",
                ));
            };
            *minus_medium = v[0];
            *plus_medium = v[1];
        }
        _ => {
            return Err(Error::new(
                "invalid_property",
                "この面には指定した項目を適用できません",
            ));
        }
    }
    Ok(())
}
fn validate_surface(surface: &Surface) -> Result<()> {
    let mut scene = raytrace_io::assembly::empty_scene("geometry validation");
    let mut s = surface.clone();
    s.interaction = Interaction::Absorb;
    s.coating = None;
    scene.surfaces.push(s);
    scene.prepare()?;
    Ok(())
}
pub(crate) fn set(p: &mut Project, id: &str, value: &P) -> Result<()> {
    match locate(p, id)? {
        Target::Project => match value {
            P::Name(name) => p.scene.name = name.clone(),
            P::MonochromaticOverrideUm(v) => {
                if let Some(v) = v {
                    positive(*v)?;
                }
                p.monochromatic_override_um = *v;
            }
            P::AllowUncoated(v) => p.allow_uncoated = *v,
            P::MaxDepth(n) if *n > 0 && *n <= 1_000_000 => p.scene.settings.max_depth = *n,
            P::MaxEventsPerLaunch(n) if *n > 0 && *n <= 1_000_000 => {
                p.scene.settings.max_events_per_launch = *n
            }
            P::MinRelativeBranchWeight(v) if v.is_finite() && (0.0..=1.0).contains(v) => {
                p.scene.settings.min_relative_branch_weight = *v
            }
            _ => {
                return Err(Error::new(
                    "invalid_property",
                    "このプロジェクトの項目・値を確認してください",
                ));
            }
        },
        Target::Component(i) => {
            let c = &mut p.assembly.as_mut().unwrap().components[i];
            match value {
                P::Name(name) => c.name = name.clone(),
                P::Enabled(v) => c.enabled = *v,
                P::PositionMm(v) => {
                    finite(v)?;
                    c.pose.translation_mm = V3(*v);
                }
                P::RotationDeg(v) => {
                    finite(v)?;
                    c.pose.rotation_deg = V3(*v);
                }
                P::RodSizeMm(v) => {
                    dimensions(v)?;
                    let r = c.rod.as_mut().ok_or_else(|| {
                        Error::new("invalid_property", "ロッドのIDを指定してください")
                    })?;
                    r.width_mm = v[0];
                    r.height_mm = v[1];
                    r.length_mm = v[2];
                }
                P::DetectorSizeMm(_) | P::Pixels(_)
                    if matches!(c.kind, raytrace_io::assembly::ComponentKind::Detector) =>
                {
                    let s = c
                        .project
                        .scene
                        .surfaces
                        .first_mut()
                        .ok_or_else(|| Error::new("not_found", "検出面がありません"))?;
                    surface_set(s, value)?;
                }
                _ => {
                    return Err(Error::new(
                        "invalid_property",
                        "この部品には指定した項目を適用できません",
                    ));
                }
            }
        }
        Target::Source(component, i) => {
            let q = match component {
                Some(c) => &mut *p.assembly.as_mut().unwrap().components[c].project,
                None => p,
            };
            source_set(&mut q.scene.sources[i], value)?;
        }
        Target::Surface(component, i) => {
            if component.is_some_and(|c| p.assembly.as_ref().unwrap().components[c].rod.is_some()) {
                return Err(Error::new(
                    "read_only",
                    "ロッドの生成面は、部品の位置・傾き・寸法から編集してください",
                ));
            }
            let q = match component {
                Some(c) => &mut *p.assembly.as_mut().unwrap().components[c].project,
                None => p,
            };
            if let P::SurfaceMedia(v) = value
                && v.iter().any(|i| *i >= q.scene.materials.len())
            {
                return Err(Error::new("invalid_value", "材料番号を確認してください"));
            }
            surface_set(&mut q.scene.surfaces[i], value)?;
        }
    }
    Ok(())
}

pub(crate) fn describe(p: &Project, id: &str) -> Result<Value> {
    let target = locate(p, id)?;
    let pose =
        |position: V3, rotation: V3| vec![P::PositionMm(position.0), P::RotationDeg(rotation.0)];
    let surface = |s: &Surface| {
        let mut v = pose(s.pose.translation_mm, s.pose.rotation_deg);
        v.push(P::Name(s.name.clone()));
        match &s.shape {
            raytrace_core::model::Shape::Sphere { radius_mm }
            | raytrace_core::model::Shape::EvenAsphere { radius_mm, .. } => {
                v.push(P::RadiusMm(*radius_mm))
            }
            _ => (),
        }
        if let raytrace_core::model::Shape::EvenAsphere { conic, .. } = s.shape {
            v.push(P::Conic(conic));
        }
        if let Interaction::Dielectric {
            minus_medium,
            plus_medium,
            ..
        } = s.interaction
        {
            v.push(P::SurfaceMedia([minus_medium, plus_medium]));
        }
        if let Interaction::Detector { pixels } = s.interaction {
            let (x, y) = s.aperture.bounds();
            v.extend([P::Pixels(pixels), P::DetectorSizeMm([x * 2., y * 2.])]);
        }
        v
    };
    let values = match target {
        Target::Project => vec![
            P::Name(p.scene.name.clone()),
            P::MonochromaticOverrideUm(p.monochromatic_override_um),
            P::AllowUncoated(p.allow_uncoated),
            P::MaxDepth(p.scene.settings.max_depth),
            P::MaxEventsPerLaunch(p.scene.settings.max_events_per_launch),
            P::MinRelativeBranchWeight(p.scene.settings.min_relative_branch_weight),
        ],
        Target::Component(i) => {
            let c = &p.assembly.as_ref().unwrap().components[i];
            let mut v = pose(c.pose.translation_mm, c.pose.rotation_deg);
            v.extend([P::Name(c.name.clone()), P::Enabled(c.enabled)]);
            if let Some(r) = &c.rod {
                v.push(P::RodSizeMm([r.width_mm, r.height_mm, r.length_mm]));
            }
            if matches!(c.kind, raytrace_io::assembly::ComponentKind::Detector)
                && let Some(s) = c.project.scene.surfaces.first()
            {
                v.extend(
                    surface(s)
                        .into_iter()
                        .filter(|p| matches!(p, P::Pixels(_) | P::DetectorSizeMm(_))),
                );
            }
            v
        }
        Target::Source(component, i) => {
            let q = component.map_or(p, |c| &p.assembly.as_ref().unwrap().components[c].project);
            let s = &q.scene.sources[i];
            let mut v = vec![
                P::PositionMm(s.origin_mm.0),
                P::Direction(s.direction.0),
                P::Rays(s.count),
                P::Seed(s.seed),
                P::TotalWeight(s.total_weight),
                P::DiskRadiusMm(s.disk_radius_mm),
                P::ConeHalfAngleDeg(s.cone_half_angle_deg),
                P::Sampling(match s.sampling {
                    raytrace_core::model::Sampling::Sobol => crate::SamplingMode::Sobol,
                    raytrace_core::model::Sampling::Pseudorandom => {
                        crate::SamplingMode::Pseudorandom
                    }
                }),
            ];
            if s.spectrum.is_none() {
                v.push(P::WavelengthUm(s.wavelength_um));
            }
            v.push(P::Spectrum(s.spectrum.as_ref().map(|s| s.samples.clone())));
            if let Some(d) = &s.diode {
                v.extend([
                    P::DiodeDivergenceDeg(d.divergence_deg),
                    P::DiodeArray(d.array),
                    P::DiodeSpacingMm(d.spacing_mm),
                    P::DiodeHalfWidthMm(d.half_width_mm),
                    P::DiodeGaussianWidthMm(d.gaussian_width_mm),
                    P::DiodeAngularSupergaussian(d.angular_supergaussian),
                    P::DiodeSpatialSupergaussian(d.spatial_supergaussian),
                    P::DiodeAstigmatismMm(d.astigmatism_mm),
                    P::DiodeRotationDeg(d.rotation_deg.0),
                ]);
            }
            v
        }
        Target::Surface(component, i) => {
            if let Some(c) = component {
                let c = &p.assembly.as_ref().unwrap().components[c];
                if c.rod.is_some() {
                    return Ok(
                        json!({"target":id,"read_only":true,"reason":"generated_rod_surface"}),
                    );
                }
                surface(&c.project.scene.surfaces[i])
            } else {
                surface(&p.scene.surfaces[i])
            }
        }
    };
    let descriptors: Vec<_> = values
        .into_iter()
        .map(|v| {
            let (unit, range) = match &v {
                P::PositionMm(_) => ("mm", json!([-1e12, 1e12])),
                P::RotationDeg(_) | P::DiodeRotationDeg(_) => ("deg", json!([-1e12, 1e12])),
                P::RodSizeMm(_) | P::DetectorSizeMm(_) => {
                    ("mm", json!({"exclusive_minimum":0,"maximum":1e12}))
                }
                P::WavelengthUm(_) | P::MonochromaticOverrideUm(_) => {
                    ("um", json!({"exclusive_minimum":0,"maximum":1e12}))
                }
                P::DiskRadiusMm(_)
                | P::DiodeHalfWidthMm(_)
                | P::DiodeGaussianWidthMm(_)
                | P::DiodeAstigmatismMm(_) => ("mm", json!([0, 1e12])),
                P::RadiusMm(_) | P::DiodeSpacingMm(_) => ("mm", json!([-1e12, 1e12])),
                P::DiodeDivergenceDeg(_) => ("deg", json!({"minimum":0})),
                P::DiodeArray(_) => ("count", json!([1, 10000])),
                P::DiodeAngularSupergaussian(_) => ("", json!([0.01, 50])),
                P::DiodeSpatialSupergaussian(_) => ("", json!({"minimum":0.01})),
                P::TotalWeight(_) => ("relative_weight", json!([0, 1e12])),
                P::ConeHalfAngleDeg(_) => ("deg", json!({"minimum":0,"exclusive_maximum":90})),
                P::Rays(_) => ("rays", json!([1, 2_000_000])),
                P::Pixels(_) => ("pixels", json!({"minimum":1,"max_product":1_000_000})),
                _ => ("", Value::Null),
            };
            json!({"change":v,"unit":unit,"range":range,"coordinates":"parent_local"})
        })
        .collect();
    let material_scope = if matches!(target, Target::Component(_)) {
        id
    } else {
        id.split_once('/').map_or("project", |(parent, _)| parent)
    };
    Ok(json!({"target":id,"properties":descriptors,"material_scope":material_scope}))
}
