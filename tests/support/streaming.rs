use raytrace_rs::{
    assembly,
    model::{Sampling, Scene},
    zemax,
};
use std::path::Path;

fn diode_native(astigmatism: f64) -> String {
    let mut text = "MODE NSC\nUNIT MM\nPWAV 1\nWAVM 1 .55 1\nSURF 1\nTYPE NONSEQCO\nNSOH NSC_SDIO 0 0\nNSOP 3 4 5 10 20 30\n".to_owned();
    for (id, value) in [
        (2, 8192.),
        (3, 1.),
        (6, astigmatism),
        (7, 10.),
        (8, 2.),
        (9, 12.),
        (10, 0.5),
        (11, 1.),
        (12, 1.),
        (15, 1.),
        (16, 1.),
        (17, 2.),
        (18, 1.),
        (19, 1.),
        (20, 0.5),
    ] {
        text.push_str(&format!("NSOD {id} {value}\n"));
    }
    text.push_str("NSOH NSC_DETC 0 0\nNSOP 0 0 100 0 0 0 ABSORB\nNSOD 1 1000\nNSOD 2 1000\nNSOD 3 16\nNSOD 4 16\n");
    text
}
fn pupil_native(mode: usize, finite: bool) -> String {
    format!(
        "MODE SEQ\nUNIT MM\nGCAT own\nENPD 6\nRAIM {mode} 0 0\nFTYP {} 0 2 2\nXFLN 0 3\nYFLN 0 0\nFWGN 1 2\nWAVM 1 .45 1\nWAVM 2 .65 1\nPWAV 1\nSURF 0\nTYPE STANDARD\nDISZ {}\nSURF 1\nTYPE STANDARD\nCURV .05\nDIAM 5.5\nGLAS OWN 0 0\nDISZ 2\nSURF 2\nTYPE STANDARD\nCURV -.05\nDIAM 5.5\nDISZ 8\nSURF 3\nTYPE STANDARD\nSTOP\nDIAM 6\nDISZ 10\nSURF 4\nTYPE STANDARD\nDIAM 20\nDISZ 0\n",
        if finite { 1 } else { 0 },
        if finite { "50" } else { "INFINITY" }
    )
}

pub fn scenes() -> Vec<(String, Scene)> {
    static SCENES: std::sync::OnceLock<Vec<(String, Scene)>> = std::sync::OnceLock::new();
    SCENES.get_or_init(build_scenes).clone()
}

fn build_scenes() -> Vec<(String, Scene)> {
    let mut scenes = Vec::new();
    for name in [
        "folded_mirror",
        "integrator_rod",
        "nonsequential_slab",
        "sequential_lens",
    ] {
        let text = std::fs::read_to_string(
            Path::new(env!("CARGO_MANIFEST_DIR")).join(format!("examples/{name}.json")),
        )
        .unwrap();
        scenes.push((name.into(), serde_json::from_str(&text).unwrap()));
    }
    scenes.push((
        "assembly".into(),
        assembly::demo_project().unwrap().trace_scene().unwrap(),
    ));
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("target/streaming_fixtures");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("diode.zmx");
    std::fs::write(&path, diode_native(2.)).unwrap();
    let diode = zemax::read_zmx(&path, &[]).unwrap().trace_scene().unwrap();
    scenes.push(("sobol_diode".into(), diode.clone()));
    let mut random = diode;
    random.sources[0].sampling = Sampling::Pseudorandom;
    random.sources[0].diode.as_mut().unwrap().astigmatism_mm = 0.;
    scenes.push(("random_diode".into(), random));
    let mut spectrum = scenes[3].1.clone();
    spectrum.sources[0].spectrum = Some(raytrace_rs::spectrum::Spectrum {
        samples: vec![[0.45, 2.], [0.55, 1.], [0.65, 1.]],
    });
    scenes.push(("spectrum".into(), spectrum));
    std::fs::write(
        dir.join("own.AGF"),
        "NM OWN 2 0 1.46 50 0 0 0\nCD .8 .005 .3 .01 .1 10\nLD .4 .8\n",
    )
    .unwrap();
    let path = dir.join("pupil.zmx");
    std::fs::write(&path, pupil_native(1, false)).unwrap();
    scenes.push((
        "pupil_aiming".into(),
        zemax::read_zmx(&path, &[]).unwrap().trace_scene().unwrap(),
    ));
    scenes
}
