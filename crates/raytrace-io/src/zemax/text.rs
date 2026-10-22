use super::*;

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
pub(super) fn encode(text: &str, encoding: &TextEncoding) -> Result<Vec<u8>, String> {
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

pub(super) fn words(line: &str) -> Vec<&str> {
    line.split_whitespace().collect()
}
pub(super) fn number(s: &str) -> Result<f64, String> {
    s.replace(['D', 'd'], "E")
        .parse()
        .map_err(|_| format!("数値を読めません: {s}"))
}
pub(super) fn parameters(lines: &[String], key: &str) -> BTreeMap<usize, f64> {
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
pub(super) fn checked_parameters(
    lines: &[String],
    key: &str,
) -> Result<BTreeMap<usize, f64>, String> {
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
pub(super) fn field<'a>(lines: &'a [String], key: &str) -> Option<&'a str> {
    lines.iter().find_map(|l| {
        let l = l.trim();
        l.strip_prefix(key)
            .filter(|tail| tail.is_empty() || tail.starts_with(char::is_whitespace))
            .map(str::trim)
    })
}
pub(super) fn scalar(lines: &[String], key: &str, default: f64) -> Result<f64, String> {
    field(lines, key)
        .and_then(|f| f.split_whitespace().next())
        .map(number)
        .unwrap_or(Ok(default))
}

pub(super) fn pose_from(line: &str) -> Result<(Pose, String), String> {
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
