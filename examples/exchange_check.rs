//! Generate an edited local ZMX and identical small-ray reference data.
use raytrace_rs::{
    math::V3,
    model::Ray,
    project::Project,
    trace::{TraceOptions, trace},
    zemax,
};
use std::{fs, path::PathBuf};
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    let input = PathBuf::from(args.first().ok_or("input ZMX")?);
    let out = PathBuf::from(args.get(1).ok_or("output directory")?);
    fs::create_dir_all(&out)?;
    let mut project = Project::open(&input, &[])?;
    project.allow_uncoated = true;
    if project.scene.surfaces.len() != 4 {
        return Err("This check requires a dummy plane, two lens faces, and an image plane".into());
    }
    for s in project.scene.surfaces.iter_mut().skip(2) {
        s.pose.translation_mm.0[2] += 0.25;
    }
    let export = out.join("lens_edited.zmx");
    zemax::export(&project, &export)?;
    let mut scene = project.trace_scene()?;
    scene.surfaces.remove(0);
    let wavelength = scene.sources[0].wavelength_um;
    let scene = scene.prepare()?;
    let xy = [
        [0.0, 0.0],
        [0.1, 0.0],
        [0.5, 0.0],
        [1.0, 0.0],
        [0.0, 1.0],
        [0.5, 0.5],
        [-1.0, -0.5],
    ];
    let rays: Vec<_> = xy
        .into_iter()
        .enumerate()
        .map(|(i, [x, y])| Ray {
            id: i as u64,
            position_mm: V3([x, y, 0.0]),
            direction: V3([0.0, 0.0, 1.0]),
            weight: 1.0,
            wavelength_um: wavelength,
            medium: 0,
            optical_path_mm: 0.0,
        })
        .collect();
    let report = trace(
        &scene,
        &rays,
        TraceOptions {
            keep_terminals: true,
            path_launch_count: 0,
        },
    );
    fs::write(
        out.join("reference.json"),
        serde_json::to_vec_pretty(
            &serde_json::json!({"input":input,"export":export,"rays":rays,"report":report,"note":"Unpolarized geometric comparison; starts after Zemax surface 1; vendor coatings preserved in ZMX"}),
        )?,
    )?;
    println!("{}", export.display());
    Ok(())
}
