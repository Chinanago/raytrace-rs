//! The design document: parts (definitions in local coordinates) and occurrences (placements).
use crate::{Error, OccurrenceId, PartId, Result};
use raytrace_core::{
    coating::Coating,
    issue::Issue,
    math::{Pose, V3},
    model::{Material, Mode, Scene, Settings},
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// File format name written to `Design::format`.
pub const FORMAT: &str = "raytrace-design-v1";

/// Calculation conditions that change how imported definitions are traced.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(default, deny_unknown_fields)]
pub struct Conditions {
    /// Trace coated surfaces as uncoated Fresnel interfaces (changes the physics).
    pub allow_uncoated: bool,
    /// Replace every source spectrum with one wavelength in micrometres.
    pub monochromatic_override_um: Option<f64>,
}
impl Conditions {
    /// `self` applied after `inner`: coatings are dropped if either asks for it and the
    /// outer wavelength wins.
    pub fn over(&self, inner: &Self) -> Self {
        Self {
            allow_uncoated: self.allow_uncoated || inner.allow_uncoated,
            monochromatic_override_um: self
                .monochromatic_override_um
                .or(inner.monochromatic_override_um),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "snake_case")]
pub enum PartKind {
    /// A complete optical system: surfaces, sources and tracing mode.
    System,
    Source,
    Optic,
    Rod,
    Detector,
}
impl PartKind {
    pub fn label(self) -> &'static str {
        match self {
            Self::System => "モデル",
            Self::Source => "光源",
            Self::Optic => "光学部品",
            Self::Rod => "ロッド",
            Self::Detector => "検出器",
        }
    }
    /// Lower-case identifier prefix of new ids, e.g. `optic-3`.
    pub fn prefix(self) -> &'static str {
        match self {
            Self::System => "system",
            Self::Source => "source",
            Self::Optic => "optic",
            Self::Rod => "rod",
            Self::Detector => "detector",
        }
    }
    pub fn includes_sources(self) -> bool {
        matches!(self, Self::Source | Self::System)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct RodSize {
    pub width_mm: f64,
    pub height_mm: f64,
    pub length_mm: f64,
    /// Optional power coatings on entrance, exit, or sides.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub coatings: BTreeMap<String, Coating>,
}

/// Where a part was imported from. The document keeps it as an opaque JSON value;
/// only the input/output layer interprets it (for example to export ZMX again).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct Provenance {
    /// Name of the source format, for example `zmx`.
    pub format: String,
    pub document: serde_json::Value,
}

/// A part definition in its own local coordinates.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct Part {
    pub id: PartId,
    pub name: String,
    pub kind: PartKind,
    /// Local optical definition: materials, surfaces, sources, and (for a `system`
    /// part) the name, reference plane, tracing mode and limits of the whole system.
    /// A `rod` part keeps its material in slot 1; its six faces are generated from `rod`.
    pub scene: Scene,
    /// One display name per entry of `scene.materials`.
    pub material_names: Vec<String>,
    /// Import findings that may block tracing.
    #[serde(default)]
    pub issues: Vec<Issue>,
    #[serde(default)]
    pub conditions: Conditions,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rod: Option<RodSize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cad_deflection_mm: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provenance: Option<Provenance>,
}
impl Part {
    /// Add an issue unless one with the same message exists.
    pub fn issue(&mut self, code: &str, message: impl Into<String>, blocking: bool) {
        let message = message.into();
        if !self.issues.iter().any(|i| i.message == message) {
            self.issues.push(Issue {
                code: code.into(),
                message,
                blocking,
            });
        }
    }
}

fn enabled_by_default() -> bool {
    true
}

/// A placed instance of a part: a rigid transform plus an enabled switch.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct Occurrence {
    pub id: OccurrenceId,
    pub part: PartId,
    pub name: String,
    #[serde(default = "enabled_by_default")]
    pub enabled: bool,
    #[serde(default)]
    pub pose: Pose,
    /// Local point placed at `pose.translation_mm`: lens first vertex or source-array centre.
    #[serde(default)]
    pub anchor_mm: V3,
}
impl Occurrence {
    /// No translation, rotation or anchor: the part keeps its own coordinates.
    pub fn is_identity(&self) -> bool {
        self.pose
            .translation_mm
            .0
            .iter()
            .chain(&self.pose.rotation_deg.0)
            .chain(&self.anchor_mm.0)
            .all(|v| *v == 0.0)
    }
}

/// The design document. Compile it with [`Design::compile`] or [`Design::trace_scene`].
#[derive(Clone, Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct Design {
    /// Always `raytrace-design-v1`.
    pub format: String,
    /// Name, reference plane, tracing mode and limits of a composed design. A single
    /// system part placed without transform (see [`Design::lone_system`]) is traced
    /// as it is and uses its own scene header instead.
    pub name: String,
    pub reference_plane: String,
    pub settings: Settings,
    pub mode: Mode,
    #[serde(default)]
    pub conditions: Conditions,
    #[serde(default)]
    pub parts: Vec<Part>,
    /// Root-level placements in order; names of composed surfaces use their position.
    #[serde(default)]
    pub occurrences: Vec<Occurrence>,
    /// Next number for new ids; numbers are never reused.
    #[serde(default = "first_id")]
    pub next_id: u64,
}
fn first_id() -> u64 {
    1
}

/// Material names of a plain scene: `AIR`, `RT_M1`, ...
pub fn default_material_names(scene: &Scene) -> Vec<String> {
    (0..scene.materials.len())
        .map(|i| {
            if i == 0 {
                "AIR".into()
            } else {
                format!("RT_M{i}")
            }
        })
        .collect()
}

pub fn air() -> Material {
    Material::Constant {
        n: 1.0,
        absorption_per_mm: 0.0,
    }
}
/// A non-sequential scene with air only: the header of composed designs.
pub fn empty_scene(name: &str) -> Scene {
    Scene {
        name: name.into(),
        reference_plane: "合成光源の発射面".into(),
        mode: Mode::NonSequential,
        materials: vec![air()],
        surfaces: Vec::new(),
        sources: Vec::new(),
        settings: Settings {
            max_depth: 128,
            ..Settings::default()
        },
    }
}

impl Design {
    /// A composed design without parts.
    pub fn empty() -> Self {
        let scene = empty_scene("組立光学系");
        Self {
            format: FORMAT.into(),
            name: scene.name,
            reference_plane: scene.reference_plane,
            settings: scene.settings,
            mode: scene.mode,
            conditions: Conditions::default(),
            parts: Vec::new(),
            occurrences: Vec::new(),
            next_id: 1,
        }
    }

    /// A design of one `system` part holding `scene`, placed without transform.
    pub fn from_scene(scene: Scene) -> Self {
        let material_names = default_material_names(&scene);
        let part = Part {
            id: PartId(format!("{}-1", PartKind::System.prefix())),
            name: scene.name.clone(),
            kind: PartKind::System,
            scene,
            material_names,
            issues: Vec::new(),
            conditions: Conditions::default(),
            rod: None,
            cad_deflection_mm: None,
            provenance: None,
        };
        Self::from_system_part(part)
    }

    /// A design of one `system` part placed without transform.
    pub fn from_system_part(mut part: Part) -> Self {
        part.kind = PartKind::System;
        let mut design = Self {
            format: FORMAT.into(),
            name: part.scene.name.clone(),
            reference_plane: part.scene.reference_plane.clone(),
            settings: part.scene.settings.clone(),
            mode: part.scene.mode,
            conditions: Conditions::default(),
            parts: Vec::new(),
            occurrences: Vec::new(),
            next_id: 1,
        };
        part.id = design.new_part_id(PartKind::System);
        let occurrence = Occurrence {
            id: design.new_occurrence_id(PartKind::System),
            part: part.id.clone(),
            name: part.name.clone(),
            enabled: true,
            pose: Pose::default(),
            anchor_mm: V3::default(),
        };
        design.parts.push(part);
        design.occurrences.push(occurrence);
        design
    }

    /// Next free id `<prefix>-<number>`; the counter only grows.
    pub fn new_id(&mut self, prefix: &str) -> String {
        loop {
            let id = format!("{prefix}-{}", self.next_id);
            self.next_id += 1;
            if !self.parts.iter().any(|p| p.id == id.as_str())
                && !self.occurrences.iter().any(|o| o.id == id.as_str())
            {
                return id;
            }
        }
    }
    pub(crate) fn new_part_id(&mut self, kind: PartKind) -> PartId {
        PartId(self.new_id(kind.prefix()))
    }
    pub(crate) fn new_occurrence_id(&mut self, kind: PartKind) -> OccurrenceId {
        OccurrenceId(self.new_id(kind.prefix()))
    }

    pub fn part(&self, id: &PartId) -> Option<&Part> {
        self.parts.iter().find(|p| p.id == *id)
    }
    pub fn part_mut(&mut self, id: &PartId) -> Option<&mut Part> {
        self.parts.iter_mut().find(|p| p.id == *id)
    }
    pub fn occurrence(&self, id: &OccurrenceId) -> Option<&Occurrence> {
        self.occurrences.iter().find(|o| o.id == *id)
    }
    pub fn occurrence_mut(&mut self, id: &OccurrenceId) -> Option<&mut Occurrence> {
        self.occurrences.iter_mut().find(|o| o.id == *id)
    }
    pub fn occurrence_index(&self, id: &OccurrenceId) -> Option<usize> {
        self.occurrences.iter().position(|o| o.id == *id)
    }
    /// The part placed by `occurrences[index]`.
    pub fn occurrence_part(&self, index: usize) -> Option<&Part> {
        self.part(&self.occurrences.get(index)?.part)
    }
    pub fn occurrence_part_mut(&mut self, index: usize) -> Option<&mut Part> {
        let id = self.occurrences.get(index)?.part.clone();
        self.part_mut(&id)
    }

    /// Index of the part when the design is exactly one enabled `system` occurrence
    /// without transform. Such a design is traced verbatim, in its own mode.
    fn lone_part_index(&self) -> Option<usize> {
        let [occurrence] = self.occurrences.as_slice() else {
            return None;
        };
        if !occurrence.enabled || !occurrence.is_identity() {
            return None;
        }
        self.parts
            .iter()
            .position(|p| p.id == occurrence.part && p.kind == PartKind::System)
    }
    /// The verbatim system part (see [`Design::compile`]), if the design is a single
    /// unplaced system.
    pub fn lone_system(&self) -> Option<&Part> {
        self.lone_part_index().map(|i| &self.parts[i])
    }
    pub fn lone_system_mut(&mut self) -> Option<&mut Part> {
        self.lone_part_index().map(|i| &mut self.parts[i])
    }
    /// True unless the design is a single unplaced system.
    pub fn is_assembly(&self) -> bool {
        self.lone_part_index().is_none()
    }

    /// Name shown for the whole design.
    pub fn display_name(&self) -> &str {
        self.lone_system().map_or(&self.name, |p| &p.scene.name)
    }
    pub fn set_display_name(&mut self, name: impl Into<String>) {
        let name = name.into();
        if let Some(part) = self.lone_system_mut() {
            part.scene.name = name.clone();
        }
        self.name = name;
    }
    /// Tracing limits of the design (of the lone system when there is one).
    pub fn settings_mut(&mut self) -> &mut Settings {
        match self.lone_part_index() {
            Some(i) => &mut self.parts[i].scene.settings,
            None => &mut self.settings,
        }
    }
    pub fn settings_ref(&self) -> &Settings {
        self.lone_system()
            .map_or(&self.settings, |p| &p.scene.settings)
    }
    /// Tracing mode of the design (of the lone system when there is one).
    pub fn trace_mode(&self) -> Mode {
        self.lone_system().map_or(self.mode, |p| p.scene.mode)
    }
    pub fn set_trace_mode(&mut self, mode: Mode) {
        if let Some(part) = self.lone_system_mut() {
            part.scene.mode = mode;
        }
        self.mode = mode;
    }

    /// Calculation conditions that are in effect for `compile`d scenes: the design's own,
    /// combined with those of a verbatim system part (composed designs apply part
    /// conditions while composing).
    pub fn effective_conditions(&self) -> Conditions {
        self.lone_system().map_or_else(
            || self.conditions.clone(),
            |p| self.conditions.over(&p.conditions),
        )
    }

    /// Remove an occurrence and any part that nothing places any more.
    pub fn remove_occurrence(&mut self, index: usize) -> Option<Occurrence> {
        if index >= self.occurrences.len() {
            return None;
        }
        let removed = self.occurrences.remove(index);
        if !self.occurrences.iter().any(|o| o.part == removed.part) {
            self.parts.retain(|p| p.id != removed.part);
        }
        Some(removed)
    }

    /// Insert an independent copy of `occurrences[index]` and its part right after it.
    pub fn duplicate_occurrence(&mut self, index: usize) -> Result<OccurrenceId> {
        let source = self
            .occurrences
            .get(index)
            .cloned()
            .ok_or_else(|| Error::design("複製する配置がありません"))?;
        let mut part = self
            .part(&source.part)
            .cloned()
            .ok_or_else(|| Error::design("配置が参照する部品がありません"))?;
        part.id = self.new_part_id(part.kind);
        let mut copy = source;
        copy.id = self.new_occurrence_id(part.kind);
        copy.part = part.id.clone();
        copy.name = format!("{} コピー", copy.name);
        let id = copy.id.clone();
        self.parts.push(part);
        self.occurrences.insert(index + 1, copy);
        Ok(id)
    }
}
