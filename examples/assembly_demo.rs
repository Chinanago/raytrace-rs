//! Generate a self-contained source + lens + rod + detector design.
// Tests and examples may panic on failure; the no-panic rule (DESIGN.md P8) covers product code.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
use raytrace_rs::design;
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let path = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "results/assembly_demo.json".into());
    let path = std::path::Path::new(&path);
    if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
        std::fs::create_dir_all(parent)?;
    }
    design::save(&design::demo_design()?, path)?;
    println!("{}", path.display());
    Ok(())
}
