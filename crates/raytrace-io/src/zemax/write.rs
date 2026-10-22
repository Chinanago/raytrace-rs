use super::*;

/// Preserve the complete source text on an unchanged document. Geometry edits
/// are patched only for supported coaxial sequential surfaces.
pub fn export(project: &Project, path: &Path) -> Result<(), String> {
    if project.assembly.is_some() {
        return Err(
            "組立光学系はJSONプロジェクトで保存できます。部品を含むZMX書出しは追加予定です".into(),
        );
    }
    let mut sidecars = exchange::recover(project, path)?;
    if let Some(original) = &project.zemax {
        if serde_json::to_value(&project.scene).map_err(|e| e.to_string())?
            == serde_json::to_value(&original.baseline_scene).map_err(|e| e.to_string())?
        {
            copy_dependencies(original, path)?;
            exchange::write(&sidecars)?;
            return fs::write(path, encode(&original.text, &original.encoding)?)
                .map_err(|e| e.to_string());
        }
    }
    let scene = &project.scene;
    scene.clone().prepare()?;
    if project
        .issues
        .iter()
        .any(|i| i.blocking && !matches!(i.code.as_str(), "coating" | "spectrum"))
    {
        return Err("未対応の光学定義を含むZMXの編集はJSONへ保存してください。元ZMXの無変更保存に対応しています".into());
    }
    if project
        .zemax
        .as_ref()
        .is_some_and(|z| z.text.lines().any(|l| l.contains("COORDBRK")))
    {
        return Err(
            "座標連鎖を含むモデルの編集はJSONへ保存してください。元ZMXの無変更保存に対応しています"
                .into(),
        );
    }
    let baseline = project.zemax.as_ref().map(|z| &z.baseline_scene.surfaces);
    let coatings_changed = scene.surfaces.iter().enumerate().any(|(i, s)| {
        serde_json::to_value(&s.coating).ok()
            != serde_json::to_value(
                baseline
                    .and_then(|v| v.get(i))
                    .and_then(|s| s.coating.as_ref()),
            )
            .ok()
    });
    if coatings_changed && scene.mode == Mode::NonSequential {
        return Err(
            "コーティングを変更したモデルのZMX書出しは追加予定です。JSONには全条件を保存できます"
                .into(),
        );
    }
    if scene.mode == Mode::NonSequential {
        if let Some(original) = &project.zemax {
            let text = patch_nsc(project, original, path, &mut sidecars)?;
            let bytes = encode(&text, &original.encoding)?;
            copy_dependencies(original, path)?;
            exchange::write(&sidecars)?;
            return fs::write(path, bytes).map_err(|e| e.to_string());
        }
    }
    if scene.mode != Mode::Sequential {
        return Err("非シーケンシャルZMXは元定義の無変更保存に対応しています。編集したモデルはJSONプロジェクトで保存してください".into());
    }
    if scene.surfaces.iter().any(|s| {
        s.pose.translation_mm.0[..2].iter().any(|v| v.abs() > 1e-12)
            || s.pose.rotation_deg.norm() > 1e-12
    }) {
        return Err(
            "傾き・偏心を含むZMX書出しは追加予定です。JSONプロジェクトには保存できます".into(),
        );
    }
    let original = project.zemax.as_ref();
    if let Some(z) = original.filter(|z| {
        z.baseline_scene.sources.len() > 1
            && z.baseline_scene.sources.iter().any(|s| s.pupil.is_some())
    }) {
        let normalized = |sources: &[Source]| {
            let total = sources.iter().map(|s| s.total_weight).sum::<f64>();
            sources
                .iter()
                .map(|s| s.total_weight / total)
                .collect::<Vec<_>>()
        };
        if normalized(&scene.sources) != normalized(&z.baseline_scene.sources) {
            return Err("視野別の光量重みの編集はJSONへ保存してください".into());
        }
        let spectrum = |s: &Source| {
            let mut samples = s
                .spectrum
                .as_ref()
                .map_or(vec![[s.wavelength_um, 1.]], |s| s.samples.clone());
            let total = samples.iter().map(|v| v[1]).sum::<f64>();
            for v in &mut samples {
                v[1] /= total;
            }
            samples
        };
        if scene
            .sources
            .windows(2)
            .any(|w| spectrum(&w[0]) != spectrum(&w[1]))
        {
            return Err(
                "視野別に異なるスペクトルは単一のWAVM表で表現できません。JSONへ保存してください"
                    .into(),
            );
        }
    }
    let same_illumination = original.is_some_and(|z| {
        let signature = |s: &Scene| {
            s.sources
                .iter()
                .map(|s| {
                    serde_json::to_value((
                        &s.pupil,
                        s.origin_mm,
                        s.direction,
                        s.disk_radius_mm,
                        s.cone_half_angle_deg,
                        &s.diode,
                    ))
                    .ok()
                })
                .collect::<Vec<_>>()
        };
        signature(scene) == signature(&z.baseline_scene)
    });
    if !same_illumination
        && scene
            .sources
            .windows(2)
            .any(|s| s[0].disk_radius_mm != s[1].disk_radius_mm || s[0].origin_mm != s[1].origin_mm)
    {
        return Err(
            "異なる照射面・瞳径の光源を1つの逐次照射へ変換できません。JSONへ保存してください"
                .into(),
        );
    }
    if !same_illumination
        && scene.sources.iter().any(|s| {
            s.pupil.is_some()
                || s.diode.is_some()
                || s.cone_half_angle_deg != 0.0
                || s.origin_mm.0[0] != 0.0
                || s.origin_mm.0[1] != 0.0
                || (s.direction.unit() - V3([0.0, 0.0, 1.0])).norm() > 1e-12
        })
    {
        return Err("逐次ZMXへの新規書出しは軸上平行光に対応しています。光源条件を含むモデルはJSONで保存できます".into());
    }
    if original.is_some_and(|z| z.surface_numbers.len() != scene.surfaces.len()) {
        return Err("元ZMXの面数が変わっています。JSONプロジェクトに保存してください".into());
    }
    if original.is_some_and(|z| {
        scene
            .surfaces
            .first()
            .zip(z.baseline_scene.surfaces.first())
            .is_some_and(|(a, b)| (a.pose.translation_mm - b.pose.translation_mm).norm() > 1e-12)
    }) {
        return Err(
            "最初の基準面の移動はこのZMXで表現できません。配置を含むJSONで保存してください".into(),
        );
    }
    let catalog = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("RAYTRACE")
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect::<String>()
        + "_RAYTRACE";
    let mut agf = exchange::generated_header(path, "CC");
    let custom_catalog = original.is_none_or(|z| {
        serde_json::to_value(&z.baseline_scene.materials).ok()
            != serde_json::to_value(&scene.materials).ok()
    });
    let mut names = vec![String::new(); scene.materials.len()];
    for (i, material) in scene.materials.iter().enumerate().skip(1) {
        let name = project
            .material_names
            .get(i)
            .cloned()
            .unwrap_or_else(|| format!("RT_M{i}"));
        names[i] = name.clone();
        if let Some(record) = original
            .and_then(|z| z.glass_records.get(&name))
            .filter(|_| {
                original.is_some_and(|z| {
                    serde_json::to_value(z.baseline_scene.materials.get(i)).ok()
                        == serde_json::to_value(Some(material)).ok()
                })
            })
        {
            agf.push_str(record);
            continue;
        }
        agf.push_str(&exchange::material_record(&name, material)?);
    }
    let coating_file = if coatings_changed && scene.surfaces.iter().any(|s| s.coating.is_some()) {
        if project
            .issues
            .iter()
            .any(|i| i.blocking && i.code == "coating")
        {
            return Err(
                "未対応コーティングを含むファイルのコーティング編集はJSONへ保存してください".into(),
            );
        }
        let mut definitions = BTreeMap::new();
        for c in scene.surfaces.iter().filter_map(|s| s.coating.as_ref()) {
            let definition = c.to_zemax()?;
            if definitions.get(&c.name).is_some_and(|v| *v != definition) {
                return Err("同じコーティング名に異なる定義があります".into());
            }
            definitions.insert(c.name.clone(), definition);
        }
        let filename = format!(
            "{}.raytrace-coatings.DAT",
            path.file_stem().unwrap_or_default().to_string_lossy()
        );
        let text = exchange::generated_header(path, "!")
            + &definitions.into_values().collect::<Vec<_>>().join("\n");
        exchange::put(
            &mut sidecars,
            path.with_file_name(&filename),
            text.into_bytes(),
        )?;
        Some(filename)
    } else {
        None
    };
    let mut replacements: BTreeMap<usize, Vec<String>> = BTreeMap::new();
    let mut forward = true;
    for (i, s) in scene.surfaces.iter().enumerate() {
        let id = original.map_or(i + 2, |z| z.surface_numbers[i]);
        let (typ, curvature, conic, coeff) = match &s.shape {
            Shape::Plane => ("STANDARD", 0.0, 0.0, Vec::new()),
            Shape::Sphere { radius_mm } => ("STANDARD", 1.0 / radius_mm, 0.0, Vec::new()),
            Shape::EvenAsphere {
                radius_mm,
                conic,
                coefficients,
            } => ("EVENASPH", 1.0 / radius_mm, *conic, coefficients.clone()),
            _ => return Err("この面形状の逐次ZMX書出しは未対応です".into()),
        };
        let (hx, hy) = s.aperture.bounds();
        let semi = if matches!(s.aperture, Aperture::Circle { .. }) {
            hx
        } else {
            hx.hypot(hy)
        };
        let gap = scene.surfaces.get(i + 1).map_or(0.0, |next| {
            next.pose.translation_mm.0[2] - s.pose.translation_mm.0[2]
        });
        let mut lines = vec![
            format!("SURF {id}"),
            format!("  COMM {}", s.name.replace(['\r', '\n'], " ")),
            format!("  TYPE {typ}"),
            format!("  CURV {curvature:.17e} 0 0 0 0 \"\""),
            format!("  CONI {conic:.17e}"),
            format!("  DISZ {gap:.17e}"),
            format!("  DIAM {semi:.17e} 0 0 0 1 \"\""),
        ];
        if let Some(coating) = &s.coating {
            lines.push(format!("  COAT {}", coating.name));
        }
        match s.interaction {
            Interaction::Dielectric {
                minus_medium,
                plus_medium,
                reflectance: None,
                ..
            } => {
                let after = if forward { plus_medium } else { minus_medium };
                if after != 0 {
                    lines.push(format!("  GLAS {} 0 0 1.5 50 0 0", names[after]));
                }
            }
            Interaction::Detector { .. } if i + 1 == scene.surfaces.len() => (),
            Interaction::Mirror { reflectance } if reflectance == 1. || s.coating.is_some() => {
                lines.push("  GLAS MIRROR 0 0".into())
            }
            _ => return Err("固定反射率・ミラー・中間検出器のZMX書出しは未対応です".into()),
        }
        if let Aperture::Rectangle { .. } = s.aperture {
            lines.push(format!("  SQAP {hx:.17e} {hy:.17e} 0"));
        } else {
            lines.push(format!("  CLAP 0 {hx:.17e} 0"));
        }
        for (j, c) in coeff.iter().enumerate() {
            lines.push(format!("  PARM {} {c:.17e}", j + 2));
        }
        replacements.insert(id, lines);
        if gap != 0. {
            forward = gap > 0.;
        } else if matches!(s.interaction, Interaction::Mirror { .. }) {
            forward = !forward;
        }
    }
    let text = if let Some(original) = original {
        let mut output = Vec::new();
        let mut active = None;
        for line in original.text.lines() {
            if let Some(rest) = line.trim().strip_prefix("SURF ") {
                active = rest.trim().parse::<usize>().ok();
                if let Some(lines) = active.and_then(|id| replacements.get(&id)) {
                    output.extend(lines.clone());
                    continue;
                }
            }
            let key = words(line).first().copied().unwrap_or("");
            if key == "COAT"
                && active
                    .and_then(|id| original.surface_numbers.iter().position(|n| *n == id))
                    .is_some_and(|i| {
                        scene.surfaces[i].coating.is_some()
                            || original.baseline_scene.surfaces[i].coating.is_some()
                    })
            {
                continue;
            }
            if active.is_some_and(|id| replacements.contains_key(&id))
                && [
                    "SURF", "COMM", "TYPE", "CURV", "CONI", "DISZ", "DIAM", "GLAS", "SQAP", "CLAP",
                    "FLAP", "PARM",
                ]
                .contains(&key)
            {
                continue;
            }
            if key == "GCAT" && custom_catalog {
                output.push(format!(
                    "GCAT {catalog} {}",
                    line.trim().strip_prefix("GCAT").unwrap_or("")
                ));
            } else {
                output.push(line.into());
            }
        }
        output.join("\r\n") + "\r\n"
    } else {
        let waves = exchange::sequential_waves(scene)?;
        let pupil = scene
            .sources
            .first()
            .map_or(1.0, |s| 2.0 * s.disk_radius_mm.max(0.01));
        let mut head=format!("VERS 260000 0 0\r\nMODE SEQ\r\nNAME {}\r\nUNIT MM X W X CM MR CPMM\r\nENPD {pupil}\r\nENVD 20 1 0\r\nGCAT {catalog}\r\nFTYP 0 0 1 1 0 0 0\r\nXFLN 0\r\nYFLN 0\r\nPWAV 1\r\n",scene.name.replace(['\r','\n']," "));
        for (i, [w, weight]) in waves.iter().enumerate() {
            head.push_str(&format!("WAVM {} {w:.17e} {weight:.17e}\r\n", i + 1));
        }
        if waves.is_empty() {
            head.push_str("WAVM 1 0.55 1\r\n");
        }
        head.push_str("SURF 0\r\n  TYPE STANDARD\r\n  CURV 0\r\n  DISZ INFINITY\r\n  DIAM 0\r\n");
        let first_z = scene
            .surfaces
            .first()
            .ok_or("光学面がありません")?
            .pose
            .translation_mm
            .0[2];
        head.push_str(&format!("SURF 1\r\n  TYPE STANDARD\r\n  STOP\r\n  CURV 0\r\n  DISZ {first_z:.17e}\r\n  DIAM {}\r\n",pupil/2.0));
        for lines in replacements.values() {
            head.push_str(&lines.join("\r\n"));
            head.push_str("\r\n");
        }
        head
    };
    let spectra_changed = original.is_some_and(|o| {
        let signature = |s: &Scene| {
            s.sources
                .iter()
                .map(|s| serde_json::to_value((s.wavelength_um, &s.spectrum, s.total_weight)).ok())
                .collect::<Vec<_>>()
        };
        signature(scene) != signature(&o.baseline_scene)
    });
    let text = if spectra_changed {
        let waves = exchange::sequential_waves(scene)?;
        let primary = scene
            .sources
            .first()
            .and_then(|s| waves.iter().position(|w| w[0] == s.wavelength_um))
            .map_or(1, |i| i + 1);
        let header = format!(
            "PWAV {primary}\r\n{}\r\n",
            waves
                .iter()
                .enumerate()
                .map(|(i, [w, p])| format!("WAVM {} {w:.17e} {p:.17e}", i + 1))
                .collect::<Vec<_>>()
                .join("\r\n")
        );
        header
            + &text
                .lines()
                .filter(|l| {
                    let w = words(l);
                    !matches!(w.first(), Some(&"WAVM") | Some(&"PWAV"))
                })
                .collect::<Vec<_>>()
                .join("\r\n")
    } else {
        text
    };
    let text = if let Some(filename) = coating_file {
        format!(
            "COFN {filename}\r\n{}",
            text.lines()
                .filter(|l| !l.trim_start().starts_with("COFN "))
                .collect::<Vec<_>>()
                .join("\r\n")
        )
    } else {
        text
    };
    let bytes = encode(
        &text,
        original.map_or(&TextEncoding::Utf16Le, |z| &z.encoding),
    )?;
    if let Some(original) = original {
        copy_dependencies(original, path)?;
    }
    if custom_catalog {
        exchange::put(
            &mut sidecars,
            path.with_file_name(format!("{catalog}.AGF")),
            agf.into_bytes(),
        )?;
    }
    exchange::write(&sidecars)?;
    fs::write(path, bytes).map_err(|e| e.to_string())
}

fn copy_dependencies(original: &ZmxDocument, output: &Path) -> Result<(), String> {
    use std::collections::BTreeSet;
    let from = original.source_path.parent().unwrap_or(Path::new("."));
    let to = output.parent().unwrap_or(Path::new("."));
    let mut files = BTreeSet::new();
    for line in original.text.lines() {
        let w = words(line);
        if w.is_empty() {
            continue;
        }
        if w[0] == "NSOH" && w.get(1) == Some(&"NSC_IMPT") {
            files.insert(
                w.get(4..)
                    .unwrap_or(&[])
                    .join(" ")
                    .trim_matches('"')
                    .to_string(),
            );
        }
        if w[0] == "GCAT" {
            for name in &w[1..] {
                files.insert(format!("{name}.AGF"));
            }
        }
        if w[0] == "COFN" {
            for name in &w[1..] {
                if name.contains('.') {
                    files.insert(name.trim_matches('"').into());
                }
            }
        }
        for name in line.split('"') {
            if name.to_uppercase().ends_with(".SPCD") {
                files.insert(name.into());
            }
        }
    }
    let mut pending = Vec::new();
    for name in files {
        let relative = Path::new(&name);
        if name.is_empty()
            || relative
                .components()
                .any(|c| !matches!(c, std::path::Component::Normal(_)))
        {
            continue;
        }
        let source = from.join(relative);
        if !source.is_file() {
            continue;
        }
        let destination = to.join(relative);
        let bytes = fs::read(source).map_err(|e| e.to_string())?;
        if destination.is_file() {
            if fs::read(&destination).map_err(|e| e.to_string())? != bytes {
                return Err(format!(
                    "依存ファイル{}が保存先に既にあります。別フォルダーへ保存してください",
                    destination.display()
                ));
            }
        } else {
            pending.push((destination, bytes));
        }
    }
    for (path, bytes) in pending {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        fs::write(path, bytes).map_err(|e| e.to_string())?;
    }
    Ok(())
}

fn patch_nsc(
    project: &Project,
    original: &ZmxDocument,
    output: &Path,
    sidecars: &mut exchange::Sidecars,
) -> Result<String, String> {
    let a = &project.scene;
    let b = &original.baseline_scene;
    if a.sources
        .iter()
        .zip(&b.sources)
        .any(|(s, t)| s.medium != t.medium)
    {
        return Err("Inside Ofによる初期媒質を変えたモデルはJSONで保存してください".into());
    }
    if a.sources.len() != b.sources.len()
        || a.surfaces.len() != b.surfaces.len()
        || serde_json::to_value(&a.materials).ok() != serde_json::to_value(&b.materials).ok()
    {
        return Err("NSCの光源数・面数・材料の変更はJSONプロジェクトに保存してください".into());
    }
    for (s, t) in a.surfaces.iter().zip(&b.surfaces) {
        if s.name != t.name
            || serde_json::to_value(&s.shape).ok() != serde_json::to_value(&t.shape).ok()
            || serde_json::to_value(&s.interaction).ok()
                != serde_json::to_value(&t.interaction).ok()
            || serde_json::to_value(&s.aperture).ok() != serde_json::to_value(&t.aperture).ok()
        {
            return Err("NSCの形状・材質の変更はJSONプロジェクトに保存してください".into());
        }
    }
    let spectral = exchange::nsc_spectra(a, original, output, sidecars)?;
    let local_poses = exchange::nsc_local_poses(a, original)?;
    let mut source_index = 0;
    let mut current_source = None;
    let mut object_index = 0usize;
    let mut lines = Vec::new();
    let mut values: Vec<BTreeMap<usize, f64>> = a
        .sources
        .iter()
        .map(|s| {
            let d = s
                .diode
                .as_ref()
                .ok_or("Source Diodeの定義が変わっています")?;
            Ok(BTreeMap::from([
                (2, s.count as f64),
                (3, s.total_weight),
                (6, d.astigmatism_mm),
                (7, d.divergence_deg[0]),
                (8, d.angular_supergaussian[0]),
                (9, d.divergence_deg[1]),
                (10, d.angular_supergaussian[1]),
                (11, d.array[0] as f64),
                (12, d.array[1] as f64),
                (13, d.spacing_mm[0]),
                (14, d.spacing_mm[1]),
                (15, d.half_width_mm[0]),
                (16, d.gaussian_width_mm[0]),
                (17, d.spatial_supergaussian[0]),
                (18, d.half_width_mm[1]),
                (19, d.gaussian_width_mm[1]),
                (20, d.spatial_supergaussian[1]),
            ]))
        })
        .collect::<Result<_, String>>()?;
    for (source, index) in &spectral.wave_indices {
        values[*source].insert(4, *index as f64);
    }
    let mut seen = std::collections::BTreeSet::new();
    let mut saw_spectrum = false;
    let mut inserted_waves = false;
    for line in original.text.lines() {
        let w = words(line);
        if matches!(w.first(), Some(&"NSOH") | Some(&"SURF")) {
            append_missing_parameters(&mut lines, current_source, &seen, &values);
            if !saw_spectrum {
                if let Some(row) = current_source.and_then(|i| spectral.settings[i].as_ref()) {
                    lines.push(row.clone());
                }
            }
            saw_spectrum = false;
            seen.clear();
            current_source = None;
        }
        if !inserted_waves && w.first() == Some(&"SURF") {
            lines.extend(spectral.new_waves.clone());
            inserted_waves = true;
        }
        if w.first() == Some(&"NSOH") {
            object_index += 1;
            current_source = None;
            if w.get(1) == Some(&"NSC_SDIO") {
                current_source = Some(source_index);
                source_index += 1;
            }
        }
        if w.first() == Some(&"NSOP") {
            if let Some(pose) = object_index.checked_sub(1).and_then(|i| local_poses.get(i)) {
                lines.push(format!(
                    "  NSOP {} {} {} {} {} {} {}",
                    pose.translation_mm.0[0],
                    pose.translation_mm.0[1],
                    pose.translation_mm.0[2],
                    pose.rotation_deg.0[0],
                    pose.rotation_deg.0[1],
                    pose.rotation_deg.0[2],
                    w.get(7..).unwrap_or(&[]).join(" ")
                ));
                continue;
            }
        }
        if w.starts_with(&["NSCS", "7", "3"]) {
            saw_spectrum = true;
            if let Some(row) = current_source.and_then(|i| spectral.settings[i].as_ref()) {
                lines.push(row.clone());
                continue;
            }
        }
        if w.first() == Some(&"NSOD") && w.len() >= 3 {
            if let Some(i) = current_source {
                let k = w[1].parse::<usize>().unwrap_or(0);
                seen.insert(k);
                let value = values[i].get(&k);
                if let Some(value) = value {
                    lines.push(format!(
                        "  NSOD {} {:.17e} {}",
                        w[1],
                        value,
                        w.get(3..).unwrap_or(&[]).join(" ")
                    ));
                    continue;
                }
            }
        }
        lines.push(line.into());
    }
    append_missing_parameters(&mut lines, current_source, &seen, &values);
    if !saw_spectrum {
        if let Some(row) = current_source.and_then(|i| spectral.settings[i].as_ref()) {
            lines.push(row.clone());
        }
    }
    Ok(lines.join("\r\n") + "\r\n")
}

fn append_missing_parameters(
    lines: &mut Vec<String>,
    source: Option<usize>,
    seen: &std::collections::BTreeSet<usize>,
    values: &[BTreeMap<usize, f64>],
) {
    if let Some(i) = source {
        for (key, value) in &values[i] {
            if !seen.contains(key) {
                lines.push(format!("  NSOD {key} {value:.17e} 0 0 1 0 0 \"\""));
            }
        }
    }
}
