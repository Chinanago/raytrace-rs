//! Generate a self-contained source + lens + rod + detector project.
use raytrace_rs::assembly;
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let path = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "results/assembly_demo.json".into());
    let path = std::path::Path::new(&path);
    if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
        std::fs::create_dir_all(parent)?;
    }
    assembly::demo_project()?.save(path)?;
    println!("{}", path.display());
    Ok(())
}
