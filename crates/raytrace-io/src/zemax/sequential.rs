use super::nsc::read_source_spectrum;
use super::*;
use crate::Result;

fn read_coating(
    path: &Path,
    extra: &[PathBuf],
    selected: Option<&str>,
    name: &str,
) -> Result<raytrace_core::coating::Coating> {
    let mut dirs = vec![path.parent().unwrap_or(Path::new(".")).to_path_buf()];
    dirs.extend_from_slice(extra);
    if let Some(user) = std::env::var_os("USERPROFILE") {
        dirs.push(PathBuf::from(user).join("Documents/Zemax/Coatings"));
    }
    for dir in dirs {
        let Ok(entries) = fs::read_dir(dir) else {
            continue;
        };
        let mut files: Vec<_> = entries
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|s| s.eq_ignore_ascii_case("dat")))
            .filter(|p| {
                selected.is_none_or(|s| {
                    p.file_name()
                        .is_some_and(|n| n.eq_ignore_ascii_case(s.trim_matches('"')))
                })
            })
            .collect();
        files.sort();
        for file in files {
            if fs::metadata(&file).map_or(true, |m| m.len() > 32 * 1024 * 1024) {
                continue;
            }
            let bytes = fs::read(&file).map_err(|e| crate::Error::io(&file, e))?;
            if let Some(c) = raytrace_core::coating::parse_zemax(&decode(&bytes).0, name)? {
                return Ok(c);
            }
        }
    }
    Err(crate::Error::zmx("IDEAL/TABLE定義が見つかりません。コーティングDATをZMXと同じフォルダー、または追加カタログへ配置してください".into()))
}

pub(super) fn read_sequential(
    all: &[String],
    location: (&Path, &[PathBuf]),
    wavelength: f64,
    project: &mut Project,
    glasses: &BTreeMap<String, Glass>,
    records: &mut BTreeMap<String, String>,
    bindings: &mut Vec<usize>,
) -> Result<()> {
    let (path, extra) = location;
    let mut blocks: Vec<(usize, Vec<String>)> = Vec::new();
    for line in all {
        if let Some(tail) = line.trim().strip_prefix("SURF ") {
            blocks.push((
                tail.trim()
                    .parse()
                    .map_err(|_| crate::Error::zmx("SURF番号が不正です".into()))?,
                Vec::new(),
            ));
        } else if let Some((_, lines)) = blocks.last_mut() {
            lines.push(line.clone());
        }
    }
    if blocks.len() < 2 {
        return Err(crate::Error::zmx("ZMXに面データがありません".into()));
    }
    let pupil = scalar(all, "ENPD", 10.0)?.abs() / 2.0;
    let mut frame_pose = Pose::default();
    let mut medium = 0;
    let mut forward = true;
    let last = blocks
        .iter()
        .rev()
        .find(|(i, lines)| {
            *i != 0
                && field(lines, "TYPE").and_then(|s| s.split_whitespace().next())
                    != Some("COORDBRK")
        })
        .ok_or_else(|| crate::Error::zmx("ZMXに光学面がありません".into()))?
        .0;
    for (id, lines) in blocks.iter().skip(1) {
        let typ = field(lines, "TYPE")
            .unwrap_or("STANDARD")
            .split_whitespace()
            .next()
            .unwrap_or("");
        let curvature = scalar(lines, "CURV", 0.0)?;
        let conic = scalar(lines, "CONI", 0.0)?;
        let coeff = checked_parameters(lines, "PARM")?;
        if typ == "COORDBRK" {
            if coeff.iter().any(|(i, v)| *i > 6 && *v != 0.) {
                project.issue(
                    "reference",
                    format!("面{id}: Coordinate Breakの追加パラメーターは未対応です"),
                    true,
                );
            }
            let value = |k| *coeff.get(&k).unwrap_or(&0.);
            let decenter = V3([value(1), value(2), 0.]);
            let tilt = V3([value(3), value(4), value(5)]);
            let local = if value(6) == 0. {
                raytrace_core::math::Frame::from_intrinsic_xyz(&Pose {
                    translation_mm: decenter,
                    rotation_deg: tilt,
                })
                .pose()
            } else {
                let f = raytrace_core::math::Frame::new(&Pose {
                    translation_mm: V3::default(),
                    rotation_deg: tilt,
                });
                Pose {
                    translation_mm: f.world_vector(decenter),
                    rotation_deg: tilt,
                }
            };
            for line in lines {
                let w = words(line);
                if w.first().is_some_and(|k| {
                    ![
                        "TYPE", "COMM", "CURV", "CONI", "PARM", "DISZ", "DIAM", "HIDE", "SLAB",
                        "MEMA", "POPS", "FIMP", "NOTE",
                    ]
                    .contains(k)
                }) {
                    project.issue(
                        "reference",
                        format!("面{id}: Coordinate Breakの{}は未対応です", w[0]),
                        true,
                    );
                }
            }
            frame_pose = raytrace_core::math::Frame::new(&frame_pose).transform_pose(&local);
            let distance = scalar(lines, "DISZ", 0.)?;
            if !distance.is_finite() {
                return Err(crate::Error::zmx(
                    "Coordinate Breakの厚さは有限値が必要です".into(),
                ));
            }
            frame_pose.translation_mm =
                raytrace_core::math::Frame::new(&frame_pose).world_point(V3([0., 0., distance]));
            if distance != 0. {
                forward = distance > 0.;
            }
            project.issue(
                "coordinate_break",
                format!("面{id}: Coordinate Breakの変換順序と厚さを適用"),
                false,
            );
            continue;
        }
        if coeff.iter().any(|(i, v)| *i > 9 && *v != 0.0) {
            project.issue(
                "surface",
                format!("面{id}: A18より高い非球面項は未対応です"),
                true,
            );
        }
        let shape = match typ {
            "STANDARD" if curvature == 0.0 => Shape::Plane,
            "STANDARD" if conic == 0.0 => Shape::Sphere {
                radius_mm: 1.0 / curvature,
            },
            "STANDARD" | "EVENASPH" => {
                if coeff.get(&1).is_some_and(|v| v.abs() > 0.0) {
                    project.issue(
                        "surface",
                        format!("面{id}: EVENASPHの2次追加項は未対応です"),
                        true,
                    );
                }
                let coefficients: Vec<_> =
                    (2..=9).map(|i| *coeff.get(&i).unwrap_or(&0.0)).collect();
                if curvature != 0.0 {
                    Shape::EvenAsphere {
                        radius_mm: 1.0 / curvature,
                        conic,
                        coefficients,
                    }
                } else if coefficients.iter().any(|v| *v != 0.0) {
                    Shape::FlatEvenAsphere { coefficients }
                } else {
                    Shape::Plane
                }
            }
            _ => {
                project.issue(
                    "surface",
                    format!("面{id}: 面型{typ}は表示用に保持します。追跡は未対応です"),
                    true,
                );
                Shape::Plane
            }
        };
        let semi = scalar(lines, "DIAM", pupil.max(1.0))?.abs().max(1e-5);
        let aperture = if let Some(rect) = field(lines, "SQAP") {
            let w = words(rect);
            if w.len() < 2 {
                return Err(crate::Error::zmx("SQAPの寸法が不足しています".into()));
            }
            Aperture::Rectangle {
                half_width_mm: number(w[0])?,
                half_height_mm: number(w[1])?,
            }
        } else if let Some(circle) = field(lines, "CLAP") {
            let w = words(circle);
            if w.len() < 2 {
                return Err(crate::Error::zmx("CLAPの寸法が不足しています".into()));
            }
            if number(w[0])? != 0.0 {
                project.issue(
                    "aperture",
                    format!("面{id}: 中央遮蔽を含む円形開口は未対応です"),
                    true,
                );
            }
            Aperture::Circle {
                radius_mm: number(w[1])?,
            }
        } else {
            Aperture::Circle { radius_mm: semi }
        };
        let glass = field(lines, "GLAS")
            .and_then(|s| s.split_whitespace().next())
            .unwrap_or("");
        let mirror = glass.eq_ignore_ascii_case("MIRROR");
        let after = if mirror {
            medium
        } else {
            material_id(project, glasses, glass, records)
        };
        let interaction = if *id == last {
            Interaction::Detector { pixels: [128, 128] }
        } else if mirror {
            Interaction::Mirror { reflectance: 1. }
        } else {
            Interaction::Dielectric {
                minus_medium: if forward { medium } else { after },
                plus_medium: if forward { after } else { medium },
                reflectance: None,
            }
        };
        let coating =
            if let Some(coat) = field(lines, "COAT").filter(|s| !s.is_empty() && *s != "_NONE_") {
                let name = coat
                    .split_whitespace()
                    .next()
                    .unwrap_or("")
                    .trim_matches('"');
                match read_coating(path, extra, field(all, "COFN"), name) {
                    Ok(c)
                        if matches!(
                            interaction,
                            Interaction::Dielectric { .. } | Interaction::Mirror { .. }
                        ) =>
                    {
                        Some(c)
                    }
                    Ok(_) => {
                        project.issue(
                            "coating",
                            format!("面{id}: 検出器・吸収面へのコーティング適用は未対応です"),
                            true,
                        );
                        None
                    }
                    Err(e) => {
                        project.issue("coating", format!("面{id}: {name}: {e}"), true);
                        None
                    }
                }
            } else {
                None
            };
        for line in lines {
            let w = words(line);
            if w.is_empty() {
                continue;
            }
            if ![
                "TYPE", "COMM", "STOP", "FIMP", "CURV", "CONI", "PARM", "DISZ", "GLAS", "DIAM",
                "SQAP", "CLAP", "COAT", "HIDE", "MIRR", "SLAB", "POPS", "FLAP", "MEMA", "SSID",
                "MNUM", "OPER", "NOTE", "PUSH", "TOL", "TOLR",
            ]
            .contains(&w[0])
            {
                project.issue(
                    "surface",
                    format!("面{id}: 未対応レコード{}の作用を確認してください", w[0]),
                    true,
                );
            }
            if ["SLEN", "PICK", "XDAT", "COOR", "NSOH", "ELAP", "OBSC"].contains(&w[0]) {
                project.issue(
                    "surface",
                    format!("面{id}: {}の光学的な作用は未対応です", w[0]),
                    true,
                );
            }
        }
        project.scene.surfaces.push(Surface {
            coating,
            name: format!("S{id} {}", field(lines, "COMM").unwrap_or(typ)),
            pose: frame_pose.clone(),
            shape,
            aperture,
            interaction,
        });
        bindings.push(*id);
        let distance = scalar(lines, "DISZ", 0.0)?;
        if !distance.is_finite() {
            project.issue("surface", format!("面{id}: 無限の面間隔は未対応です"), true);
        } else {
            frame_pose.translation_mm =
                raytrace_core::math::Frame::new(&frame_pose).world_point(V3([0., 0., distance]));
            if distance != 0. {
                forward = distance > 0.;
            } else if mirror {
                forward = !forward;
            }
        }
        medium = after;
    }
    let first = project
        .scene
        .surfaces
        .first()
        .ok_or_else(|| crate::Error::zmx("ZMXに追跡可能な光学面がありません".into()))?
        .pose
        .translation_mm
        .0[2];
    let spectrum = match read_source_spectrum(path, all, &[], 0.) {
        Ok(Some(s)) if s.samples.len() > 1 => Some(s),
        Ok(_) => None,
        Err(e) => {
            project.issue("spectrum", e.to_string(), true);
            None
        }
    };
    let base = Source {
        pupil: None,
        spectrum,
        sampling: Sampling::Sobol,
        count: 8192,
        origin_mm: V3([0.0, 0.0, first - 1.0]),
        direction: V3([0.0, 0.0, 1.0]),
        disk_radius_mm: pupil.max(0.01),
        cone_half_angle_deg: 0.0,
        wavelength_um: wavelength,
        medium: 0,
        total_weight: 1.0,
        seed: 42,
        diode: None,
    };
    match illumination::sources(all, &blocks, &project.scene, bindings, base.clone()) {
        Ok(sources) => project.scene.sources = sources,
        Err(e) => {
            project.scene.sources.push(base);
            project.issue("illumination_definition", e.to_string(), true);
        }
    }
    project.issue("illumination","逐次ZMXの波長表・視野の相対重みを使用。光量の基準は瞳補正後の発射光線の重み合計を100%とします。入射瞳、Ray Aiming時はSTOPのXY座標を面積一様に選び、視野の瞳補正を適用します。実開口の外れは光量収支へ記録します",false);
    Ok(())
}
