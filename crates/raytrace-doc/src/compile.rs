//! Compilation of the design document into the scene that the tracer consumes.
//!
//! Rules (docs/DESIGN.md §5):
//! - Exactly one enabled `system` occurrence without transform: the part is the scene,
//!   unchanged (names, material table, issues, tracing mode).
//! - Anything else is composed as a non-sequential assembly: surface names get the
//!   occurrence number and name, material tables are merged, sources and pupils move
//!   with the occurrence, and the conditions of each part apply to its own content.
//! - An occurrence without transform never touches the coordinates it places.
use crate::{
    Design, Error, Occurrence, Part, Result,
    design::{air, empty_scene},
};
use raytrace_core::{
    issue::Issue,
    math::{Frame, Pose, V3},
    model::{Aperture, Interaction, Mode, Scene, Shape, Surface},
};

/// The traceable result of compiling a design.
#[derive(Clone, Debug)]
pub struct Compiled {
    pub scene: Scene,
    /// One name per entry of `scene.materials`.
    pub material_names: Vec<String>,
    /// Findings of the parts that are in effect, prefixed with the occurrence name when composed.
    pub issues: Vec<Issue>,
}

impl Design {
    /// Check ids and references.
    pub fn validate(&self) -> Result<()> {
        if self.format != crate::design::FORMAT {
            return Err(Error::design("未対応の設計文書形式です"));
        }
        for (i, part) in self.parts.iter().enumerate() {
            if self.parts[..i].iter().any(|p| p.id == part.id) {
                return Err(Error::design(format!(
                    "部品IDが重複しています: {}",
                    part.id
                )));
            }
        }
        for (i, occurrence) in self.occurrences.iter().enumerate() {
            if self.occurrences[..i].iter().any(|o| o.id == occurrence.id) {
                return Err(Error::design(format!(
                    "配置IDが重複しています: {}",
                    occurrence.id
                )));
            }
            if self.part(&occurrence.part).is_none() {
                return Err(Error::design(format!(
                    "配置 {} が存在しない部品 {} を参照しています",
                    occurrence.id, occurrence.part
                )));
            }
        }
        Ok(())
    }

    /// Compile the document. Pure: the same design always gives the same scene.
    pub fn compile(&self) -> Result<Compiled> {
        self.validate()?;
        if let Some(part) = self.lone_system() {
            return Ok(Compiled {
                scene: part.scene.clone(),
                material_names: part.material_names.clone(),
                issues: part.issues.clone(),
            });
        }
        if self.mode == Mode::Sequential {
            return Err(Error::design(
                "シーケンシャル追跡は、移動・回転のないモデル部品を1つだけ配置した設計に限ります。非シーケンシャルに変更してください",
            ));
        }
        self.compose()
    }

    /// The scene to trace: the compiled scene with the calculation conditions applied.
    /// Blocking issues that no condition resolves are reported as an error.
    pub fn trace_scene(&self) -> Result<Scene> {
        let Compiled {
            mut scene, issues, ..
        } = self.compile()?;
        let conditions = self.effective_conditions();
        let blocked: Vec<_> = issues
            .iter()
            .filter(|i| {
                i.blocking
                    && !(i.code == "coating" && conditions.allow_uncoated
                        || i.code == "spectrum" && conditions.monochromatic_override_um.is_some())
            })
            .map(|i| i.message.as_str())
            .collect();
        if !blocked.is_empty() {
            return Err(Error::design(blocked.join("\n")));
        }
        if conditions.allow_uncoated {
            for surface in &mut scene.surfaces {
                surface.coating = None;
            }
        }
        if let Some(wavelength) = conditions.monochromatic_override_um {
            for source in &mut scene.sources {
                source.wavelength_um = wavelength;
                source.spectrum = None;
                if let Some(o) = source.pupil.as_mut().and_then(|p| p.optical.as_mut()) {
                    o.primary_wavelength_um = wavelength;
                }
            }
        }
        Ok(scene)
    }

    fn compose(&self) -> Result<Compiled> {
        let mut scene = empty_scene(&self.name);
        scene.reference_plane = self.reference_plane.clone();
        scene.settings = self.settings.clone();
        let mut names = vec!["AIR".to_string()];
        let mut values = vec![serde_json::to_value(air())?];
        let mut issues = Vec::new();
        for (index, occurrence) in self
            .occurrences
            .iter()
            .enumerate()
            .filter(|(_, o)| o.enabled)
        {
            let part = self.part(&occurrence.part).ok_or_else(|| {
                Error::design(format!("配置 {} の部品がありません", occurrence.id))
            })?;
            if !occurrence.pose.translation_mm.finite()
                || !occurrence.pose.rotation_deg.finite()
                || !occurrence.anchor_mm.finite()
            {
                return Err(Error::design(format!(
                    "{}: 配置が不正です",
                    occurrence.name
                )));
            }
            let place = Placement::new(occurrence);
            let surfaces = part.local_surfaces()?;
            let mut used = vec![0];
            for s in &surfaces {
                if let Interaction::Dielectric {
                    minus_medium,
                    plus_medium,
                    ..
                } = s.interaction
                {
                    used.extend([minus_medium, plus_medium]);
                }
            }
            if part.kind.includes_sources() {
                used.extend(part.scene.sources.iter().map(|s| s.medium));
            }
            used.sort_unstable();
            used.dedup();
            let mut map = vec![0; part.scene.materials.len()];
            for id in used {
                let material = part
                    .scene
                    .materials
                    .get(id)
                    .ok_or_else(|| Error::design("部品の材料番号が不正です"))?;
                let value = serde_json::to_value(material)?;
                if id == 0 && value != values[0] {
                    return Err(Error::design(
                        "部品の媒質0は屈折率1・吸収0の空気で指定してください",
                    ));
                }
                let global = if let Some(i) = values.iter().position(|v| *v == value) {
                    i
                } else {
                    values.push(value);
                    scene.materials.push(material.clone());
                    names.push(format!(
                        "{} / {}",
                        occurrence.name,
                        part.material_names
                            .get(id)
                            .cloned()
                            .unwrap_or_else(|| format!("M{id}"))
                    ));
                    values.len() - 1
                };
                map[id] = global;
            }
            let surface_offset = scene.surfaces.len();
            for mut s in surfaces {
                if part.conditions.allow_uncoated {
                    s.coating = None;
                }
                s.name = format!("{}:{} / {}", index + 1, occurrence.name, s.name);
                s.pose = place.pose(&s.pose);
                if let Interaction::Dielectric {
                    minus_medium,
                    plus_medium,
                    ..
                } = &mut s.interaction
                {
                    *minus_medium = map[*minus_medium];
                    *plus_medium = map[*plus_medium];
                }
                scene.surfaces.push(s);
            }
            if part.kind.includes_sources() {
                for source in &part.scene.sources {
                    let mut s = source.clone();
                    if !place.identity {
                        s.origin_mm = place.frame.world_point(s.origin_mm - place.anchor);
                        s.direction = place.frame.world_vector(s.direction);
                    }
                    s.medium = map[s.medium];
                    if let Some(p) = &mut s.pupil {
                        p.pose = place.pose(&p.pose);
                        if let Some(o) = &mut p.optical {
                            o.first_surface += surface_offset;
                        }
                        if let Some(i) = &mut p.surface {
                            *i += surface_offset;
                        }
                    }
                    if let Some(d) = &mut s.diode
                        && !place.identity
                    {
                        d.rotation_deg = place
                            .frame
                            .transform_pose(&Pose {
                                rotation_deg: d.rotation_deg,
                                ..Pose::default()
                            })
                            .rotation_deg;
                    }
                    if let Some(w) = part.conditions.monochromatic_override_um {
                        s.wavelength_um = w;
                        s.spectrum = None;
                        if let Some(o) = s.pupil.as_mut().and_then(|p| p.optical.as_mut()) {
                            o.primary_wavelength_um = w;
                        }
                    }
                    scene.sources.push(s);
                }
            }
            for issue in &part.issues {
                if issue.code == "coating" && part.conditions.allow_uncoated
                    || issue.code == "spectrum"
                        && part.conditions.monochromatic_override_um.is_some()
                {
                    continue;
                }
                issues.push(Issue {
                    code: issue.code.clone(),
                    message: format!("{}: {}", occurrence.name, issue.message),
                    blocking: issue.blocking,
                });
            }
        }
        Ok(Compiled {
            scene,
            material_names: names,
            issues,
        })
    }

    /// Write the flat scene's source edits back into the parts, in part coordinates.
    /// `flat` must have the sources of `compile()` in the same order.
    pub fn sync_sources(&mut self, flat: &Scene) {
        let mut index = 0;
        for o in 0..self.occurrences.len() {
            let occurrence = &self.occurrences[o];
            if !occurrence.enabled {
                continue;
            }
            let place = Placement::new(occurrence);
            let anchor = place.anchor;
            let Some(part) = self.occurrence_part_mut(o) else {
                continue;
            };
            if !part.kind.includes_sources() {
                continue;
            }
            for source in &mut part.scene.sources {
                let Some(edited) = flat.sources.get(index) else {
                    return;
                };
                let medium = source.medium;
                let pupil_surface = source.pupil.as_ref().and_then(|p| p.surface);
                let pupil_first = source
                    .pupil
                    .as_ref()
                    .and_then(|p| p.optical.as_ref())
                    .map(|o| o.first_surface);
                *source = edited.clone();
                source.medium = medium;
                if !place.identity {
                    source.origin_mm = place.frame.local_point(source.origin_mm) + anchor;
                    source.direction = place.frame.local_vector(source.direction);
                }
                if let Some(p) = &mut source.pupil {
                    if !place.identity {
                        p.pose = place.frame.inverse_pose(&p.pose);
                        p.pose.translation_mm = p.pose.translation_mm + anchor;
                    }
                    p.surface = pupil_surface;
                    if let Some(o) = &mut p.optical {
                        o.first_surface = pupil_first.unwrap_or(o.first_surface);
                    }
                }
                if let Some(d) = &mut source.diode
                    && !place.identity
                {
                    d.rotation_deg = place
                        .frame
                        .inverse_pose(&Pose {
                            rotation_deg: d.rotation_deg,
                            ..Pose::default()
                        })
                        .rotation_deg;
                }
                index += 1;
            }
        }
    }
}

/// The rigid transform of one occurrence. An identity placement is never applied.
struct Placement {
    identity: bool,
    frame: Frame,
    anchor: V3,
}
impl Placement {
    fn new(occurrence: &Occurrence) -> Self {
        Self {
            identity: occurrence.is_identity(),
            frame: Frame::new(&occurrence.pose),
            anchor: occurrence.anchor_mm,
        }
    }
    /// A local pose in the design's coordinates.
    fn pose(&self, pose: &Pose) -> Pose {
        if self.identity {
            return pose.clone();
        }
        let local = Pose {
            translation_mm: pose.translation_mm - self.anchor,
            rotation_deg: pose.rotation_deg,
        };
        self.frame.transform_pose(&local)
    }
}

impl Part {
    /// Surfaces in the part's local coordinates; a rod generates its six faces.
    pub(crate) fn local_surfaces(&self) -> Result<Vec<Surface>> {
        let Some(r) = &self.rod else {
            return Ok(self.scene.surfaces.clone());
        };
        if [r.width_mm, r.height_mm, r.length_mm]
            .into_iter()
            .any(|v| !v.is_finite() || v <= 0.0)
        {
            return Err(Error::design("ロッドの寸法は正の有限値で指定してください"));
        }
        let (x, y, z) = (r.width_mm / 2.0, r.height_mm / 2.0, r.length_mm);
        if r.coatings
            .keys()
            .any(|k| !["entrance", "exit", "sides"].contains(&k.as_str()))
        {
            return Err(Error::design(
                "ロッドのコーティング対象はentrance/exit/sidesで指定してください",
            ));
        }
        let make = |name: &str, p, rot, hx, hy, minus, plus| Surface {
            coating: r
                .coatings
                .get(match name {
                    "入口" => "entrance",
                    "出口" => "exit",
                    _ => "sides",
                })
                .cloned(),
            name: name.into(),
            pose: Pose {
                translation_mm: V3(p),
                rotation_deg: V3(rot),
            },
            shape: Shape::Plane,
            aperture: Aperture::Rectangle {
                half_width_mm: hx,
                half_height_mm: hy,
            },
            interaction: Interaction::Dielectric {
                minus_medium: minus,
                plus_medium: plus,
                reflectance: None,
            },
        };
        Ok(vec![
            make("入口", [0.0, 0.0, 0.0], [0.0, 0.0, 0.0], x, y, 0, 1),
            make("出口", [0.0, 0.0, z], [0.0, 0.0, 0.0], x, y, 1, 0),
            make(
                "側面 +X",
                [x, 0.0, z / 2.0],
                [0.0, 90.0, 0.0],
                z / 2.0,
                y,
                1,
                0,
            ),
            make(
                "側面 -X",
                [-x, 0.0, z / 2.0],
                [0.0, -90.0, 0.0],
                z / 2.0,
                y,
                1,
                0,
            ),
            make(
                "側面 +Y",
                [0.0, y, z / 2.0],
                [-90.0, 0.0, 0.0],
                x,
                z / 2.0,
                1,
                0,
            ),
            make(
                "側面 -Y",
                [0.0, -y, z / 2.0],
                [90.0, 0.0, 0.0],
                x,
                z / 2.0,
                1,
                0,
            ),
        ])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Component, OccurrenceId, PartId, PartKind};
    use raytrace_core::model::Material;

    fn scene() -> Scene {
        serde_json::from_str(
            r#"{"name":"slab","reference_plane":"launch","mode":"sequential",
            "materials":[{"type":"constant","n":1.0},{"type":"constant","n":1.5}],
            "surfaces":[
              {"name":"front","shape":{"type":"plane"},"aperture":{"type":"circle","radius_mm":2},
               "interaction":{"type":"dielectric","minus_medium":0,"plus_medium":1}},
              {"name":"back","pose":{"translation_mm":[0,0,3]},"shape":{"type":"plane"},
               "aperture":{"type":"circle","radius_mm":2},
               "interaction":{"type":"dielectric","minus_medium":1,"plus_medium":0}},
              {"name":"screen","pose":{"translation_mm":[0,0,9]},"shape":{"type":"plane"},
               "aperture":{"type":"circle","radius_mm":2},"interaction":{"type":"detector","pixels":[4,4]}}],
            "sources":[{"count":8,"origin_mm":[0,0,-1],"direction":[0,0,1],"disk_radius_mm":1,
               "wavelength_um":0.55,"medium":0,"total_weight":1,"seed":1}]}"#,
        )
        .unwrap()
    }
    fn json(scene: &Scene) -> String {
        serde_json::to_string(scene).unwrap()
    }

    #[test]
    fn a_single_unplaced_system_is_its_own_scene() {
        let mut design = Design::from_scene(scene());
        design
            .lone_system_mut()
            .unwrap()
            .issue("note", "kept as is", false);
        let compiled = design.compile().unwrap();
        assert_eq!(json(&compiled.scene), json(&scene()));
        assert_eq!(compiled.scene.mode, Mode::Sequential);
        assert_eq!(compiled.material_names, ["AIR", "RT_M1"]);
        assert_eq!(compiled.issues.len(), 1);
        assert_eq!(compiled.issues[0].message, "kept as is");
        assert!(!design.is_assembly());
    }

    #[test]
    fn placing_the_system_or_adding_a_part_composes_a_non_sequential_assembly() {
        let mut moved = Design::from_scene(scene());
        moved.occurrences[0].pose.translation_mm = V3([0.0, 0.0, 5.0]);
        assert!(moved.is_assembly());
        // The design keeps the sequential mode of the system, which cannot be composed.
        assert!(moved.compile().is_err());
        moved.mode = Mode::NonSequential;
        let scene = moved.compile().unwrap().scene;
        assert_eq!(scene.mode, Mode::NonSequential);
        assert_eq!(scene.surfaces[0].name, "1:slab / front");
        assert_eq!(scene.surfaces[0].pose.translation_mm, V3([0.0, 0.0, 5.0]));
        assert_eq!(scene.sources[0].origin_mm, V3([0.0, 0.0, 4.0]));

        let mut added = Design::from_scene(self::scene());
        let mut screen = Component::detector();
        screen.occurrence.pose.translation_mm.0[2] = 20.0;
        added.add_component(screen).unwrap();
        assert_eq!(added.mode, Mode::NonSequential);
        assert_eq!(added.parts.len(), 2);
        assert_eq!(added.occurrences.len(), 2);
        let compiled = added.compile().unwrap();
        assert_eq!(compiled.scene.mode, Mode::NonSequential);
        assert!(compiled.scene.surfaces[0].name.starts_with("1:slab / "));
        assert_eq!(
            compiled.scene.surfaces.last().unwrap().name,
            "2:スクリーン / 検出面"
        );
    }

    #[test]
    fn a_placement_without_transform_leaves_coordinates_untouched() {
        let mut design = Design::empty();
        let mut rod = Component::rod(
            Material::Constant {
                n: 1.5,
                absorption_per_mm: 0.0,
            },
            "glass".into(),
        );
        rod.occurrence.pose = Pose::default();
        design.add_component(rod).unwrap();
        let scene = design.compile().unwrap().scene;
        // The side faces keep their exact local angles instead of values recomputed
        // from a rotation matrix.
        let angles: Vec<_> = scene
            .surfaces
            .iter()
            .map(|s| s.pose.rotation_deg.0)
            .collect();
        assert_eq!(angles[2], [0.0, 90.0, 0.0]);
        assert_eq!(angles[3], [0.0, -90.0, 0.0]);
        assert_eq!(angles[4], [-90.0, 0.0, 0.0]);
        assert_eq!(angles[5], [90.0, 0.0, 0.0]);
    }

    #[test]
    fn ids_are_validated_unique_and_never_reused() {
        let mut design = Design::from_scene(scene());
        let (part, occurrence) = (design.parts[0].id.clone(), design.occurrences[0].id.clone());
        assert_eq!(
            (part.as_str(), occurrence.as_str()),
            ("system-1", "system-2")
        );
        let next = design.new_id("optic");
        assert_eq!(next, "optic-3");
        design.remove_occurrence(0);
        assert!(design.parts.is_empty());
        assert_eq!(design.new_id("optic"), "optic-4");

        let mut duplicate = Design::from_scene(scene());
        let copy = duplicate.parts[0].clone();
        duplicate.parts.push(copy);
        assert!(
            duplicate
                .validate()
                .unwrap_err()
                .to_string()
                .contains("重複")
        );

        let mut dangling = Design::from_scene(scene());
        dangling.occurrences[0].part = PartId::new("missing-9").unwrap();
        assert!(
            dangling
                .compile()
                .unwrap_err()
                .to_string()
                .contains("missing-9")
        );
        assert!(serde_json::from_str::<OccurrenceId>("\"Not Valid\"").is_err());
    }

    #[test]
    fn conditions_of_parts_and_design_both_apply() {
        let mut design = Design::from_scene(scene());
        design
            .lone_system_mut()
            .unwrap()
            .issue("coating", "needs override", true);
        assert!(design.trace_scene().is_err());
        design.lone_system_mut().unwrap().conditions.allow_uncoated = true;
        assert!(design.trace_scene().is_ok());
        design.lone_system_mut().unwrap().conditions.allow_uncoated = false;
        design.conditions.allow_uncoated = true;
        assert!(design.trace_scene().is_ok());
        design.conditions.monochromatic_override_um = Some(0.6);
        assert_eq!(design.trace_scene().unwrap().sources[0].wavelength_um, 0.6);
    }

    #[test]
    fn components_keep_their_kind_and_duplicates_are_independent() {
        let mut design = Design::empty();
        design.add_component(Component::detector()).unwrap();
        design.add_component(Component::detector()).unwrap();
        assert_eq!(design.occurrences[0].id.as_str(), "detector-2");
        assert_eq!(design.occurrences[1].id.as_str(), "detector-4");
        let id = design.duplicate_occurrence(0).unwrap();
        assert_eq!(design.occurrences[1].id, id);
        assert_eq!(design.occurrences[1].name, "スクリーン コピー");
        design.occurrence_part_mut(1).unwrap().kind = PartKind::Detector;
        assert_ne!(design.occurrences[0].part, design.occurrences[1].part);
        assert_eq!(design.parts.len(), 3);
        design.remove_occurrence(1);
        assert_eq!(design.parts.len(), 2);
    }
}
