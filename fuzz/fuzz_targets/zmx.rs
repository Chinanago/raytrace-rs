#![no_main]
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    raytrace_fuzz::with_input(data, "input.zmx", |path| {
        let _ = raytrace_formats::zemax::read_zmx(path, &[]);
    });
});
