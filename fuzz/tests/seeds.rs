use raytrace_formats::{cad, zemax};
use raytrace_fuzz::with_input;

#[test]
fn owned_seeds_reach_successful_imports() {
    with_input(include_bytes!("../corpus/zmx/plane"), "input.zmx", |path| {
        let model = zemax::read_zmx(path, &[]).unwrap();
        assert!(!model.scene.surfaces.is_empty());
    });
    with_input(include_bytes!("../corpus/agf/glass"), "FUZZ.agf", |path| {
        let material =
            zemax::read_catalog_material(path.parent().unwrap(), "FUZZ", "FUZZ").unwrap();
        assert_eq!(material.index(0.55), Some(1.5));
    });
    with_input(include_bytes!("../corpus/step/box"), "input.step", |path| {
        // A successful import reaches both the Part 21 parser and tessellation.
        cad::load_step_native_with_tolerance(path, 0.1).unwrap();
    });
}
