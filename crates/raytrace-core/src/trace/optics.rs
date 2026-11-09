use crate::Result;
use crate::model::PreparedScene;
use std::collections::hash_map::Entry;

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
    /// `launch` is the ray id used to report a wavelength outside a material's range.
    pub(super) fn get(
        &mut self,
        scene: &PreparedScene,
        wavelength: f64,
        launch: u64,
    ) -> Result<&[(f64, f64)]> {
        let key = wavelength.to_bits();
        if key == self.primary_key {
            return Ok(&self.primary);
        }
        if self.primary.is_empty() {
            self.primary = evaluate_optics(scene, wavelength, launch).collect::<Result<_>>()?;
            self.primary_key = key;
            return Ok(&self.primary);
        }
        if self.other.len() + 1 < self.capacity {
            match self.other.entry(key) {
                Entry::Occupied(entry) => Ok(entry.into_mut()),
                Entry::Vacant(entry) => Ok(entry
                    .insert(evaluate_optics(scene, wavelength, launch).collect::<Result<_>>()?)),
            }
        } else if let Some(optics) = self.other.get(&key) {
            Ok(optics)
        } else {
            // Preserve exact wavelength evaluation beyond the cache budget; reuse storage.
            self.uncached.clear();
            for optics in evaluate_optics(scene, wavelength, launch) {
                self.uncached.push(optics?);
            }
            Ok(&self.uncached)
        }
    }
}

pub(super) fn evaluate_optics(
    scene: &PreparedScene,
    wavelength: f64,
    launch: u64,
) -> impl Iterator<Item = Result<(f64, f64)>> + '_ {
    // Every branch keeps its launch wavelength; evaluate dispersion and spectral
    // absorption once per material, rather than on every internal reflection.
    scene
        .spec
        .materials
        .iter()
        .enumerate()
        .map(
            move |(material, m)| match (m.index(wavelength), m.absorption_at(wavelength)) {
                (Some(index), Some(absorption)) => Ok((index, absorption)),
                _ => Err(crate::Error::Wavelength {
                    material,
                    wavelength_um: wavelength,
                    launch,
                }),
            },
        )
}
