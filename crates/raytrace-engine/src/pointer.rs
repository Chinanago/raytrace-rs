//! JSON Pointers into a design, with `@<id>` to address an array element by its `id` field.
use crate::engine::EngineError;
use serde_json::Value;

/// Split a pointer (`""` or `/a/b`) into unescaped segments (`~1` is `/`, `~0` is `~`).
pub(crate) fn parse(pointer: &str) -> Result<Vec<String>, EngineError> {
    if pointer.is_empty() {
        return Ok(Vec::new());
    }
    let Some(rest) = pointer.strip_prefix('/') else {
        return Err(EngineError::invalid(format!(
            "パス {pointer} は / で始まるJSON Pointerで指定してください（例: /occurrences/@rod-4/pose/translation_mm/2）"
        )));
    };
    Ok(rest
        .split('/')
        .map(|s| s.replace("~1", "/").replace("~0", "~"))
        .collect())
}

fn escape(segment: &str) -> String {
    segment.replace('~', "~0").replace('/', "~1")
}

/// Find the value at `segments`. A missing last member of an object is created as `null` when
/// `create_last` is set. Returns the value and the pointer with every `@<id>` replaced by the
/// index of the element.
pub(crate) fn resolve<'a>(
    root: &'a mut Value,
    segments: &[String],
    create_last: bool,
) -> Result<(&'a mut Value, String), EngineError> {
    let mut current = root;
    let mut resolved = String::new();
    for (i, segment) in segments.iter().enumerate() {
        let last = i + 1 == segments.len();
        let here = if resolved.is_empty() {
            "文書".to_string()
        } else {
            resolved.clone()
        };
        current = match current {
            Value::Object(members) => {
                if !members.contains_key(segment) {
                    if last && create_last {
                        members.insert(segment.clone(), Value::Null);
                    } else {
                        return Err(EngineError::invalid(format!(
                            "{here} に項目 {segment} がありません（doc.get で構造を確認してください）"
                        )));
                    }
                }
                resolved.push('/');
                resolved.push_str(&escape(segment));
                members.get_mut(segment).ok_or_else(|| {
                    EngineError::invalid(format!("項目 {segment} を取得できません"))
                })?
            }
            Value::Array(items) => {
                let index = match segment.strip_prefix('@') {
                    Some(id) => items
                        .iter()
                        .position(|item| item.get("id").and_then(Value::as_str) == Some(id))
                        .ok_or_else(|| {
                            EngineError::invalid(format!("{here} に id が {id} の要素がありません"))
                        })?,
                    None => segment.parse::<usize>().map_err(|_| {
                        EngineError::invalid(format!(
                            "{here} は配列です。{segment} は添字（0から）か @<id> で指定してください"
                        ))
                    })?,
                };
                let length = items.len();
                resolved.push('/');
                resolved.push_str(&index.to_string());
                items.get_mut(index).ok_or_else(|| {
                    EngineError::invalid(format!(
                        "{here} の添字 {index} は範囲外です（要素数 {length}）"
                    ))
                })?
            }
            _ => {
                return Err(EngineError::invalid(format!(
                    "{here} は値であり、その中の {segment} は指定できません"
                )));
            }
        };
    }
    Ok((current, resolved))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn ids_and_indices_address_array_elements() {
        let mut doc = json!({"a": [{"id": "x-1", "v": [1, 2]}, {"id": "x-2", "v": [3]}], "b/c": 1});
        let segments = parse("/a/@x-2/v/0").unwrap();
        let (value, resolved) = resolve(&mut doc, &segments, false).unwrap();
        assert_eq!(
            (value.clone(), resolved),
            (json!(3), "/a/1/v/0".to_string())
        );
        let (value, _) = resolve(&mut doc, &parse("/a/0/v/1").unwrap(), false).unwrap();
        assert_eq!(*value, json!(2));
        let (value, resolved) = resolve(&mut doc, &parse("/b~1c").unwrap(), false).unwrap();
        assert_eq!((value.clone(), resolved), (json!(1), "/b~1c".to_string()));
    }

    #[test]
    fn bad_pointers_are_invalid_requests() {
        let mut doc = json!({"a": [{"id": "x-1"}], "s": 1});
        for bad in ["a", "/a/@nope", "/a/7", "/a/z", "/missing", "/s/x"] {
            let error = parse(bad)
                .and_then(|s| resolve(&mut doc, &s, false).map(|_| ()))
                .unwrap_err();
            assert_eq!(error.code, "invalid_request", "{bad}");
        }
        let (value, _) = resolve(&mut doc, &parse("/created").unwrap(), true).unwrap();
        assert!(value.is_null());
    }
}
