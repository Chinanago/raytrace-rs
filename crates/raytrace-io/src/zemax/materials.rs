use super::*;

#[derive(Clone, Debug)]
pub(super) struct Glass {
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
pub(super) fn catalogs(
    path: &Path,
    extra: &[PathBuf],
    declared: &[String],
) -> BTreeMap<String, Glass> {
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
pub(super) fn material_id(
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
