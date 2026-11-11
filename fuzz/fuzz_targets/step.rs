#![no_main]
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    raytrace_fuzz::with_input(data, "input.step", |path| {
        let _ = raytrace_formats::cad::load_step_native_with_tolerance(path, 0.1);
    });
});
