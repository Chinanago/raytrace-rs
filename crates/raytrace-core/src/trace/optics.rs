use crate::model::PreparedScene;

pub(super) struct OpticalCache {
    primary_key: u64,
    primary: Vec<(f64, f64)>,
    other: std::collections::HashMap<u64, Vec<(f64, f64)>>,
    uncached: Vec<(f64, f64)>,
    capacity: usize,
}
impl OpticalCache {
    pub(super) fn new(scene: &PreparedScene) -> Self {
        Self {
            // Positive finite wavelengths cannot have the all-zero bit pattern.
            primary_key: 0,
            primary: Vec::new(),
            other: std::collections::HashMap::new(),
            uncached: Vec::new(),
            // Bound cached material data to ~64 KiB per chunk, up to 256 wavelengths.
            capacity: (65536 / (scene.spec.materials.len().max(1) * size_of::<(f64, f64)>()))
                .clamp(1, 256),
        }
    }
    pub(super) fn get(&mut self, scene: &PreparedScene, wavelength: f64) -> &[(f64, f64)] {
        let key = wavelength.to_bits();
        if key == self.primary_key {
            return &self.primary;
        }
        if self.primary.is_empty() {
            self.primary_key = key;
            self.primary = evaluate_optics(scene, wavelength).collect();
            return &self.primary;
        }
        if self.other.len() + 1 < self.capacity {
            self.other
                .entry(key)
                .or_insert_with(|| evaluate_optics(scene, wavelength).collect())
        } else if let Some(optics) = self.other.get(&key) {
            optics
        } else {
            // Preserve exact wavelength evaluation beyond the cache budget; reuse storage.
            self.uncached.clear();
            self.uncached.extend(evaluate_optics(scene, wavelength));
            &self.uncached
        }
    }
}

pub(super) fn evaluate_optics(
    scene: &PreparedScene,
    wavelength: f64,
) -> impl Iterator<Item = (f64, f64)> + '_ {
    // Every branch keeps its launch wavelength; evaluate dispersion and spectral
    // absorption once per material, rather than on every internal reflection.
    scene.spec.materials.iter().map(move |m| {
        (
            m.index(wavelength).expect("validated wavelength"),
            m.absorption_at(wavelength).expect("validated wavelength"),
        )
    })
}
