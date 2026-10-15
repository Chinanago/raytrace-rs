//! Native ZMX/AGF parsing. Unknown data stays in the original document.
use crate::{
    math::{Pose, V3},
    model::*,
    project::Project,
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
                    .as_chunks::<2>()
                    .0
                    .iter()
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
                    .as_chunks::<2>()
                    .0
                    .iter()
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
    name: String,
    formula: usize,
    coefficients: Vec<f64>,
    range: Option<[f64; 2]>,
    transmission: Vec<[f64; 3]>,
    text: String,
}
impl Glass {
    fn material(&self) -> Result<Material, String> {
        let range = self
            .range
            .or_else(|| Some([self.transmission.first()?[0], self.transmission.last()?[0]]))
            .filter(|r| r[0] > 0.0 && r[1] > r[0])
            .ok_or_else(|| format!("{}: AGFの有効波長範囲LDがありません", self.name))?;
        if self.coefficients.len() < 6 {
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
            n => return Err(format!("{}: AGF分散式{n}は未対応です", self.name)),
        };
        if self.transmission.is_empty() {
            Ok(base)
        } else {
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
                        name: w[1].into(),
                        formula: w[2].parse().unwrap_or(0),
                        coefficients: Vec::new(),
                        range: None,
                        transmission: Vec::new(),
                        text: String::new(),
                    });
                }
                if let Some(glass) = current.as_mut() {
                    glass.text.push_str(line);
                    glass.text.push('\n');
                    let vals: Vec<_> = w.iter().skip(1).filter_map(|s| number(s).ok()).collect();
                    match w[0] {
                        "CD" => glass.coefficients = vals,
                        "LD" if vals.len() >= 2 => glass.range = Some([vals[0], vals[1]]),
                        "IT" if vals.len() >= 3
                            && vals[2] > 0.0
                            && (0.0..=1.0).contains(&vals[1]) =>
                        {
                            glass.transmission.push([vals[0], vals[1], vals[2]])
                        }
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
    let (text, encoding) = decode(&fs::read(path).map_err(|e| e.to_string())?);
    let all: Vec<String> = text.lines().map(str::to_owned).collect();
    if field(&all, "UNIT").is_some_and(|s| s.split_whitespace().next() != Some("MM")) {
        return Err("現在はmm単位のZMXに対応しています".into());
    }
    let mode = field(&all, "MODE").unwrap_or("SEQ");
    let wavelengths = parameters(&all, "WAVM");
    let primary = scalar(&all, "PWAV", 1.0)? as usize;
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
    let glasses = catalogs(path, extra, &declared);
    let mut records = BTreeMap::new();
    let mut bindings = Vec::new();
    if mode.starts_with("NSC") {
        read_nsc(&all, path, wavelength, &mut project, &glasses, &mut records)?;
    } else {
        read_sequential(
            &all,
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
    Ok(project)
}

fn read_sequential(
    all: &[String],
    wavelength: f64,
    project: &mut Project,
    glasses: &BTreeMap<String, Glass>,
    records: &mut BTreeMap<String, String>,
    bindings: &mut Vec<usize>,
) -> Result<(), String> {
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
    let mut z = 0.0;
    let mut medium = 0;
    let last = blocks.last().unwrap().0;
    for (id, lines) in blocks.iter().skip(1) {
        let typ = field(lines, "TYPE")
            .unwrap_or("STANDARD")
            .split_whitespace()
            .next()
            .unwrap_or("");
        let curvature = scalar(lines, "CURV", 0.0)?;
        let conic = scalar(lines, "CONI", 0.0)?;
        let coeff = parameters(lines, "PARM");
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
        let after = material_id(project, glasses, glass, records);
        let interaction = if *id == last {
            Interaction::Detector { pixels: [128, 128] }
        } else {
            Interaction::Dielectric {
                minus_medium: medium,
                plus_medium: after,
                reflectance: None,
            }
        };
        if let Some(coat) = field(lines, "COAT").filter(|s| !s.is_empty() && *s != "_NONE_") {
            project.issue("coating",format!("面{id}: コーティング{coat}は元ファイルに保持します。未コートFresnelでの計算には下の明示設定が必要です"),true);
        }
        for line in lines {
            let w = words(line);
            if w.is_empty() {
                continue;
            }
            if line.starts_with(char::is_whitespace)
                && ![
                    "TYPE", "COMM", "STOP", "FIMP", "CURV", "CONI", "PARM", "DISZ", "GLAS", "DIAM",
                    "SQAP", "CLAP", "COAT", "HIDE", "MIRR", "SLAB", "POPS", "FLAP", "MEMA", "SSID",
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
            name: format!("S{id} {}", field(lines, "COMM").unwrap_or(typ)),
            pose: Pose {
                translation_mm: V3([0.0, 0.0, z]),
                ..Pose::default()
            },
            shape,
            aperture,
            interaction,
        });
        bindings.push(*id);
        let distance = scalar(lines, "DISZ", 0.0)?;
        if !distance.is_finite() {
            project.issue("surface", format!("面{id}: 無限の面間隔は未対応です"), true);
        } else {
            z += distance;
        }
        medium = after;
    }
    let first = project
        .scene
        .surfaces
        .first()
        .unwrap()
        .pose
        .translation_mm
        .0[2];
    project.scene.sources.push(Source {
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
    });
    project.issue("illumination","逐次ZMXの形状・材料を読込。照射条件は主波長の軸上平行光です。視野・瞳の設定は照射条件で調整してください",false);
    Ok(())
}

fn read_nsc(
    all: &[String],
    path: &Path,
    wavelength: f64,
    project: &mut Project,
    glasses: &BTreeMap<String, Glass>,
    records: &mut BTreeMap<String, String>,
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
    for (i, lines) in objects.iter().enumerate() {
        let head = words(field(lines, "NSOH").unwrap_or(""));
        if head.is_empty() {
            continue;
        }
        let typ = head[0];
        let reference = head
            .get(3..)
            .unwrap_or(&[])
            .join(" ")
            .trim_matches('"')
            .to_owned();
        let (pose, glass) = pose_from(field(lines, "NSOP").unwrap_or("0 0 0 0 0 0"))?;
        let p = parameters(lines, "NSOD");
        let param = |k, default| *p.get(&k).unwrap_or(&default);
        let label = format!("O{} {typ}", i + 1);
        if let Some(ns) = field(lines, "NSOH") {
            let h = words(ns);
            if h.get(1..3).is_some_and(|v| v.iter().any(|s| *s != "0")) {
                project.issue(
                    "reference",
                    format!("{label}: 参照オブジェクトに対する配置は未対応です"),
                    true,
                );
            }
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
                if [8, 10, 17, 20]
                    .iter()
                    .any(|k| (param(*k, 1.0) - 1.0).abs() > 1e-12)
                    || param(6, 0.0) != 0.0
                {
                    project.issue(
                        "source",
                        format!("{label}: 非点収差または超Gaussian係数は未対応です"),
                        true,
                    );
                }
                let frame = crate::math::Frame::new(&pose);
                let waves = parameters(all, "WAVM");
                let source_wave = *waves.get(&(param(4, 1.0) as usize)).unwrap_or(&wavelength);
                project.scene.sources.push(Source {
                    sampling: Sampling::Sobol,
                    count: param(2, 1000.0).max(1.0) as usize,
                    origin_mm: pose.translation_mm,
                    direction: frame.world_vector(V3([0.0, 0.0, 1.0])),
                    disk_radius_mm: 0.0,
                    cone_half_angle_deg: 0.0,
                    wavelength_um: source_wave,
                    medium: 0,
                    total_weight: param(3, 1.0),
                    seed: 42 + i as u64,
                    diode: Some(Diode {
                        rotation_deg: pose.rotation_deg,
                        divergence_deg: [param(7, 1.0), param(9, 1.0)],
                        array: [param(11, 1.0) as usize, param(12, 1.0) as usize],
                        spacing_mm: [param(13, 0.0), param(14, 0.0)],
                        half_width_mm: [param(15, 0.0), param(18, 0.0)],
                        gaussian_width_mm: [param(16, 0.0), param(19, 0.0)],
                    }),
                });
                for line in lines {
                    if line.contains(".SPCD") || line.contains(".spcd") {
                        let name = line
                            .split('"')
                            .find(|s| s.to_uppercase().ends_with(".SPCD"))
                            .unwrap_or("SPCD");
                        let exists = path.parent().unwrap_or(Path::new(".")).join(name).is_file();
                        project.issue("spectrum",format!("参照スペクトル {name} ({})。現段階の計算は単色設定を明示してください",if exists{"ファイルあり・分布読込は未対応"}else{"ファイルが不足しています"}),true);
                        break;
                    }
                }
            }
            "NSC_DETC" => project.scene.surfaces.push(Surface {
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
                let medium = material_id(project, glasses, &glass, records);
                if param(1, 1.0) != 1.0 || param(2, 1.0) != 1.0 {
                    project.issue(
                        "cad",
                        format!("{label}: CADのスケールまたは体積設定が未対応です"),
                        true,
                    );
                }
                let step = path.parent().unwrap_or(Path::new(".")).join(&reference);
                match crate::cad::load_step(&step) {
                    Ok(shape) => project.scene.surfaces.push(Surface {
                        name: reference,
                        pose,
                        shape,
                        aperture: Aperture::Rectangle {
                            half_width_mm: 1e8,
                            half_height_mm: 1e8,
                        },
                        interaction: Interaction::Dielectric {
                            minus_medium: medium,
                            plus_medium: 0,
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
        if lines.iter().any(|l| {
            let w = words(l);
            w.len() > 3 && w[0] == "NSCS" && w[1] == "1" && w.last().is_some_and(|s| *s != "_NONE_")
        }) {
            project.issue(
                "coating",
                format!("{label}: 面コーティングの追跡は未対応です"),
                true,
            );
        }
    }
    if project
        .scene
        .surfaces
        .iter()
        .any(|s| matches!(s.shape, Shape::TriangleMesh { .. }))
    {
        project.issue("cad_accuracy","STEPはFreeCADで初回に0.02 mm設定で分割し、頂点法線を補間します。メッシュを変えたときの結果の収束が検証対象です",false);
    }
    Ok(())
}

/// Preserve the complete source text on an unchanged document. Geometry edits
/// are patched only for supported coaxial sequential surfaces.
pub fn export(project: &Project, path: &Path) -> Result<(), String> {
    if let Some(original) = &project.zemax {
        if serde_json::to_value(&project.scene).map_err(|e| e.to_string())?
            == serde_json::to_value(&original.baseline_scene).map_err(|e| e.to_string())?
        {
            copy_dependencies(original, path)?;
            return fs::write(path, encode(&original.text, &original.encoding)?)
                .map_err(|e| e.to_string());
        }
    }
    let scene = &project.scene;
    if scene.mode == Mode::NonSequential {
        if let Some(original) = &project.zemax {
            let text = patch_nsc(project, original)?;
            copy_dependencies(original, path)?;
            return fs::write(path, encode(&text, &original.encoding)?).map_err(|e| e.to_string());
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
    if original.is_none()
        && scene.sources.iter().any(|s| {
            s.diode.is_some()
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
    let catalog = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("RAYTRACE")
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect::<String>();
    let mut agf = String::from("CC Raytrace material exchange\n");
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
        let (formula, coeff, range) = match material {
            Material::Constant {
                n,
                absorption_per_mm,
            } if *absorption_per_mm == 0.0 => {
                (1, vec![n * n, 0.0, 0.0, 0.0, 0.0, 0.0], [0.001, 1000.0])
            }
            Material::Schott {
                coefficients,
                valid_range_um,
                absorption_per_mm,
            } if *absorption_per_mm == 0.0 => (1, coefficients.to_vec(), *valid_range_um),
            Material::Sellmeier {
                b,
                c_um2,
                valid_range_um,
                absorption_per_mm,
            } if *absorption_per_mm == 0.0 => (
                2,
                vec![b[0], c_um2[0], b[1], c_um2[1], b[2], c_um2[2]],
                *valid_range_um,
            ),
            _ => return Err("編集した吸収表・表形式材料のZemax書出しは未対応です".into()),
        };
        agf.push_str(&format!(
            "NM {name} {formula} 0 1.5 50 0 0\nCD {}\nLD {} {}\n",
            coeff
                .iter()
                .map(|v| format!("{v:.17e}"))
                .collect::<Vec<_>>()
                .join(" "),
            range[0],
            range[1]
        ));
    }
    let mut replacements: BTreeMap<usize, Vec<String>> = BTreeMap::new();
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
        if gap < 0.0 {
            return Err("ZMX書出しには面位置を昇順にしてください".into());
        }
        let mut lines = vec![
            format!("SURF {id}"),
            format!("  COMM {}", s.name.replace(['\r', '\n'], " ")),
            format!("  TYPE {typ}"),
            format!("  CURV {curvature:.17e} 0 0 0 0 \"\""),
            format!("  CONI {conic:.17e}"),
            format!("  DISZ {gap:.17e}"),
            format!("  DIAM {semi:.17e} 0 0 0 1 \"\""),
        ];
        match s.interaction {
            Interaction::Dielectric {
                plus_medium,
                reflectance: None,
                ..
            } => {
                if plus_medium != 0 {
                    lines.push(format!("  GLAS {} 0 0 1.5 50 0 0", names[plus_medium]));
                }
            }
            Interaction::Detector { .. } if i + 1 == scene.surfaces.len() => (),
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
        let waves: Vec<_> = scene.sources.iter().map(|s| s.wavelength_um).collect();
        let pupil = scene
            .sources
            .first()
            .map_or(1.0, |s| 2.0 * s.disk_radius_mm.max(0.01));
        let mut head=format!("VERS 260000 0 0\r\nMODE SEQ\r\nNAME {}\r\nUNIT MM X W X CM MR CPMM\r\nENPD {pupil}\r\nENVD 20 1 0\r\nGCAT {catalog}\r\nFTYP 0 0 1 1 0 0 0\r\nXFLN 0\r\nYFLN 0\r\nPWAV 1\r\n",scene.name.replace(['\r','\n']," "));
        for (i, w) in waves.iter().enumerate() {
            head.push_str(&format!("WAVM {} {w:.17e} 1\r\n", i + 1));
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
    if let Some(original) = original {
        copy_dependencies(original, path)?;
    }
    if custom_catalog {
        fs::write(path.with_extension("AGF"), agf).map_err(|e| e.to_string())?;
    }
    fs::write(
        path,
        encode(
            &text,
            original.map_or(&TextEncoding::Utf16Le, |z| &z.encoding),
        )?,
    )
    .map_err(|e| e.to_string())
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

fn patch_nsc(project: &Project, original: &ZmxDocument) -> Result<String, String> {
    let a = &project.scene;
    let b = &original.baseline_scene;
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
    if a.sources
        .iter()
        .zip(&b.sources)
        .any(|(s, t)| s.wavelength_um != t.wavelength_um)
    {
        return Err("NSCの波長設定の変更はJSONプロジェクトに保存してください。ZMXの波長表更新は追加予定です".into());
    }
    let mut source_index = 0;
    let mut current_source = None;
    let mut current_surface = None;
    let mut object_index = 0usize;
    let mut lines = Vec::new();
    let values: Vec<BTreeMap<usize, f64>> = a
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
                (7, d.divergence_deg[0]),
                (9, d.divergence_deg[1]),
                (11, d.array[0] as f64),
                (12, d.array[1] as f64),
                (13, d.spacing_mm[0]),
                (14, d.spacing_mm[1]),
                (15, d.half_width_mm[0]),
                (16, d.gaussian_width_mm[0]),
                (18, d.half_width_mm[1]),
                (19, d.gaussian_width_mm[1]),
            ]))
        })
        .collect::<Result<_, String>>()?;
    let mut seen = std::collections::BTreeSet::new();
    for line in original.text.lines() {
        let w = words(line);
        if matches!(w.first(), Some(&"NSOH") | Some(&"SURF")) {
            append_missing_parameters(&mut lines, current_source, &seen, &values);
            seen.clear();
            current_source = None;
            current_surface = None;
        }
        if w.first() == Some(&"NSOH") {
            object_index += 1;
            current_source = None;
            current_surface = None;
            if w.get(1) == Some(&"NSC_SDIO") {
                current_source = Some(source_index);
                source_index += 1;
            } else if w.get(1) == Some(&"NSC_IMPT") {
                let name = w.get(4..).unwrap_or(&[]).join(" ");
                current_surface = a.surfaces.iter().position(|s| s.name == name);
            } else if w.get(1) == Some(&"NSC_DETC") {
                let name = format!("O{object_index} NSC_DETC");
                current_surface = a.surfaces.iter().position(|s| s.name == name);
            }
        }
        if w.first() == Some(&"NSOP") {
            let pose = if let Some(i) = current_source {
                let s = &a.sources[i];
                Some(Pose {
                    translation_mm: s.origin_mm,
                    rotation_deg: s.diode.as_ref().map_or(V3::default(), |d| d.rotation_deg),
                })
            } else {
                current_surface.map(|i| a.surfaces[i].pose.clone())
            };
            if let Some(pose) = pose {
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
