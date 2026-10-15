use raytrace_rs::{
    export::{self, DisplaySettings},
    storage::Preferences,
    trace::DetectorMap,
};
use std::{
    fs,
    path::PathBuf,
    sync::atomic::{AtomicUsize, Ordering},
};
static NEXT: AtomicUsize = AtomicUsize::new(0);
fn folder() -> PathBuf {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("target/export_tests")
        .join(format!(
            "{}_{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
    fs::create_dir_all(&p).unwrap();
    p
}
#[test]
fn exported_pixels_keep_power_coordinates_units_and_reference_without_renormalizing() {
    let dir = folder();
    let map = DetectorMap {
        name: "screen".into(),
        pixels: [2, 2],
        half_extent_mm: [2.0, 1.0],
        weights: vec![1.0, 2.0, 3.0, 4.0],
    };
    let display = DisplaySettings {
        peak_normalized: false,
        square_root: false,
        maximum_percent: Some(80.0),
    };
    export::write_screen(&dir, &map, "forward light", 10.0, display).unwrap();
    let csv = fs::read_to_string(dir.join("pixels.csv")).unwrap();
    let data: Vec<Vec<f64>> = csv
        .lines()
        .skip(1)
        .map(|l| l.split(',').map(|v| v.parse().unwrap()).collect())
        .collect();
    assert_eq!(data[0], vec![-1.0, -0.5, 1.0, 10.0, 5.0]);
    assert_eq!(data[3], vec![1.0, 0.5, 4.0, 40.0, 20.0]);
    assert_eq!(data.iter().map(|r| r[2]).sum::<f64>(), 10.0);
    let metadata: serde_json::Value =
        serde_json::from_slice(&fs::read(dir.join("metadata.json")).unwrap()).unwrap();
    assert_eq!(metadata["received_percent"], 100.0);
    assert_eq!(metadata["display"]["maximum_percent"], 80.0);
    assert_eq!(display.maximum_weight(4.0, 10.0), 8.0);
    #[cfg(feature = "gui")]
    {
        let png = image::open(dir.join("screen.png")).unwrap().to_rgb8();
        assert_eq!(png.get_pixel(0, 0).0, export::rgb(3.0 / 8.0, false));
    }
    assert!(export::write_screen(&dir, &map, "forward light", 0.0, display).is_err());
}
#[test]
fn recent_files_are_deduplicated_bounded_and_saved_independently_of_the_build_tree() {
    let dir = folder();
    let mut p = Preferences::default();
    for i in 0..12 {
        let path = dir.join(format!("{i}.json"));
        fs::write(&path, "{}").unwrap();
        p.remember(&path);
    }
    p.remember(&dir.join("11.json"));
    assert_eq!(p.recent_files.len(), 10);
    p.save(&dir.join("data")).unwrap();
    let restored = Preferences::load(&dir.join("data"));
    assert_eq!(restored.recent_files, p.recent_files);
    assert!(restored.recent_files[0].is_absolute());
    assert_eq!(restored.recent_files[0].file_name().unwrap(), "11.json");
}
