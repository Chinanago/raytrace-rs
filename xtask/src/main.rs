//! Development checks that keep the architecture of docs/DESIGN.md honest.
//!
//! - `cargo xtask layers`: workspace crates depend only on the same or lower layers, GUI
//!   libraries stay in L5, and the computation core uses no files, clocks, environment,
//!   network or processes.
//! - `cargo xtask wasm`: every crate of L0-L4 builds for `wasm32-unknown-unknown`.
use serde_json::Value;
use std::{
    env, fs,
    path::{Path, PathBuf},
    process::{Command, ExitCode},
};

/// Layer of every workspace crate (docs/DESIGN.md §4). L0 and L1 share `raytrace-core`.
const LAYERS: &[(&str, u8)] = &[
    ("raytrace-core", 1),
    ("raytrace-doc", 2),
    ("raytrace-formats", 3),
    ("raytrace-io", 3),
    ("raytrace-app", 4),
    // Library facade that re-exports L1-L4.
    ("raytrace-rs", 4),
    ("raytrace-cli", 5),
    ("raytrace-gui", 5),
];

/// Development tools outside the product layers.
const TOOLS: &[&str] = &["xtask"];

/// External crates that only the front-end layer (L5) may use.
const FRONT_END_ONLY: &[&str] = &[
    "eframe",
    "egui",
    "egui-wgpu",
    "egui-winit",
    "egui_glow",
    "glow",
    "rfd",
    "wgpu",
    "winit",
];

/// Crates that must stay pure, with the `std` modules they may not use.
const PURE: &[(&str, &[&str])] = &[(
    "crates/raytrace-core/src",
    &["fs", "time", "env", "net", "process"],
)];

/// Highest layer that must build for WebAssembly.
const WASM_LAYER: u8 = 4;
const WASM_TARGET: &str = "wasm32-unknown-unknown";

fn main() -> ExitCode {
    let task = env::args().nth(1);
    let result = match task.as_deref() {
        Some("layers") => layers(),
        Some("wasm") => wasm(),
        _ => Err(vec![
            "usage: cargo xtask layers | cargo xtask wasm (see docs/DESIGN.md)".into(),
        ]),
    };
    match result {
        Ok(summary) => {
            println!("{summary}");
            ExitCode::SUCCESS
        }
        Err(problems) => {
            for problem in problems {
                eprintln!("{problem}");
            }
            ExitCode::FAILURE
        }
    }
}

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."))
}

fn cargo() -> Command {
    let mut command = Command::new(env::var("CARGO").unwrap_or_else(|_| "cargo".into()));
    command.current_dir(root());
    command
}

fn layer_of(name: &str) -> Option<u8> {
    LAYERS.iter().find(|(n, _)| *n == name).map(|(_, l)| *l)
}

fn metadata() -> Result<Value, Vec<String>> {
    let output = cargo()
        .args(["metadata", "--format-version", "1", "--no-deps", "--locked"])
        .output()
        .map_err(|e| vec![format!("cargo metadata failed to start: {e}")])?;
    if !output.status.success() {
        return Err(vec![String::from_utf8_lossy(&output.stderr).into_owned()]);
    }
    serde_json::from_slice(&output.stdout).map_err(|e| vec![format!("cargo metadata: {e}")])
}

fn layers() -> Result<String, Vec<String>> {
    let metadata = metadata()?;
    let packages = metadata["packages"].as_array().cloned().unwrap_or_default();
    let workspace: Vec<&str> = packages.iter().filter_map(|p| p["name"].as_str()).collect();
    let mut problems = Vec::new();
    for package in &packages {
        let name = package["name"].as_str().unwrap_or_default();
        if TOOLS.contains(&name) {
            continue;
        }
        let Some(layer) = layer_of(name) else {
            problems.push(format!(
                "{name}: no layer assigned; add it to LAYERS in xtask/src/main.rs and docs/DESIGN.md §4"
            ));
            continue;
        };
        for dependency in package["dependencies"].as_array().into_iter().flatten() {
            // Tests and benchmarks may use anything.
            if dependency["kind"] == "dev" {
                continue;
            }
            let target = dependency["name"].as_str().unwrap_or_default();
            if workspace.contains(&target) {
                match layer_of(target) {
                    Some(lower) if lower <= layer => {}
                    Some(higher) => problems.push(format!(
                        "{name} (L{layer}) depends on {target} (L{higher}): dependencies must point to the same or a lower layer"
                    )),
                    None => problems.push(format!("{name} depends on the tool crate {target}")),
                }
            } else if layer < 5 && FRONT_END_ONLY.contains(&target) {
                problems.push(format!(
                    "{name} (L{layer}) depends on {target}: GUI libraries belong to the L5 front ends"
                ));
            }
        }
    }
    let mut files = 0;
    for (dir, banned) in PURE {
        for file in rust_files(&root().join(dir)) {
            files += 1;
            let text = fs::read_to_string(&file).unwrap_or_default();
            for (line, module) in std_modules(&text) {
                if banned.contains(&module.as_str()) {
                    problems.push(format!(
                        "{}:{line}: std::{module} is not allowed in the pure computation core",
                        file.strip_prefix(root()).unwrap_or(&file).display()
                    ));
                }
            }
        }
    }
    if problems.is_empty() {
        Ok(format!(
            "layers: {} crates follow the layer order; {files} core files are pure",
            packages.len()
        ))
    } else {
        Err(problems)
    }
}

fn rust_files(dir: &Path) -> Vec<PathBuf> {
    let mut found = Vec::new();
    let mut pending = vec![dir.to_path_buf()];
    while let Some(dir) = pending.pop() {
        for entry in fs::read_dir(&dir).into_iter().flatten().flatten() {
            let path = entry.path();
            if path.is_dir() {
                pending.push(path);
            } else if path.extension().is_some_and(|e| e == "rs") {
                found.push(path);
            }
        }
    }
    found.sort();
    found
}

/// `(line, module)` for every `std::module` path and each item of a `use std::{a, b::c}`
/// group (only the first segment of each item). Line and doc comments are ignored.
fn std_modules(text: &str) -> Vec<(usize, String)> {
    let code: Vec<(usize, char)> = text
        .lines()
        .enumerate()
        .flat_map(|(index, line)| {
            let line = line.split("//").next().unwrap_or_default();
            line.chars().chain(['\n']).map(move |c| (index + 1, c))
        })
        .collect();
    let ident = |c: char| c.is_alphanumeric() || c == '_';
    let mut found = Vec::new();
    let mut i = 0;
    while i + 5 <= code.len() {
        let window: String = code[i..i + 5].iter().map(|(_, c)| c).collect();
        let starts_word = i == 0 || !ident(code[i - 1].1);
        if window != "std::" || !starts_word {
            i += 1;
            continue;
        }
        i += 5;
        if code.get(i).is_some_and(|(_, c)| *c == '{') {
            // A group: take the first segment of every comma-separated item at depth 1.
            let (mut depth, mut take, mut name, mut line) = (1, true, String::new(), 0);
            i += 1;
            while let Some(&(number, c)) = code.get(i) {
                i += 1;
                // An identifier ends before the character that follows it is interpreted.
                if !name.is_empty() && !ident(c) {
                    found.push((line, std::mem::take(&mut name)));
                    take = false;
                }
                match c {
                    '{' => depth += 1,
                    '}' => depth -= 1,
                    ',' if depth == 1 => take = true,
                    _ => {}
                }
                if depth == 1 && take && ident(c) {
                    if name.is_empty() {
                        line = number;
                    }
                    name.push(c);
                }
                if depth == 0 {
                    break;
                }
            }
        } else {
            let line = code.get(i).map_or(0, |(n, _)| *n);
            let name: String = code[i..]
                .iter()
                .map(|(_, c)| *c)
                .take_while(|c| ident(*c))
                .collect();
            if !name.is_empty() {
                found.push((line, name));
            }
        }
    }
    found
}

fn wasm() -> Result<String, Vec<String>> {
    let crates: Vec<&str> = LAYERS
        .iter()
        .filter(|(_, layer)| *layer <= WASM_LAYER)
        .map(|(name, _)| *name)
        .collect();
    let mut command = cargo();
    command.args(["check", "--locked", "--target", WASM_TARGET]);
    for name in &crates {
        command.args(["-p", name]);
    }
    let status = command
        .status()
        .map_err(|e| vec![format!("cargo check failed to start: {e}")])?;
    if status.success() {
        Ok(format!(
            "wasm: {} builds for {WASM_TARGET}",
            crates.join(", ")
        ))
    } else {
        Err(vec![format!(
            "L0-L{WASM_LAYER} must build for {WASM_TARGET} (install it with `rustup target add {WASM_TARGET}`)"
        )])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn std_modules_reads_paths_and_use_groups() {
        let text = "use std::{\n    fs,\n    sync::Arc,\n};\nlet t = std::time::Instant::now(); // std::net in a comment\nuse std::{env, io::{self, Read}};\n";
        let modules: Vec<String> = std_modules(text).into_iter().map(|(_, m)| m).collect();
        assert_eq!(modules, ["fs", "sync", "time", "env", "io"]);
    }

    #[test]
    fn every_layer_is_known() {
        for (name, layer) in LAYERS {
            assert!(*layer <= 5, "{name}");
        }
        assert_eq!(layer_of("raytrace-gui"), Some(5));
    }
}
