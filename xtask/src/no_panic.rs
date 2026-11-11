//! Clippy's `panic` lint does not cover assertion macros. Inspect Rust syntax so
//! comments/strings are ignored and assertions remain available in tests.
use std::{
    fs,
    path::{Path, PathBuf},
};
use syn::visit::{self, Visit};

#[derive(Default)]
struct Assertions {
    macros: Vec<String>,
    directory: PathBuf,
    modules: Vec<(PathBuf, PathBuf)>,
}

fn test_only(attrs: &[syn::Attribute]) -> bool {
    attrs.iter().any(|a| {
        a.path().is_ident("test")
            || (a.path().is_ident("cfg")
                && a.parse_args::<syn::Path>()
                    .is_ok_and(|p| p.is_ident("test")))
    })
}

impl<'ast> Visit<'ast> for Assertions {
    fn visit_item_mod(&mut self, item: &'ast syn::ItemMod) {
        if test_only(&item.attrs) {
            return;
        }
        let directory = self.directory.join(item.ident.to_string());
        if item.content.is_none() {
            let explicit = item.attrs.iter().find_map(|attr| {
                if !attr.path().is_ident("path") {
                    return None;
                }
                let syn::Meta::NameValue(value) = &attr.meta else {
                    return None;
                };
                let syn::Expr::Lit(value) = &value.value else {
                    return None;
                };
                let syn::Lit::Str(value) = &value.lit else {
                    return None;
                };
                Some(self.directory.join(value.value()))
            });
            let plain = directory.with_extension("rs");
            let file = explicit.unwrap_or_else(|| {
                if plain.is_file() {
                    plain
                } else {
                    directory.join("mod.rs")
                }
            });
            self.modules.push((file, directory));
        } else {
            let previous = std::mem::replace(&mut self.directory, directory);
            visit::visit_item_mod(self, item);
            self.directory = previous;
        }
    }

    fn visit_item_fn(&mut self, item: &'ast syn::ItemFn) {
        if !test_only(&item.attrs) {
            visit::visit_item_fn(self, item);
        }
    }

    fn visit_macro(&mut self, item: &'ast syn::Macro) {
        if let Some(name) = item.path.segments.last().map(|s| s.ident.to_string())
            && [
                "assert",
                "assert_eq",
                "assert_ne",
                "debug_assert",
                "debug_assert_eq",
                "debug_assert_ne",
            ]
            .contains(&name.as_str())
        {
            self.macros.push(name);
        }
        visit::visit_macro(self, item);
    }
}

pub fn check(root: &Path) -> Result<String, Vec<String>> {
    let mut problems = Vec::new();
    let mut count = 0;
    let mut pending = Vec::new();
    for dir in ["crates", "src"] {
        for file in super::rust_files(&root.join(dir)) {
            if file.parent().is_some_and(|p| p.ends_with("src"))
                && file
                    .file_name()
                    .is_some_and(|n| n == "lib.rs" || n == "main.rs")
            {
                pending.push((file.clone(), file.parent().unwrap_or(root).to_path_buf()));
            }
        }
    }
    let mut seen = std::collections::BTreeSet::new();
    while let Some((file, directory)) = pending.pop() {
        if !seen.insert(file.clone()) {
            continue;
        }
        count += 1;
        match fs::read_to_string(&file)
            .map_err(|e| e.to_string())
            .and_then(|text| syn::parse_file(&text).map_err(|e| e.to_string()))
        {
            Ok(syntax) => {
                let mut assertions = Assertions {
                    directory,
                    ..Default::default()
                };
                assertions.visit_file(&syntax);
                for name in assertions.macros {
                    problems.push(format!(
                        "{}: {name}! may panic in product code; return an error instead",
                        file.display()
                    ));
                }
                pending.extend(assertions.modules);
            }
            Err(e) => problems.push(format!("{}: {e}", file.display())),
        }
    }
    if problems.is_empty() {
        Ok(format!(
            "no-panic: {count} product files contain no assertion macros"
        ))
    } else {
        Err(problems)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_assertions_without_matching_comments_strings_or_tests() {
        let syntax = syn::parse_file(
            r#"
            // assert!(false);
            fn product() {
                let _ = "assert!(false)";
                assert!(true); std::assert_eq!(1, 1); debug_assert_ne!(1, 2);
            }
            #[cfg(test)] mod tests { fn helper() { assert!(true); } }
            #[test] fn unit_test() { assert!(true); }
        "#,
        )
        .unwrap();
        let mut found = Assertions::default();
        found.visit_file(&syntax);
        assert_eq!(found.macros, ["assert", "assert_eq", "debug_assert_ne"]);
    }
}
