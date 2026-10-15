//! Explicit subset of native Sequential object / field / pupil illumination.
use super::*;
use crate::model::PupilIllumination;

pub(super) fn sources(
    all: &[String],
    blocks: &[(usize, Vec<String>)],
    scene: &Scene,
    bindings: &[usize],
    base: Source,
) -> Result<Vec<Source>, String> {
    let ft = field(all, "FTYP").map(words).unwrap_or_default();
    let kind = ft.first().map(|s| number(s)).transpose()?.unwrap_or(0.);
    let count = ft.get(2).map(|s| number(s)).transpose()?.unwrap_or(1.);
    if count.fract() != 0. || !(1. ..=64.).contains(&count) {
        return Err("視野数は1〜64点に対応しています".into());
    }
    let count = count as usize;
    let values = |key, default| -> Result<Vec<f64>, String> {
        match field(all, key) {
            None => Ok(vec![default; count]),
            Some(s) => {
                let a = words(s)
                    .iter()
                    .map(|v| number(v))
                    .collect::<Result<Vec<_>, _>>()?;
                if a.len() < count || a.iter().any(|v| !v.is_finite()) {
                    return Err(format!("{key}の視野データが不正です"));
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
        return Err("FWGNには非負の重みと正の合計が必要です".into());
    }
    if kind != 0. && kind != 1. {
        return Err("視野型はAngleとObject Heightに対応しています".into());
    }
    let object = blocks.first().ok_or("物体面がありません")?;
    if object.0 != 0
        || field(&object.1, "TYPE")
            .unwrap_or("STANDARD")
            .split_whitespace()
            .next()
            != Some("STANDARD")
    {
        return Err("物体面はSTANDARDのSURF 0に対応しています".into());
    }
    let distance = scalar(&object.1, "DISZ", f64::INFINITY)?;
    if field(&object.1, "GLAS").is_some_and(|s| !s.trim().is_empty()) {
        return Err("物体側に材料を指定した照射は未対応です".into());
    }
    let finite = distance.is_finite();
    if finite && distance <= 0. || !finite && distance != f64::INFINITY {
        return Err("物体距離には正の距離またはINFINITYを指定してください".into());
    }
    if kind == 1. && !finite {
        return Err("Object Heightは有限距離の物体に対応しています".into());
    }
    if kind == 0. && xs.iter().chain(&ys).any(|v| v.abs() >= 90.) {
        return Err("Angle視野は各軸±90度未満に対応しています".into());
    }
    let needs_pupil = finite || xs.iter().chain(&ys).any(|v| *v != 0.);
    // Vignetting factors intentionally require an explicit implementation.
    for key in ["VDXN", "VDYN", "VCXN", "VCYN", "VANN"] {
        if values(key, 0.)?.iter().any(|v| *v != 0.) {
            return Err(format!("視野の{key}補正は未対応です"));
        }
    }
    let mut pupil = None;
    if needs_pupil {
        if scalar(all, "RAIM", 0.)? != 0. {
            return Err("Ray Aimingを必要とする視野照射は未対応です".into());
        }
        let diameter = scalar(all, "ENPD", f64::NAN)?;
        if !diameter.is_finite() || diameter <= 0. {
            return Err("視野照射にはENPDによる正の入射瞳径が必要です".into());
        }
        let stop = blocks
            .iter()
            .find(|(_, l)| l.iter().any(|v| v.trim() == "STOP"))
            .ok_or("視野照射にはSTOP面が必要です")?
            .0;
        let i = bindings
            .iter()
            .position(|b| *b == stop)
            .ok_or("STOP面の座標を解決できません")?;
        if scene.surfaces[..i].iter().any(|s|!matches!(s.interaction,Interaction::Dielectric{minus_medium,plus_medium,..} if minus_medium==plus_medium)) {
            return Err("STOPより前に屈折・反射面がある視野照射は、入射瞳の計算とRay Aimingが未対応です".into());
        }
        let pose = &scene.surfaces[i].pose;
        if pose.rotation_deg.norm() > 1e-12
            || pose.translation_mm.0[..2].iter().any(|v| v.abs() > 1e-12)
            || pose.translation_mm.0[2] < 0.
        {
            return Err("視野照射のSTOPは物体側Z軸上・無回転の配置に対応しています".into());
        }
        pupil = Some(PupilIllumination {
            pose: pose.clone(),
            aperture: Aperture::Circle {
                radius_mm: diameter / 2.,
            },
            finite_object: finite,
            surface: Some(i),
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
            s.pupil = Some(p.clone());
        }
        result.push(s);
    }
    Ok(result)
}
