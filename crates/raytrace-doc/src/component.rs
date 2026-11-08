//! Building blocks for editing: a part together with its first occurrence.
use crate::{
    Conditions, Design, Error, Occurrence, OccurrenceId, Part, PartId, PartKind, Result, RodSize,
    design::{default_material_names, empty_scene},
};
use raytrace_core::{
    geometry::sag,
    math::{Frame, Pose, V3},
    model::{Aperture, Interaction, Material, Mode, Scene, Shape, Surface},
};

/// A part and the occurrence that will place it. The ids are placeholders until
/// [`Design::add_component`] assigns fresh ones.
#[derive(Clone, Debug)]
pub struct Component {
    pub part: Part,
    pub occurrence: Occurrence,
}

fn placeholder(prefix: &str) -> String {
    format!("{prefix}-0")
}

impl Component {
    fn new(kind: PartKind, name: &str, anchor_mm: V3, mut part: Part) -> Self {
        part.kind = kind;
        part.id = PartId(placeholder(kind.prefix()));
        Self {
            occurrence: Occurrence {
                id: OccurrenceId(placeholder(kind.prefix())),
                part: part.id.clone(),
                name: name.into(),
                enabled: true,
                pose: Pose::default(),
                anchor_mm,
            },
            part,
        }
    }

    /// Turn a standalone design (one unplaced system) into a part of `kind`.
    ///
    /// Sources are kept for `source` and `system` parts. Other kinds lose sources,
    /// detectors and surfaces that separate identical media, and pupils refer to poses.
    /// A sequential lens gets its barrel closed so that it can be traced in an assembly.
    pub fn imported(design: Design, kind: PartKind) -> Result<Self> {
        let Some(lone) = design.lone_system() else {
            return Err(Error::design(
                "組立プロジェクトは「開く…」で読み込んでください",
            ));
        };
        let mut part = lone.clone();
        part.conditions = design.conditions.over(&part.conditions);
        if !kind.includes_sources() {
            part.scene.sources.clear();
        }
        if kind != PartKind::System {
            for source in &mut part.scene.sources {
                if let Some(p) = &mut source.pupil
                    && let Some(i) = p.surface.take()
                {
                    p.pose = part
                        .scene
                        .surfaces
                        .get(i)
                        .ok_or_else(|| Error::design("瞳の参照面が不正です"))?
                        .pose
                        .clone();
                }
            }
            part.scene.surfaces.retain(|s| match s.interaction {
                Interaction::Detector { .. } => false,
                Interaction::Dielectric {
                    minus_medium,
                    plus_medium,
                    ..
                } => minus_medium != plus_medium,
                _ => true,
            });
        }
        part.issues.retain(|i| {
            i.code != "illumination"
                && (kind.includes_sources()
                    || !["source", "spectrum", "illumination_definition"]
                        .contains(&i.code.as_str()))
        });
        if part.scene.mode == Mode::Sequential {
            let sides = close_sequential_lens(&part.scene.surfaces)?;
            if !sides.is_empty() {
                part.issue("barrel", "側面は各光学面の口径端を結ぶ円筒／円錐で構成します。実物の有効径・側面形状に合わせて確認してください", false);
                part.scene.surfaces.extend(sides);
            }
        }
        if kind == PartKind::Source && part.scene.sources.is_empty() {
            return Err(Error::design(
                "指定したファイルに利用可能な光源がありません",
            ));
        }
        if kind == PartKind::Optic && part.scene.surfaces.is_empty() {
            return Err(Error::design(
                "指定したファイルに利用可能な光学面がありません",
            ));
        }
        let anchor_mm = if kind == PartKind::Source {
            let sources = &part.scene.sources;
            V3(std::array::from_fn(|a| {
                let lo = sources
                    .iter()
                    .map(|s| s.origin_mm.0[a])
                    .fold(f64::INFINITY, f64::min);
                let hi = sources
                    .iter()
                    .map(|s| s.origin_mm.0[a])
                    .fold(f64::NEG_INFINITY, f64::max);
                if a == 2 { lo } else { 0.5 * (lo + hi) }
            }))
        } else if kind == PartKind::Optic {
            part.scene.surfaces[0].pose.translation_mm
        } else {
            V3::default()
        };
        let name = part.scene.name.clone();
        part.name = name.clone();
        Ok(Self::new(kind, &name, anchor_mm, part))
    }

    /// A rectangular rod of `material`, named `name` in the material list.
    pub fn rod(material: Material, name: String) -> Self {
        let mut scene = empty_scene("ロッド");
        scene.materials.push(material);
        let part = Part {
            id: PartId(placeholder("rod")),
            name: "ロッド".into(),
            kind: PartKind::Rod,
            scene,
            material_names: vec!["AIR".into(), name],
            issues: Vec::new(),
            conditions: Conditions::default(),
            rod: Some(RodSize {
                coatings: Default::default(),
                width_mm: 1.0,
                height_mm: 1.0,
                length_mm: 30.0,
            }),
            cad_deflection_mm: None,
            provenance: None,
        };
        Self::new(PartKind::Rod, "ロッド", V3::default(), part)
    }

    /// A 20 mm square screen with 64 x 64 pixels.
    pub fn detector() -> Self {
        let mut scene = empty_scene("スクリーン");
        scene.surfaces.push(Surface {
            coating: None,
            name: "検出面".into(),
            pose: Pose::default(),
            shape: Shape::Plane,
            aperture: Aperture::Rectangle {
                half_width_mm: 10.0,
                half_height_mm: 10.0,
            },
            interaction: Interaction::Detector { pixels: [64, 64] },
        });
        let material_names = default_material_names(&scene);
        let part = Part {
            id: PartId(placeholder("detector")),
            name: "スクリーン".into(),
            kind: PartKind::Detector,
            scene,
            material_names,
            issues: Vec::new(),
            conditions: Conditions::default(),
            rod: None,
            cad_deflection_mm: None,
            provenance: None,
        };
        Self::new(PartKind::Detector, "スクリーン", V3::default(), part)
    }
}

impl Design {
    /// Append a component with fresh ids. A standalone design (one unplaced system)
    /// first becomes the first part of an assembly. The design is unchanged on error.
    pub fn add_component(&mut self, component: Component) -> Result<OccurrenceId> {
        let mut candidate = self.clone();
        if candidate.lone_system().is_some() {
            let conditions = candidate.conditions.clone();
            let current = Component::imported(candidate, PartKind::System)?;
            candidate = Self::empty();
            candidate.conditions = conditions;
            candidate.push_component(current);
        }
        let id = candidate.push_component(component);
        candidate.compile()?;
        *self = candidate;
        Ok(id)
    }

    /// Append a component with fresh ids, without conversion or validation.
    pub fn push_component(&mut self, mut component: Component) -> OccurrenceId {
        let kind = component.part.kind;
        component.part.id = self.new_part_id(kind);
        component.occurrence.part = component.part.id.clone();
        component.occurrence.id = self.new_occurrence_id(kind);
        let id = component.occurrence.id.clone();
        self.parts.push(component.part);
        self.occurrences.push(component.occurrence);
        id
    }
}

/// Close the side walls of sequential lenses as cones between the aperture edges.
fn close_sequential_lens(surfaces: &[Surface]) -> Result<Vec<Surface>> {
    let faces: Vec<_> = surfaces.iter().filter(|s| matches!(s.interaction, Interaction::Dielectric { minus_medium, plus_medium, .. } if minus_medium != plus_medium)).collect();
    let mut sides = Vec::new();
    for (i, a) in faces.iter().enumerate() {
        let Interaction::Dielectric {
            plus_medium: inside,
            ..
        } = a.interaction
        else {
            continue;
        };
        if inside == 0 {
            continue;
        }
        let b = faces
            .get(i + 1)
            .ok_or_else(|| Error::design("レンズの出口面がありません"))?;
        if !matches!(b.interaction, Interaction::Dielectric { minus_medium, .. } if minus_medium == inside)
        {
            return Err(Error::design("レンズの媒質が連続していません"));
        }
        let frame = Frame::new(&a.pose);
        let relative = frame.inverse_pose(&b.pose);
        if relative.rotation_deg.norm() > 1e-10
            || relative.translation_mm.0[0].abs() > 1e-10
            || relative.translation_mm.0[1].abs() > 1e-10
        {
            return Err(Error::design("偏心した逐次レンズの側面生成は未対応です"));
        }
        let (Aperture::Circle { radius_mm: ra }, Aperture::Circle { radius_mm: rb }) =
            (&a.aperture, &b.aperture)
        else {
            return Err(Error::design(
                "逐次レンズの部品読込は円形口径に対応しています",
            ));
        };
        let za = sag(&a.shape, *ra, 0.0)
            .ok_or_else(|| Error::design("レンズ口径端の形状が不正です"))?
            .0;
        let zb = relative.translation_mm.0[2]
            + sag(&b.shape, *rb, 0.0)
                .ok_or_else(|| Error::design("レンズ口径端の形状が不正です"))?
                .0;
        if zb <= za + 1e-8 {
            return Err(Error::design("レンズ口径端で厚みが正になる形状が必要です"));
        }
        sides.push(Surface {
            coating: None,
            name: format!("側面 {}", i + 1),
            pose: frame.transform_pose(&Pose {
                translation_mm: V3([0., 0., za]),
                ..Pose::default()
            }),
            shape: Shape::ConicalSide {
                radius_start_mm: *ra,
                radius_end_mm: *rb,
                length_mm: zb - za,
            },
            aperture: Aperture::Circle {
                radius_mm: ra.max(*rb),
            },
            interaction: Interaction::Dielectric {
                minus_medium: inside,
                plus_medium: 0,
                reflectance: None,
            },
        });
    }
    Ok(sides)
}

/// Scene of the demonstration assembly: disk source, biconvex lens, rod and screen.
pub fn demo_design() -> Result<Design> {
    let lens: Scene = serde_json::from_str(include_str!("../../../examples/sequential_lens.json"))?;
    let mut source = lens.clone();
    source.surfaces.clear();
    for s in &mut source.sources {
        s.count = 2048;
    }
    source.name = "円板光源".into();
    source.mode = Mode::NonSequential;
    let mut d = Design::empty();
    d.add_component(Component::imported(
        Design::from_scene(source),
        PartKind::Source,
    )?)?;
    let mut optic = Component::imported(Design::from_scene(lens.clone()), PartKind::Optic)?;
    optic.occurrence.name = "両凸レンズ".into();
    optic.occurrence.pose.translation_mm.0[2] = 10.0;
    d.add_component(optic)?;
    let mut rod = Component::rod(lens.materials[1].clone(), "Silica".into());
    rod.occurrence.pose.translation_mm.0[2] = 68.5;
    d.add_component(rod)?;
    let mut detector = Component::detector();
    detector.occurrence.pose.translation_mm.0[2] = 100.5;
    d.add_component(detector)?;
    Ok(d)
}
