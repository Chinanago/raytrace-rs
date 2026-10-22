//! Native ZMX/AGF parsing. Unknown data stays in the original document.
mod exchange;
mod illumination;
use crate::project::Project;
use raytrace_core::{
    math::{Pose, V3},
    model::*,
};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum TextEncoding {
    Utf8,
    Utf8Bom,
    Utf16Le,
    Utf16Be,
    Windows1252,
}
pub fn decode(bytes: &[u8]) -> (String, TextEncoding) {
    if bytes.starts_with(&[255, 254]) {
        return (
            String::from_utf16_lossy(
                &bytes[2..]
                    .chunks_exact(2)
                    .map(|p| u16::from_le_bytes([p[0], p[1]]))
                    .collect::<Vec<_>>(),
            ),
            TextEncoding::Utf16Le,
        );
    }
    if bytes.starts_with(&[254, 255]) {
        return (
            String::from_utf16_lossy(
                &bytes[2..]
                    .chunks_exact(2)
                    .map(|p| u16::from_be_bytes([p[0], p[1]]))
                    .collect::<Vec<_>>(),
            ),
            TextEncoding::Utf16Be,
        );
    }
    if bytes.starts_with(&[239, 187, 191]) {
        return (
            String::from_utf8_lossy(&bytes[3..]).into(),
            TextEncoding::Utf8Bom,
        );
    }
    if let Ok(text) = std::str::from_utf8(bytes) {
        return (text.into(), TextEncoding::Utf8);
    }
    (
        encoding_rs::WINDOWS_1252.decode(bytes).0.into_owned(),
        TextEncoding::Windows1252,
    )
}
/// Read an SPCD spectrum using the same text encoding detection as ZMX files.
pub fn read_spcd(path: &Path) -> Result<raytrace_core::spectrum::Spectrum, String> {
    let bytes = fs::read(path).map_err(|e| e.to_string())?;
    raytrace_core::spectrum::Spectrum::parse_spcd(&decode(&bytes).0)
}
fn encode(text: &str, encoding: &TextEncoding) -> Result<Vec<u8>, String> {
    Ok(match encoding {
        TextEncoding::Utf8 => text.as_bytes().to_vec(),
        TextEncoding::Utf8Bom => [vec![239, 187, 191], text.as_bytes().to_vec()].concat(),
        TextEncoding::Utf16Le => [
            vec![255, 254],
            text.encode_utf16().flat_map(u16::to_le_bytes).collect(),
        ]
        .concat(),
        TextEncoding::Utf16Be => [
            vec![254, 255],
            text.encode_utf16().flat_map(u16::to_be_bytes).collect(),
        ]
        .concat(),
        TextEncoding::Windows1252 => {
            let (bytes, _, errors) = encoding_rs::WINDOWS_1252.encode(text);
            if errors {
                return Err(
                    "元の文字コードで表現できません。JSONプロジェクトに保存してください".into(),
                );
            }
            bytes.into_owned()
        }
    })
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ZmxDocument {
    pub text: String,
    pub encoding: TextEncoding,
    pub source_path: PathBuf,
    pub baseline_scene: Scene,
    /// One source-document surface number per editable sequential surface.
    pub surface_numbers: Vec<usize>,
    pub glass_records: BTreeMap<String, String>,
}
#[derive(Clone, Debug)]
struct Glass {
    errors: Vec<String>,
    name: String,
    formula: usize,
    coefficients: Vec<f64>,
    range: Option<[f64; 2]>,
    transmission: Vec<[f64; 3]>,
    text: String,
}
impl Glass {
    fn material(&self) -> Result<Material, String> {
        if !self.errors.is_empty() {
            return Err(format!("{}: {}", self.name, self.errors.join("; ")));
        }
        let range = self
            .range
            .or_else(|| Some([self.transmission.first()?[0], self.transmission.last()?[0]]))
            .filter(|r| r[0] > 0.0 && r[1] > r[0])
            .ok_or_else(|| format!("{}: AGFの有効波長範囲LDがありません", self.name))?;
        let required = raytrace_core::dispersion::coefficient_count(self.formula)
            .ok_or_else(|| format!("{}: AGF分散式{}は未対応です", self.name, self.formula))?;
        if self.coefficients.len() < required || self.coefficients.iter().any(|v| !v.is_finite()) {
            return Err(format!("{}: AGF係数が不足しています", self.name));
        }
        let c = &self.coefficients;
        let base = match self.formula {
            1 => Material::Schott {
                coefficients: std::array::from_fn(|i| c[i]),
                valid_range_um: range,
                absorption_per_mm: 0.0,
            },
            2 => Material::Sellmeier {
                b: [c[0], c[2], c[4]],
                c_um2: [c[1], c[3], c[5]],
                valid_range_um: range,
                absorption_per_mm: 0.0,
            },
            formula => Material::Catalog {
                formula,
                coefficients: c[..required].to_vec(),
                valid_range_um: range,
                absorption_per_mm: 0.0,
            },
        };
        if self.transmission.is_empty() {
            Ok(base)
        } else {
            if !self.transmission.iter().all(|v| {
                v.iter().all(|v| v.is_finite())
                    && v[0] > 0.
                    && v[2] > 0.
                    && (0.0..=1.0).contains(&v[1])
            }) || !self.transmission.windows(2).all(|w| w[0][0] < w[1][0])
            {
                return Err(format!(
                    "{}: ITの波長順序・透過率・厚さが不正です",
                    self.name
                ));
            }
            let samples = self
                .transmission
                .iter()
                .map(|s| [s[0], -s[1].max(1e-300).ln() / s[2]])
                .collect();
            Ok(Material::WithAbsorption {
                dispersion: Box::new(base),
                samples,
            })
        }
    }
}
fn words(line: &str) -> Vec<&str> {
    line.split_whitespace().collect()
}
fn number(s: &str) -> Result<f64, String> {
    s.replace(['D', 'd'], "E")
        .parse()
        .map_err(|_| format!("数値を読めません: {s}"))
}
fn parameters(lines: &[String], key: &str) -> BTreeMap<usize, f64> {
    lines
        .iter()
        .filter_map(|l| {
            let w = words(l);
            if w.first().copied() != Some(key) || w.len() < 3 {
                return None;
            }
            Some((w[1].parse().ok()?, number(w[2]).ok()?))
        })
        .collect()
}
fn checked_parameters(lines: &[String], key: &str) -> Result<BTreeMap<usize, f64>, String> {
    let mut values = BTreeMap::new();
    for line in lines {
        let w = words(line);
        if w.first().copied() != Some(key) {
            continue;
        }
        if w.len() < 3 {
            return Err(format!("{key}の定義が不足しています"));
        }
        let index = w[1]
            .parse::<usize>()
            .map_err(|_| format!("{key}の番号が不正です"))?;
        let value = number(w[2])?;
        if !value.is_finite() || values.insert(index, value).is_some() {
            return Err(format!("{key}の値が有限でないか、番号が重複しています"));
        }
    }
    Ok(values)
}
fn field<'a>(lines: &'a [String], key: &str) -> Option<&'a str> {
    lines.iter().find_map(|l| {
        let l = l.trim();
        l.strip_prefix(key)
            .filter(|tail| tail.is_empty() || tail.starts_with(char::is_whitespace))
            .map(str::trim)
    })
}
fn scalar(lines: &[String], key: &str, default: f64) -> Result<f64, String> {
    field(lines, key)
        .and_then(|f| f.split_whitespace().next())
        .map(number)
        .unwrap_or(Ok(default))
}

fn catalogs(path: &Path, extra: &[PathBuf], declared: &[String]) -> BTreeMap<String, Glass> {
    let mut dirs = Vec::new();
    if let Some(home) = std::env::var_os("USERPROFILE") {
        dirs.push(PathBuf::from(home).join("Documents/Zemax/Glasscat"));
    }
    if let Some(parent) = path.parent() {
        dirs.push(parent.to_path_buf());
    }
    dirs.extend_from_slice(extra);
    let mut result = BTreeMap::new();
    for dir in dirs {
        let Ok(entries) = fs::read_dir(dir) else {
            continue;
        };
        let mut files: Vec<_> = entries
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|s| s.eq_ignore_ascii_case("agf")))
            .filter(|p| {
                declared.is_empty()
                    || declared.iter().any(|name| {
                        p.file_stem()
                            .is_some_and(|s| s.eq_ignore_ascii_case(name.as_str()))
                    })
            })
            .collect();
        files.sort_by_key(|p| {
            std::cmp::Reverse(
                declared
                    .iter()
                    .position(|name| {
                        p.file_stem()
                            .is_some_and(|s| s.eq_ignore_ascii_case(name.as_str()))
                    })
                    .unwrap_or(0),
            )
        });
        for file in files {
            let Ok(bytes) = fs::read(file) else {
                continue;
            };
            let (text, _) = decode(&bytes);
            let mut current: Option<Glass> = None;
            for line in text.lines() {
                let w = words(line);
                if w.is_empty() {
                    continue;
                }
                if w[0] == "NM" && w.len() > 2 {
                    if let Some(glass) = current.take() {
                        result.insert(glass.name.to_uppercase(), glass);
                    }
                    current = Some(Glass {
                        errors: Vec::new(),
                        name: w[1].into(),
                        formula: number(w[2])
                            .ok()
                            .filter(|v| v.is_finite() && *v > 0. && v.fract() == 0.)
                            .map_or(0, |v| v as usize),
                        coefficients: Vec::new(),
                        range: None,
                        transmission: Vec::new(),
                        text: String::new(),
                    });
                }
                if let Some(glass) = current.as_mut() {
                    glass.text.push_str(line);
                    glass.text.push('\n');
                    let vals: Vec<_> = if ["CD", "LD", "IT"].contains(&w[0]) {
                        match w
                            .iter()
                            .skip(1)
                            .map(|s| number(s))
                            .collect::<Result<Vec<_>, _>>()
                        {
                            Ok(values) => values,
                            Err(e) => {
                                glass.errors.push(e);
                                continue;
                            }
                        }
                    } else {
                        Vec::new()
                    };
                    match w[0] {
                        "CD" => glass.coefficients = vals,
                        "LD" if vals.len() == 2 => glass.range = Some([vals[0], vals[1]]),
                        "IT" if vals.len() == 3 => {
                            glass.transmission.push([vals[0], vals[1], vals[2]])
                        }
                        "LD" | "IT" => glass.errors.push(format!("{}の列数が不正です", w[0])),
                        _ => (),
                    }
                }
            }
            if let Some(glass) = current {
                result.insert(glass.name.to_uppercase(), glass);
            }
        }
    }
    result
}
fn material_id(
    project: &mut Project,
    glasses: &BTreeMap<String, Glass>,
    name: &str,
    records: &mut BTreeMap<String, String>,
) -> usize {
    if name.is_empty() || name.eq_ignore_ascii_case("AIR") {
        return 0;
    }
    if let Some(i) = project
        .material_names
        .iter()
        .position(|n| n.eq_ignore_ascii_case(name))
    {
        return i;
    }
    let material = match glasses.get(&name.to_uppercase()) {
        Some(g) => {
            records.insert(name.into(), g.text.clone());
            match g.material() {
                Ok(m) => m,
                Err(e) => {
                    project.issue("glass", e, true);
                    Material::Constant {
                        n: 1.0,
                        absorption_per_mm: 0.0,
                    }
                }
            }
        }
        None => {
            project.issue("glass",format!("材料{name}のAGFが見つかりません。ZMXと同じフォルダーにAGFを置いて開き直してください"),true);
            Material::Constant {
                n: 1.0,
                absorption_per_mm: 0.0,
            }
        }
    };
    project.material_names.push(name.into());
    project.scene.materials.push(material);
    project.scene.materials.len() - 1
}
fn pose_from(line: &str) -> Result<(Pose, String), String> {
    let w = words(line);
    if w.len() < 6 {
        return Err("NSOPの配置情報が不足しています".into());
    }
    Ok((
        Pose {
            translation_mm: V3([number(w[0])?, number(w[1])?, number(w[2])?]),
            rotation_deg: V3([number(w[3])?, number(w[4])?, number(w[5])?]),
        },
        w.get(6).copied().unwrap_or("").into(),
    ))
}

pub fn read_zmx(path: &Path, extra: &[PathBuf]) -> Result<Project, String> {
    read_zmx_with_cad_tolerance(path, extra, 0.02)
}
pub fn read_zmx_with_cad_tolerance(
    path: &Path,
    extra: &[PathBuf],
    deflection_mm: f64,
) -> Result<Project, String> {
    if !deflection_mm.is_finite() || deflection_mm <= 0.0 {
        return Err("CAD分割精度が不正です".into());
    }
    let (text, encoding) = decode(&fs::read(path).map_err(|e| e.to_string())?);
    let all: Vec<String> = text.lines().map(str::to_owned).collect();
    if field(&all, "UNIT").is_some_and(|s| s.split_whitespace().next() != Some("MM")) {
        return Err("現在はmm単位のZMXに対応しています".into());
    }
    let mode = field(&all, "MODE").unwrap_or("SEQ");
    if !["SEQ", "NSC"].contains(&mode) {
        return Err(format!("未対応のZMX追跡方式です: {mode}"));
    }
    let wavelengths = checked_parameters(&all, "WAVM")?;
    for line in &all {
        let w = words(line);
        if w.first() == Some(&"WAVM")
            && (w.len() < 3
                || w[1].parse::<usize>().ok().is_none_or(|i| i == 0)
                || number(w[2]).ok().is_none_or(|v| !v.is_finite() || v <= 0.0))
        {
            return Err(format!("ZMXの波長定義が不正です: {}", line.trim()));
        }
    }
    let primary_value = scalar(&all, "PWAV", 1.0)?;
    if !primary_value.is_finite() || primary_value < 1.0 || primary_value.fract() != 0.0 {
        return Err("ZMXの主波長番号が不正です".into());
    }
    let primary = primary_value as usize;
    let wavelength = *wavelengths
        .get(&primary)
        .or_else(|| wavelengths.values().next())
        .unwrap_or(&0.55);
    let mut project = Project::new(Scene {
        name: field(&all, "NAME")
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| path.file_stem().and_then(|s| s.to_str()).unwrap_or("Zemax"))
            .into(),
        reference_plane: "発射面の光線重み合計".into(),
        mode: if mode.starts_with("NSC") {
            Mode::NonSequential
        } else {
            Mode::Sequential
        },
        materials: vec![Material::Constant {
            n: 1.0,
            absorption_per_mm: 0.0,
        }],
        surfaces: Vec::new(),
        sources: Vec::new(),
        settings: Settings::default(),
    });
    let declared: Vec<String> = field(&all, "GCAT")
        .unwrap_or("")
        .split_whitespace()
        .map(str::to_owned)
        .collect();
    if !wavelengths.contains_key(&primary) {
        project.issue(
            "spectrum",
            "主波長が波長表にありません。計算波長を明示してください",
            true,
        );
    }
    if let Some(environment) = field(&all, "ENVD") {
        let values: Vec<_> = environment
            .split_whitespace()
            .map(number)
            .collect::<Result<_, _>>()?;
        if values.len() < 2
            || (values[0] - 20.0).abs() > 1e-12
            || (values[1] - 1.0).abs() > 1e-12
            || values.iter().any(|v| !v.is_finite())
            || values.iter().skip(2).any(|v| *v != 0.0)
        {
            project.issue(
                "environment",
                format!("環境条件ENVD {environment}の温度・圧力による屈折率補正は未対応です"),
                true,
            );
        }
    }
    let glasses = catalogs(path, extra, &declared);
    let mut records = BTreeMap::new();
    let mut bindings = Vec::new();
    if mode.starts_with("NSC") {
        read_nsc(
            &all,
            path,
            wavelength,
            &mut project,
            &glasses,
            &mut records,
            deflection_mm,
        )?;
    } else {
        read_sequential(
            &all,
            (path, extra),
            wavelength,
            &mut project,
            &glasses,
            &mut records,
            &mut bindings,
        )?;
    }
    project.zemax = Some(ZmxDocument {
        text,
        encoding,
        source_path: path.into(),
        baseline_scene: project.scene.clone(),
        surface_numbers: bindings,
        glass_records: records,
    });
    project.refresh_assembly()?;
    Ok(project)
}

fn read_coating(
    path: &Path,
    extra: &[PathBuf],
    selected: Option<&str>,
    name: &str,
) -> Result<raytrace_core::coating::Coating, String> {
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
            let bytes = fs::read(&file).map_err(|e| e.to_string())?;
            if let Some(c) = raytrace_core::coating::parse_zemax(&decode(&bytes).0, name)? {
                return Ok(c);
            }
        }
    }
    Err("IDEAL/TABLE定義が見つかりません。コーティングDATをZMXと同じフォルダー、または追加カタログへ配置してください".into())
}

fn read_sequential(
    all: &[String],
    location: (&Path, &[PathBuf]),
    wavelength: f64,
    project: &mut Project,
    glasses: &BTreeMap<String, Glass>,
    records: &mut BTreeMap<String, String>,
    bindings: &mut Vec<usize>,
) -> Result<(), String> {
    let (path, extra) = location;
    let mut blocks: Vec<(usize, Vec<String>)> = Vec::new();
    for line in all {
        if let Some(tail) = line.trim().strip_prefix("SURF ") {
            blocks.push((
                tail.trim().parse().map_err(|_| "SURF番号が不正です")?,
                Vec::new(),
            ));
        } else if let Some((_, lines)) = blocks.last_mut() {
            lines.push(line.clone());
        }
    }
    if blocks.len() < 2 {
        return Err("ZMXに面データがありません".into());
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
        .ok_or("ZMXに光学面がありません")?
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
                return Err("Coordinate Breakの厚さは有限値が必要です".into());
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
            "STANDARD" | "EVENASPH" if curvature != 0.0 => {
                if coeff.get(&1).is_some_and(|v| v.abs() > 0.0) {
                    project.issue(
                        "surface",
                        format!("面{id}: EVENASPHの2次追加項は未対応です"),
                        true,
                    );
                }
                Shape::EvenAsphere {
                    radius_mm: 1.0 / curvature,
                    conic,
                    coefficients: (2..=9).map(|i| *coeff.get(&i).unwrap_or(&0.0)).collect(),
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
                return Err("SQAPの寸法が不足しています".into());
            }
            Aperture::Rectangle {
                half_width_mm: number(w[0])?,
                half_height_mm: number(w[1])?,
            }
        } else if let Some(circle) = field(lines, "CLAP") {
            let w = words(circle);
            if w.len() < 2 {
                return Err("CLAPの寸法が不足しています".into());
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
        .ok_or("ZMXに追跡可能な光学面がありません")?
        .pose
        .translation_mm
        .0[2];
    let spectrum = match read_source_spectrum(path, all, &[], 0.) {
        Ok(Some(s)) if s.samples.len() > 1 => Some(s),
        Ok(_) => None,
        Err(e) => {
            project.issue("spectrum", e, true);
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
            project.issue("illumination_definition", e, true);
        }
    }
    project.issue("illumination","逐次ZMXの波長表・視野の相対重みを使用。光量の基準は瞳補正後の発射光線の重み合計を100%とします。入射瞳、Ray Aiming時はSTOPのXY座標を面積一様に選び、視野の瞳補正を適用します。実開口の外れは光量収支へ記録します",false);
    Ok(())
}

fn read_source_spectrum(
    path: &Path,
    all: &[String],
    lines: &[String],
    wave_index: f64,
) -> Result<Option<raytrace_core::spectrum::Spectrum>, String> {
    let settings = lines.iter().find(|l| {
        let w = words(l);
        w.starts_with(&["NSCS", "7", "3"])
    });
    let parts = settings.map(|l| words(l));
    if parts.as_ref().is_some_and(|w| w.len() < 10) {
        return Err("NSCS 7 3のスペクトル条件が不足しています".into());
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
                return Err("光源の波長番号が不正です".into());
            }
            if wave_index > 0. {
                if !waves.contains_key(&(wave_index as usize)) {
                    return Err("光源の指定波長番号が波長表にありません".into());
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
                .ok_or("SPCDのファイル名が不足しています")?;
            let mut files = vec![path.parent().unwrap_or(Path::new(".")).join(name)];
            if let Some(user) = std::env::var_os("USERPROFILE") {
                files.push(
                    PathBuf::from(user)
                        .join("Documents/Zemax/Objects/Sources/Spectrum Files")
                        .join(name),
                );
            }
            let file = files
                .iter()
                .find(|p| p.is_file())
                .ok_or_else(|| format!("参照スペクトル{name}が見つかりません"))?;
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
                return Err("スペクトルの範囲・点数が不正です".into());
            }
            let mut v: Vec<_> = (0..steps as usize)
                .map(|i| [lo + (hi - lo) * i as f64 / (steps - 1.), 1.])
                .collect();
            if mode == 7. {
                let temperature = number(w[4])?;
                if !temperature.is_finite() || temperature <= 0. {
                    return Err("黒体温度は正の有限値[K]で指定してください".into());
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
            return Err(format!(
                "光源の色・スペクトルモデル{mode}は未対応です。単色波長を明示できます"
            ))
        }
    };
    if samples.is_empty() {
        return Err("光源に波長と重みがありません".into());
    }
    let spectrum = raytrace_core::spectrum::Spectrum {
        samples: std::mem::take(&mut samples),
    };
    spectrum.validate()?;
    Ok(Some(spectrum))
}

fn nsc_reference(value: &str, row: usize) -> Result<usize, String> {
    let v = number(value)?;
    if !v.is_finite() || v.fract() != 0. || v.abs() > 1_000_000. {
        return Err("参照番号は範囲内の整数で指定してください".into());
    }
    let target = if v < 0. {
        row as i64 + v as i64
    } else {
        v as i64
    };
    if target < 0 || target >= row as i64 {
        return Err(
            "参照先はこのオブジェクトより前にある必要があります（前方・自己・循環参照）".into(),
        );
    }
    Ok(target as usize)
}

fn read_nsc(
    all: &[String],
    path: &Path,
    wavelength: f64,
    project: &mut Project,
    glasses: &BTreeMap<String, Glass>,
    records: &mut BTreeMap<String, String>,
    deflection_mm: f64,
) -> Result<(), String> {
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
            return Err("NSOHオブジェクト型がありません".into());
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
                    Err(e) => project.issue("cad", e, true),
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
