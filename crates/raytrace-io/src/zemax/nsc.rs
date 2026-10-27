use super::*;
use crate::Result;

pub(super) fn read_source_spectrum(
    path: &Path,
    all: &[String],
    lines: &[String],
    wave_index: f64,
) -> Result<Option<raytrace_core::spectrum::Spectrum>> {
    let settings = lines.iter().find(|l| {
        let w = words(l);
        w.starts_with(&["NSCS", "7", "3"])
    });
    let parts = settings.map(|l| words(l));
    if parts.as_ref().is_some_and(|w| w.len() < 10) {
        return Err(crate::Error::zmx(
            "NSCS 7 3のスペクトル条件が不足しています".into(),
        ));
    }
    let mode = parts
        .as_ref()
        .map(|w| number(w[3]))
        .transpose()?
        .unwrap_or(0.);
    let mut samples = match mode {
        0. => {
            let waves = parameters(all, "WAVM");
            if !wave_index.is_finite() || wave_index < 0. || wave_index.fract() != 0. {
                return Err(crate::Error::zmx("光源の波長番号が不正です".into()));
            }
            if wave_index > 0. {
                if !waves.contains_key(&(wave_index as usize)) {
                    return Err(crate::Error::zmx(
                        "光源の指定波長番号が波長表にありません".into(),
                    ));
                }
                return Ok(None);
            }
            let mut v = Vec::new();
            for line in all {
                let w = words(line);
                if w.first() == Some(&"WAVM") && w.len() >= 3 {
                    v.push([
                        number(w[2])?,
                        w.get(3).map(|s| number(s)).transpose()?.unwrap_or(1.),
                    ]);
                }
            }
            v.sort_by(|a, b| a[0].total_cmp(&b[0]));
            let mut merged: Vec<[f64; 2]> = Vec::new();
            for point in v {
                if let Some(last) = merged.last_mut().filter(|v| v[0] == point[0]) {
                    last[1] += point[1];
                } else {
                    merged.push(point);
                }
            }
            merged
        }
        8. => {
            let line = settings.unwrap();
            let name = line
                .split('"')
                .nth(1)
                .ok_or_else(|| crate::Error::zmx("SPCDのファイル名が不足しています".into()))?;
            let mut files = vec![path.parent().unwrap_or(Path::new(".")).join(name)];
            if let Some(user) = std::env::var_os("USERPROFILE") {
                files.push(
                    PathBuf::from(user)
                        .join("Documents/Zemax/Objects/Sources/Spectrum Files")
                        .join(name),
                );
            }
            let file = files.iter().find(|p| p.is_file()).ok_or_else(|| {
                crate::Error::zmx(format!("参照スペクトル{name}が見つかりません"))
            })?;
            return read_spcd(file).map(Some);
        }
        4. | 7. => {
            let w = parts.as_ref().unwrap();
            let lo = number(w[7])?;
            let hi = number(w[8])?;
            let steps = number(w[9])?;
            if !lo.is_finite()
                || lo <= 0.
                || !hi.is_finite()
                || hi <= lo
                || !(2.0..=200.0).contains(&steps)
                || steps.fract() != 0.
            {
                return Err(crate::Error::zmx("スペクトルの範囲・点数が不正です".into()));
            }
            let mut v: Vec<_> = (0..steps as usize)
                .map(|i| [lo + (hi - lo) * i as f64 / (steps - 1.), 1.])
                .collect();
            if mode == 7. {
                let temperature = number(w[4])?;
                if !temperature.is_finite() || temperature <= 0. {
                    return Err(crate::Error::zmx(
                        "黒体温度は正の有限値[K]で指定してください".into(),
                    ));
                }
                let logs: Vec<_> = v
                    .iter()
                    .map(|v| {
                        let x = 14387.768775039337 / (v[0] * temperature);
                        -5. * v[0].ln() - if x > 700. { x } else { x.exp_m1().ln() }
                    })
                    .collect();
                let peak = logs.iter().copied().fold(f64::NEG_INFINITY, f64::max);
                for (sample, log) in v.iter_mut().zip(logs) {
                    sample[1] = (log - peak).exp();
                }
            }
            v
        }
        _ => {
            return Err(crate::Error::zmx(format!(
                "光源の色・スペクトルモデル{mode}は未対応です。単色波長を明示できます"
            )));
        }
    };
    if samples.is_empty() {
        return Err(crate::Error::zmx("光源に波長と重みがありません".into()));
    }
    let spectrum = raytrace_core::spectrum::Spectrum {
        samples: std::mem::take(&mut samples),
    };
    spectrum.validate()?;
    Ok(Some(spectrum))
}

pub(super) fn nsc_reference(value: &str, row: usize) -> Result<usize> {
    let v = number(value)?;
    if !v.is_finite() || v.fract() != 0. || v.abs() > 1_000_000. {
        return Err(crate::Error::zmx(
            "参照番号は範囲内の整数で指定してください".into(),
        ));
    }
    let target = if v < 0. {
        row as i64 + v as i64
    } else {
        v as i64
    };
    if target < 0 || target >= row as i64 {
        return Err(crate::Error::zmx(
            "参照先はこのオブジェクトより前にある必要があります（前方・自己・循環参照）".into(),
        ));
    }
    Ok(target as usize)
}

pub(super) fn read_nsc(
    all: &[String],
    path: &Path,
    wavelength: f64,
    project: &mut Project,
    glasses: &BTreeMap<String, Glass>,
    records: &mut BTreeMap<String, String>,
    deflection_mm: f64,
) -> Result<()> {
    let mut objects: Vec<Vec<String>> = Vec::new();
    for line in all {
        if line.trim_start().starts_with("NSOH ") {
            objects.push(Vec::new());
        }
        if let Some(block) = objects.last_mut() {
            block.push(line.clone());
        }
    }
    let mut object_poses = vec![Pose::default()];
    let mut object_media = vec![0usize];
    let mut volumes = vec![false];
    for (i, lines) in objects.iter().enumerate() {
        let head = words(field(lines, "NSOH").unwrap_or(""));
        if head.is_empty() {
            return Err(crate::Error::zmx("NSOHオブジェクト型がありません".into()));
        }
        let typ = head[0];
        let reference = head
            .get(3..)
            .unwrap_or(&[])
            .join(" ")
            .trim_matches('"')
            .to_owned();
        let (local_pose, glass) = pose_from(field(lines, "NSOP").unwrap_or("0 0 0 0 0 0"))?;
        let p = checked_parameters(lines, "NSOD")?;
        let param = |k, default| *p.get(&k).unwrap_or(&default);
        let label = format!("O{} {typ}", i + 1);
        let mut resolve = |slot| match nsc_reference(head.get(slot).copied().unwrap_or("0"), i + 1)
        {
            Ok(id) => id,
            Err(e) => {
                project.issue("reference", format!("{label}: {e}"), true);
                0
            }
        };
        let reference_id = resolve(1);
        let inside_id = resolve(2);
        let pose = raytrace_core::math::Frame::new(&object_poses[reference_id])
            .transform_pose(&raytrace_core::math::Frame::from_intrinsic_xyz(&local_pose).pose());
        object_poses.push(pose.clone());
        if inside_id != 0 && !volumes[inside_id] {
            project.issue(
                "reference",
                format!("{label}: Inside Ofは先行する屈折体を指定してください"),
                true,
            );
        }
        let surrounding_medium = object_media[inside_id];
        let object_medium = if typ == "NSC_IMPT" && !glass.is_empty() {
            material_id(project, glasses, &glass, records)
        } else {
            0
        };
        object_media.push(object_medium);
        volumes.push(typ == "NSC_IMPT" && !glass.is_empty());
        for line in lines {
            let w = words(line);
            let Some(key) = w.first() else {
                continue;
            };
            if ![
                "NSOH", "NSOP", "NSOD", "NSOO", "NSCS", "COMM", "MNUM", "OPER", "NOTE", "PUSH",
                "TOL", "TOLR",
            ]
            .contains(key)
            {
                project.issue(
                    "object",
                    format!("{label}: 未対応レコード{key}の作用を確認してください"),
                    true,
                );
            }
        }
        for line in lines {
            let w = words(line);
            if w.first() == Some(&"NSCS") && w.get(1).is_none_or(|id| !["1", "7"].contains(id)) {
                project.issue(
                    "object",
                    format!(
                        "{label}: NSCS属性{}の光学的な作用は未対応です",
                        w.get(1).copied().unwrap_or("不明")
                    ),
                    true,
                );
            }
        }
        // Check optical face conditions before a geometry-only object is skipped.
        if lines.iter().any(|l| {
            let w = words(l);
            w.len() > 3
                && w[0] == "NSCS"
                && w[1] == "1"
                && w.last()
                    .is_some_and(|s| !["_NONE_", ""].contains(&s.trim_matches('"')))
        }) {
            project.issue(
                "coating",
                format!("{label}: 面コーティングの追跡は未対応です"),
                true,
            );
        }
        if field(lines, "NSOO").is_some_and(|s| {
            s.split_whitespace()
                .filter_map(|v| number(v).ok())
                .any(|v| v != 0.0)
        }) {
            project.issue(
                "reference",
                format!("{label}: オブジェクトの追加オフセットNSOOは未対応です"),
                true,
            );
        }
        match typ {
            "NSC_SDIO" => {
                let frame = raytrace_core::math::Frame::new(&pose);
                let waves = parameters(all, "WAVM");
                let source_wave = *waves.get(&(param(4, 1.0) as usize)).unwrap_or(&wavelength);
                let spectrum = match read_source_spectrum(path, all, lines, param(4, 1.)) {
                    Ok(spectrum) => spectrum,
                    Err(e) => {
                        project.issue("spectrum", format!("{label}: {e}"), true);
                        None
                    }
                };
                project.scene.sources.push(Source {
                    pupil: None,
                    spectrum,
                    sampling: Sampling::Sobol,
                    count: param(2, 1000.0).max(1.0) as usize,
                    origin_mm: pose.translation_mm,
                    direction: frame.world_vector(V3([0.0, 0.0, 1.0])),
                    disk_radius_mm: 0.0,
                    cone_half_angle_deg: 0.0,
                    wavelength_um: source_wave,
                    medium: surrounding_medium,
                    total_weight: param(3, 1.0),
                    seed: 42 + i as u64,
                    diode: Some(Diode {
                        astigmatism_mm: param(6, 0.),
                        angular_supergaussian: [param(8, 1.), param(10, 1.)],
                        spatial_supergaussian: [param(17, 1.), param(20, 1.)],
                        rotation_deg: pose.rotation_deg,
                        divergence_deg: [param(7, 1.0), param(9, 1.0)],
                        array: [param(11, 1.0) as usize, param(12, 1.0) as usize],
                        spacing_mm: [param(13, 0.0), param(14, 0.0)],
                        half_width_mm: [param(15, 0.0), param(18, 0.0)],
                        gaussian_width_mm: [param(16, 0.0), param(19, 0.0)],
                    }),
                });
                if let Err(e) = project
                    .scene
                    .sources
                    .last()
                    .unwrap()
                    .diode
                    .as_ref()
                    .unwrap()
                    .validate()
                {
                    project.issue("source", format!("{label}: {e}"), true);
                }
            }
            "NSC_DETC" => project.scene.surfaces.push(Surface {
                coating: None,
                name: label.clone(),
                pose,
                shape: Shape::Plane,
                aperture: Aperture::Rectangle {
                    half_width_mm: param(1, 1.0),
                    half_height_mm: param(2, 1.0),
                },
                interaction: Interaction::Detector {
                    pixels: [param(3, 64.0) as usize, param(4, 64.0) as usize],
                },
            }),
            "NSC_IMPT" => {
                if glass.is_empty() {
                    project.issue(
                        "air_cad",
                        format!("{reference}: 材料が空気のCAD部品として元定義を保持します"),
                        false,
                    );
                    continue;
                }
                let medium = object_medium;
                if param(1, 1.0) != 1.0 || param(2, 1.0) != 1.0 {
                    project.issue(
                        "cad",
                        format!("{label}: CADのスケールまたは体積設定が未対応です"),
                        true,
                    );
                }
                let step = path.parent().unwrap_or(Path::new(".")).join(&reference);
                match crate::cad::load_step_with_tolerance(&step, deflection_mm) {
                    Ok(shape) => project.scene.surfaces.push(Surface {
                        coating: None,
                        name: format!("{label} {reference}"),
                        pose,
                        shape,
                        aperture: Aperture::Rectangle {
                            half_width_mm: 1e8,
                            half_height_mm: 1e8,
                        },
                        interaction: Interaction::Dielectric {
                            minus_medium: medium,
                            plus_medium: surrounding_medium,
                            reflectance: None,
                        },
                    }),
                    Err(e) => project.issue("cad", e.to_string(), true),
                }
            }
            "NSC_NULL" => (),
            _ => project.issue(
                "object",
                format!("{label}: 未対応のNSCオブジェクトです。元定義は保持します"),
                true,
            ),
        }
    }
    if project
        .scene
        .surfaces
        .iter()
        .any(|s| matches!(s.shape, Shape::TriangleMesh { .. }))
    {
        project.cad_deflection_mm = Some(deflection_mm);
        project.issue("cad_accuracy",format!("STEPはRustで{deflection_mm} mm設定の適応分割を行い、曲面法線を補間します。分割精度を変えて結果の収束を確認してください"),false);
    }
    Ok(())
}
