//! Generate owned fixture data and local validation results; never vendor files.
use raytrace_rs::validation;
use std::{fs, path::PathBuf};
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let out = PathBuf::from(
        std::env::args()
            .nth(1)
            .unwrap_or_else(|| "results/validation".into()),
    );
    fs::create_dir_all(&out)?;
    let mut cases = Vec::new();
    for case in validation::cases() {
        let result = case.run(&out.join(case.name))?;
        println!(
            "{}: {} local checks; OpticStudio verification pending",
            case.name,
            result.checks.len()
        );
        cases.push(result);
    }
    let failed = cases.iter().flat_map(|c| &c.checks).any(|c| !c.passed);
    fs::write(
        out.join("suite.json"),
        serde_json::to_vec_pretty(
            &serde_json::json!({"format":"raytrace-validation-v1","cases":cases,"local_checks_passed":!failed,"opticstudio_verified":false}),
        )?,
    )?;
    if failed {
        return Err("Local validation check failed; inspect suite.json".into());
    }
    Ok(())
}
