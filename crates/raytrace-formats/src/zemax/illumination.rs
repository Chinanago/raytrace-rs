//! Explicit subset of native Sequential object / field / pupil illumination.
use super::*;
use crate::Result;
use raytrace_core::illumination::{OpticalPupil, RayAiming, Vignetting};
use raytrace_core::model::PupilIllumination;

pub(super) fn sources(
    all: &[String],
    blocks: &[(usize, Vec<String>)],
    scene: &Scene,
    bindings: &[usize],
    base: Source,
) -> Result<Vec<Source>> {
    let ft = field(all, "FTYP").map(words).unwrap_or_default();
    let kind = ft.first().map(|s| number(s)).transpose()?.unwrap_or(0.);
    let count = ft.get(2).map(|s| number(s)).transpose()?.unwrap_or(1.);
    if count.fract() != 0. || !(1. ..=64.).contains(&count) {
        return Err(crate::Error::zmx("視野数は1〜64点に対応しています".into()));
    }
    let count = count as usize;
    let values = |key, default| -> Result<Vec<f64>> {
        match field(all, key) {
            None => Ok(vec![default; count]),
            Some(s) => {
                let a = words(s)
                    .iter()
                    .map(|v| number(v))
                    .collect::<Result<Vec<_>, _>>()?;
                if a.len() < count || a.iter().any(|v| !v.is_finite()) {
                    return Err(crate::Error::zmx(format!("{key}の視野データが不正です")));
                }
                Ok(a[..count].to_vec())
            }
        }
    };
    let xs = values("XFLN", 0.)?;
    let ys = values("YFLN", 0.)?;
    let weights = values("FWGN", 1.)?;
    let total = weights.iter().sum::<f64>();
    if !total.is_finite() || total <= 0. || weights.iter().any(|w| *w < 0.) {
        return Err(crate::Error::zmx(
            "FWGNには非負の重みと正の合計が必要です".into(),
        ));
    }
    if kind != 0. && kind != 1. {
        return Err(crate::Error::zmx(
            "視野型はAngleとObject Heightに対応しています".into(),
        ));
    }
    let object = blocks
        .first()
        .ok_or_else(|| crate::Error::zmx("物体面がありません".into()))?;
    if object.0 != 0
        || field(&object.1, "TYPE")
            .unwrap_or("STANDARD")
            .split_whitespace()
            .next()
            != Some("STANDARD")
    {
        return Err(crate::Error::zmx(
            "物体面はSTANDARDのSURF 0に対応しています".into(),
        ));
    }
    let distance = scalar(&object.1, "DISZ", f64::INFINITY)?;
    if field(&object.1, "GLAS").is_some_and(|s| !s.trim().is_empty()) {
        return Err(crate::Error::zmx(
            "物体側に材料を指定した照射は未対応です".into(),
        ));
    }
    let finite = distance.is_finite();
    if finite && distance <= 0. || !finite && distance != f64::INFINITY {
        return Err(crate::Error::zmx(
            "物体距離には正の距離またはINFINITYを指定してください".into(),
        ));
    }
    if kind == 1. && !finite {
        return Err(crate::Error::zmx(
            "Object Heightは有限距離の物体に対応しています".into(),
        ));
    }
    if kind == 0. && xs.iter().chain(&ys).any(|v| v.abs() >= 90.) {
        return Err(crate::Error::zmx(
            "Angle視野は各軸±90度未満に対応しています".into(),
        ));
    }
    let vdx = values("VDXN", 0.)?;
    let vdy = values("VDYN", 0.)?;
    let vcx = values("VCXN", 0.)?;
    let vcy = values("VCYN", 0.)?;
    let tan = values("VANN", 0.)?;
    let aiming = match scalar(all, "RAIM", 0.)? {
        0. => RayAiming::Off,
        1. => RayAiming::Paraxial,
        2. => RayAiming::Real,
        _ => {
            return Err(crate::Error::zmx(
                "RAIMは0（Off）・1（Paraxial）・2（Real）に対応しています".into(),
            ));
        }
    };
    let stop = blocks
        .iter()
        .find(|(_, l)| l.iter().any(|v| v.trim() == "STOP"))
        .map(|b| b.0);
    let needs_pupil = finite
        || stop.is_some()
        || aiming != RayAiming::Off
        || xs
            .iter()
            .chain(&ys)
            .chain(&vdx)
            .chain(&vdy)
            .chain(&vcx)
            .chain(&vcy)
            .chain(&tan)
            .any(|v| *v != 0.);
    let mut pupil = None;
    if needs_pupil {
        let diameter = scalar(all, "ENPD", 2. * base.disk_radius_mm)?;
        if !diameter.is_finite() || diameter <= 0. {
            return Err(crate::Error::zmx(
                "視野照射にはENPDによる正の入射瞳径が必要です".into(),
            ));
        }
        let stop = stop.ok_or_else(|| crate::Error::zmx("視野照射にはSTOP面が必要です".into()))?;
        let i = bindings
            .iter()
            .position(|b| *b == stop)
            .ok_or_else(|| crate::Error::zmx("STOP面の座標を解決できません".into()))?;
        let pose = &scene.surfaces[i].pose;
        let first = &scene.surfaces[0].pose;
        if first.rotation_deg.norm() > 1e-10
            || first.translation_mm.0[0].hypot(first.translation_mm.0[1]) > 1e-9
        {
            return Err(crate::Error::zmx("入射瞳を計算する元ZMXは、軸上・無回転の最初の光学面に対応しています。部品としての一括移動・回転は可能です".into()));
        }
        pupil = Some(PupilIllumination {
            pose: pose.clone(),
            aperture: Aperture::Circle {
                radius_mm: diameter / 2.,
            },
            finite_object: finite,
            surface: Some(i),
            optical: Some(OpticalPupil {
                first_surface: 0,
                primary_wavelength_um: base.wavelength_um,
                aiming,
                finite_field_angle_deg: None,
            }),
            vignetting: Vignetting::default(),
        });
    }
    let mut result = Vec::new();
    for i in 0..count {
        if weights[i] == 0. {
            continue;
        }
        let mut s = base.clone();
        s.total_weight = weights[i] / total;
        s.seed = s.seed.wrapping_add(i as u64 * 0x9e3779b9);
        if let Some(p) = &pupil {
            let z = p.pose.translation_mm.0[2];
            let dir = if kind == 0. {
                V3([xs[i].to_radians().tan(), ys[i].to_radians().tan(), 1.]).unit()
            } else {
                V3([0., 0., 1.])
            };
            s.origin_mm = if finite {
                if kind == 1. {
                    V3([xs[i], ys[i], -distance])
                } else {
                    V3([
                        -dir.0[0] / dir.0[2] * (distance + z),
                        -dir.0[1] / dir.0[2] * (distance + z),
                        -distance,
                    ])
                }
            } else {
                V3([0., 0., -1.])
            };
            s.direction = if finite {
                (p.pose.translation_mm - s.origin_mm).unit()
            } else {
                dir
            };
            s.disk_radius_mm = 0.;
            let mut p = p.clone();
            p.vignetting = Vignetting {
                decenter: [vdx[i], vdy[i]],
                compression: [vcx[i], vcy[i]],
                tangential_angle_deg: tan[i],
            };
            p.vignetting.validate()?;
            if finite && kind == 0. {
                p.optical.as_mut().unwrap().finite_field_angle_deg = Some([xs[i], ys[i]]);
            }
            s.pupil = Some(p);
        }
        result.push(s);
    }
    if pupil.is_some() {
        let mut check = scene.clone();
        check.sources = result.clone();
        let prepared = check.prepare()?;
        for (i, s) in result.iter_mut().enumerate() {
            if s.pupil
                .as_ref()
                .and_then(|p| p.optical.as_ref())
                .is_some_and(|p| p.finite_field_angle_deg.is_some())
                && let Some(g) = raytrace_core::illumination::pupil_geometry(&prepared, i)?
            {
                s.origin_mm = g.object_origin_mm;
            }
        }
    }
    Ok(result)
}
