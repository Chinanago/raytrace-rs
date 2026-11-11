#![no_main]
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    raytrace_fuzz::with_input(data, "FUZZ.agf", |path| {
        let _ = raytrace_formats::zemax::read_catalog_material(
            path.parent().expect("input has a parent"),
            "FUZZ",
            "FUZZ",
        );
    });
});
