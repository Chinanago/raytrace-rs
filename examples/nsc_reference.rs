use raytrace_app::nsc_reference::*;
use raytrace_core::trace::DetectorMap;
use serde_json::{json, Value};
use std::{error::Error, fs, path::Path};
fn write(path: &Path, v: &impl serde::Serialize) -> Result<(), Box<dyn Error>> {
    fs::write(path, serde_json::to_vec_pretty(v)?)?;
    Ok(())
}
fn main() -> Result<(), Box<dyn Error>> {
    let args: Vec<_> = std::env::args().collect();
    let command = args.get(1).map(String::as_str).unwrap_or("");
    let root = Path::new(
        args.get(2)
            .ok_or("Usage: nsc_reference prepare|compare OUT_DIR [--rays N --budget-rays M]")?,
    );
    let count = |flag: &str, default| -> Result<usize, Box<dyn Error>> {
        let n = match args.iter().position(|s| s == flag) {
            Some(i) => args.get(i + 1).ok_or("Missing count")?.parse()?,
            None => default,
        };
        if n == 0 {
            return Err("Ray counts must be positive".into());
        }
        Ok(n)
    };
    if command == "prepare" {
        let catalog = std::env::var("RAYTRACE_GLASSCAT").unwrap_or_else(|_| {
            format!(
                "{}/Documents/Zemax/Glasscat",
                std::env::var("USERPROFILE").unwrap_or_default()
            )
        });
        let glass =
            raytrace_io::zemax::read_catalog_material(Path::new(&catalog), "SCHOTT", "N-BK7")?;
        for f in fixtures(
            count("--rays", 1_000_000)?,
            count("--budget-rays", 100_000)?,
        ) {
            let dir = root.join(&f.name);
            fs::create_dir_all(&dir)?;
            write(&dir.join("fixture.json"), &f)?;
            write(
                &dir.join("material_info.json"),
                &json!({"catalog":catalog,"glass":"N-BK7","air_n":1.,"indices":([0.45,0.55,0.65].map(|w|json!({"wavelength_um":w,"n":glass.index(w),"absorption_per_mm":glass.absorption_at(w)}))),"plate_rust":plate_expectation(glass.index(0.55).ok_or("Invalid glass")?,glass.absorption_at(0.55).ok_or("Invalid absorption")?,3.,false),"plate_polarized":plate_expectation(glass.index(0.55).ok_or("Invalid glass")?,glass.absorption_at(0.55).ok_or("Invalid absorption")?,3.,true)}),
            )?;
            for (label, seed) in [("A", 142), ("B", 987654321)] {
                let mut runs = Vec::new();
                for run in &f.runs {
                    println!(
                        "{} {} seed {label}: {} rays",
                        f.name, run.name, f.source.count
                    );
                    runs.push(f.trace(run, glass.clone(), seed)?);
                }
                write(
                    &dir.join(format!("rust_seed_{label}.json")),
                    &RustResults {
                        fixture: f.name.clone(),
                        runs,
                    },
                )?;
            }
        }
    } else if command == "compare" {
        let mut summary = Vec::new();
        let mut failed = false;
        println!("fixture / mode                         status  max L1");
        let mut dirs: Vec<_> = fs::read_dir(root)?.collect::<Result<_, _>>()?;
        dirs.sort_by_key(|d| d.file_name());
        for entry in dirs {
            let dir = entry.path();
            if !dir.join("fixture.json").exists() {
                continue;
            }
            let f: NscFixture = serde_json::from_slice(&fs::read(dir.join("fixture.json"))?)?;
            if !dir.join("opticstudio.json").exists() {
                println!("{} not_run", f.name);
                let result = json!({"fixture":f.name,"status":"not_run"});
                write(&dir.join("comparison.json"), &result)?;
                summary.push(result);
                continue;
            }
            let a: RustResults = serde_json::from_slice(&fs::read(dir.join("rust_seed_A.json"))?)?;
            let b: RustResults = serde_json::from_slice(&fs::read(dir.join("rust_seed_B.json"))?)?;
            let os: Value = serde_json::from_slice(&fs::read(dir.join("opticstudio.json"))?)?;
            let mut results = Vec::new();
            for run in &f.runs {
                let (Some(ra), Some(rb)) = (
                    a.runs.iter().find(|r| r.name == run.name),
                    b.runs.iter().find(|r| r.name == run.name),
                ) else {
                    failed = true;
                    results.push(json!({"name":run.name,"status":"failed","reason":"Rust seed result missing"}));
                    continue;
                };
                let Some(ro) = os["runs"]
                    .as_array()
                    .and_then(|rs| rs.iter().find(|r| r["name"] == ra.name))
                else {
                    failed = true;
                    results.push(json!({"name":ra.name,"status":"failed","reason":"OpticStudio run missing"}));
                    continue;
                };
                let maps: Vec<DetectorMap> = serde_json::from_value(ro["detectors"].clone())?;
                let m = ro["rays"].as_u64().ok_or("Missing OS ray count")? as usize;
                let mut checks = Vec::new();
                let mut error = None;
                if a.fixture != f.name || b.fixture != f.name || os["fixture"] != f.name {
                    error = Some("Fixture identity mismatch".to_owned());
                }
                if m != ra.rays || ra.rays != rb.rays {
                    error =
                        Some("Ray counts differ: re-run prepare with matching --rays".to_owned());
                }
                if maps.len() != ra.detectors.len() {
                    error = Some("Detector count differs".into());
                }
                for ((da, db), d) in ra.detectors.iter().zip(&rb.detectors).zip(&maps) {
                    match compare_maps(
                        da,
                        db,
                        d,
                        ra.rays,
                        m,
                        [ra.launched_weight, rb.launched_weight, 1.],
                    ) {
                        Ok(c) => checks.extend(c),
                        Err(e) => error = Some(e),
                    }
                }
                let mut budget_checks = Vec::new();
                let mut budget_not_compared = Vec::new();
                let ba = serde_json::to_value(&ra.small_run_budget)?;
                let bb = serde_json::to_value(&rb.small_run_budget)?;
                for category in [
                    "detected",
                    "escaped",
                    "bulk_absorbed",
                    "surface_absorbed",
                    "branch_cutoff",
                    "interaction_limit",
                    "errors",
                ] {
                    if ro["budget"]["categories"][category]["status"] == "compared" {
                        let va = if category == "errors" {
                            ba["numerical_failure"].as_f64().unwrap()
                                + ba["medium_mismatch"].as_f64().unwrap()
                        } else {
                            ba[category].as_f64().unwrap()
                        } / ba["launched"].as_f64().unwrap();
                        let vb = if category == "errors" {
                            bb["numerical_failure"].as_f64().unwrap()
                                + bb["medium_mismatch"].as_f64().unwrap()
                        } else {
                            bb[category].as_f64().unwrap()
                        } / bb["launched"].as_f64().unwrap();
                        let v = ro["budget"]["categories"][category]["fraction"]
                            .as_f64()
                            .ok_or("Missing budget fraction")?;
                        let nm = ro["budget_rays"].as_u64().ok_or("Missing budget count")? as usize;
                        budget_checks.push(Check::new(
                            format!("budget_{category}"),
                            (va - v).abs(),
                            power_tolerance(va, vb, ra.budget_rays, nm),
                        ));
                    } else {
                        budget_not_compared.push(json!({"category":category,"reason":ro["budget"]["categories"][category]["reason"]}));
                    }
                }
                if f.name == "tilted_window" {
                    let material: Value =
                        serde_json::from_slice(&fs::read(dir.join("material_info.json"))?)?;
                    for (i, category) in ["detected", "escaped", "bulk_absorbed"].iter().enumerate()
                    {
                        let expected_rust = material["plate_rust"][i]
                            .as_f64()
                            .ok_or("Missing analytic value")?;
                        let actual_rust = ba[category].as_f64().ok_or("Missing Rust budget")?
                            / ba["launched"].as_f64().unwrap();
                        budget_checks.push(Check::new(
                            format!("analytic_rust_{category}"),
                            (actual_rust - expected_rust).abs(),
                            2e-6,
                        ));
                        if let Some(actual) =
                            ro["budget"]["categories"][category]["fraction"].as_f64()
                        {
                            let expected = material["plate_polarized"][i]
                                .as_f64()
                                .ok_or("Missing analytic value")?;
                            budget_checks.push(Check::new(
                                format!("analytic_opticstudio_{category}"),
                                (actual - expected).abs(),
                                power_tolerance(
                                    expected,
                                    expected,
                                    ra.budget_rays,
                                    ro["budget_rays"].as_u64().ok_or("Missing budget count")?
                                        as usize,
                                ),
                            ));
                        }
                    }
                }
                let passed =
                    error.is_none() && checks.iter().chain(&budget_checks).all(|c| c.passed);
                failed |= !passed;
                println!(
                    "{:<38} {:<6} {:.6}",
                    format!("{}/{}", f.name, ra.name),
                    if passed { "pass" } else { "FAIL" },
                    checks.first().map_or(f64::NAN, |c| c.value)
                );
                results.push(json!({"name":ra.name,"status":if passed{"passed"}else{"failed"},"checks":checks,"budget_checks":budget_checks,"budget_not_compared":budget_not_compared,"error":error,"rust_metrics":ra.detectors.iter().map(|d| metrics(d).ok()).collect::<Vec<_>>(),"opticstudio_metrics":maps.iter().map(|d| metrics(d).ok()).collect::<Vec<_>>(),"model_difference":ro["settings"]["known_model_difference"]}));
            }
            let result = json!({"fixture":f.name,"runs":results});
            write(&dir.join("comparison.json"), &result)?;
            summary.push(result);
        }
        write(
            &root.join("summary.json"),
            &json!({"passed":!failed,"fixtures":summary}),
        )?;
        if failed {
            std::process::exit(1);
        }
    } else {
        return Err("Expected prepare or compare".into());
    }
    Ok(())
}
