//! Rigid optical components with independent local coordinates and original documents.
use crate::{
    geometry::sag,
    math::{Frame, Pose, V3},
    model::*,
    project::{Issue, Project},
};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ComponentKind {
    System,
    Source,
    Optic,
    Rod,
    Detector,
}
impl ComponentKind {
    pub fn label(self) -> &'static str {
        match self {
            Self::System => "モデル",
            Self::Source => "光源",
            Self::Optic => "光学部品",
            Self::Rod => "ロッド",
            Self::Detector => "検出器",
        }
    }
    fn includes_sources(self) -> bool {
        matches!(self, Self::Source | Self::System)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RodSize {
    pub width_mm: f64,
    pub height_mm: f64,
    pub length_mm: f64,
    /// Optional power coatings on entrance, exit, or sides.
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub coatings: std::collections::BTreeMap<String, crate::coating::Coating>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Component {
    pub name: String,
    pub enabled: bool,
    pub kind: ComponentKind,
    pub pose: Pose,
    /// Local point placed at pose.translation_mm; lens first vertex / source-array centre.
    pub anchor_mm: V3,
    pub project: Box<Project>,
    pub rod: Option<RodSize>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Assembly {
    pub components: Vec<Component>,
}

pub fn air() -> Material {
    Material::Constant {
        n: 1.0,
        absorption_per_mm: 0.0,
    }
}
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

impl Component {
    pub fn imported(mut project: Project, kind: ComponentKind) -> Result<Self, String> {
        if project.assembly.is_some() {
            return Err("組立プロジェクトは「開く…」で読み込んでください".into());
        }
        if kind == ComponentKind::Source
            && project
                .zemax
                .as_ref()
                .is_some_and(|z| z.baseline_scene.mode == Mode::Sequential)
        {
            return Err(
                "光源部品にはNSC光源ZMX、または光源を定義したJSONを指定してください".into(),
            );
        }
        if !kind.includes_sources() {
            project.scene.sources.clear();
        }
        if kind != ComponentKind::System {
            for source in &mut project.scene.sources {
                if let Some(p) = &mut source.pupil {
                    if let Some(i) = p.surface.take() {
                        p.pose = project
                            .scene
                            .surfaces
                            .get(i)
                            .ok_or("Invalid pupil surface")?
                            .pose
                            .clone();
                    }
                }
            }
            project.scene.surfaces.retain(|s| match s.interaction {
                Interaction::Detector { .. } => false,
                Interaction::Dielectric {
                    minus_medium,
                    plus_medium,
                    ..
                } => minus_medium != plus_medium,
                _ => true,
            });
        }
        project.issues.retain(|i| {
            i.code != "illumination"
                && (kind.includes_sources()
                    || !["source", "spectrum", "illumination_definition"]
                        .contains(&i.code.as_str()))
        });
        if project.scene.mode == Mode::Sequential {
            let sides = close_sequential_lens(&project.scene.surfaces)?;
            if !sides.is_empty() {
                project.issue("barrel", "側面は各光学面の口径端を結ぶ円筒／円錐で構成します。実物の有効径・側面形状に合わせて確認してください", false);
                project.scene.surfaces.extend(sides);
            }
        }
        if kind == ComponentKind::Source && project.scene.sources.is_empty() {
            return Err("指定したファイルに利用可能な光源がありません".into());
        }
        if kind == ComponentKind::Optic && project.scene.surfaces.is_empty() {
            return Err("指定したファイルに利用可能な光学面がありません".into());
        }
        let anchor_mm = if kind == ComponentKind::Source {
            let sources = &project.scene.sources;
            V3(std::array::from_fn(|a| {
                let lo = sources
                    .iter()
                    .map(|s| s.origin_mm.0[a])
                    .fold(f64::INFINITY, f64::min);
                let hi = sources
                    .iter()
                    .map(|s| s.origin_mm.0[a])
                    .fold(f64::NEG_INFINITY, f64::max);
                if a == 2 {
                    lo
                } else {
                    0.5 * (lo + hi)
                }
            }))
        } else if kind == ComponentKind::Optic {
            project.scene.surfaces[0].pose.translation_mm
        } else {
            V3::default()
        };
        Ok(Self {
            name: project.scene.name.clone(),
            enabled: true,
            kind,
            pose: Pose::default(),
            anchor_mm,
            project: Box::new(project),
            rod: None,
        })
    }

    pub fn frame(&self) -> Frame {
        Frame::new(&self.pose)
    }
    fn place(&self, pose: &Pose) -> Pose {
        let local = Pose {
            translation_mm: pose.translation_mm - self.anchor_mm,
            rotation_deg: pose.rotation_deg,
        };
        self.frame().transform_pose(&local)
    }

    pub fn rod(material: Material, name: String) -> Self {
        let mut project = Project::new(empty_scene("ロッド"));
        project.scene.materials.push(material);
        project.material_names = vec!["AIR".into(), name];
        Self {
            name: "ロッド".into(),
            enabled: true,
            kind: ComponentKind::Rod,
            pose: Pose::default(),
            anchor_mm: V3::default(),
            project: Box::new(project),
            rod: Some(RodSize {
                coatings: Default::default(),
                width_mm: 1.0,
                height_mm: 1.0,
                length_mm: 30.0,
            }),
        }
    }

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
        Self {
            name: "スクリーン".into(),
            enabled: true,
            kind: ComponentKind::Detector,
            pose: Pose::default(),
            anchor_mm: V3::default(),
            project: Box::new(Project::new(scene)),
            rod: None,
        }
    }

    fn local_surfaces(&self) -> Result<Vec<Surface>, String> {
        let Some(r) = &self.rod else {
            return Ok(self.project.scene.surfaces.clone());
        };
        if [r.width_mm, r.height_mm, r.length_mm]
            .into_iter()
            .any(|v| !v.is_finite() || v <= 0.0)
        {
            return Err("ロッドの寸法は正の有限値で指定してください".into());
        }
        let (x, y, z) = (r.width_mm / 2.0, r.height_mm / 2.0, r.length_mm);
        if r.coatings
            .keys()
            .any(|k| !["entrance", "exit", "sides"].contains(&k.as_str()))
        {
            return Err("ロッドのコーティング対象はentrance/exit/sidesで指定してください".into());
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

fn close_sequential_lens(surfaces: &[Surface]) -> Result<Vec<Surface>, String> {
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
        let b = faces.get(i + 1).ok_or("レンズの出口面がありません")?;
        if !matches!(b.interaction, Interaction::Dielectric { minus_medium, .. } if minus_medium == inside)
        {
            return Err("レンズの媒質が連続していません".into());
        }
        let frame = Frame::new(&a.pose);
        let relative = frame.inverse_pose(&b.pose);
        if relative.rotation_deg.norm() > 1e-10
            || relative.translation_mm.0[0].abs() > 1e-10
            || relative.translation_mm.0[1].abs() > 1e-10
        {
            return Err("偏心した逐次レンズの側面生成は未対応です".into());
        }
        let (Aperture::Circle { radius_mm: ra }, Aperture::Circle { radius_mm: rb }) =
            (&a.aperture, &b.aperture)
        else {
            return Err("逐次レンズの部品読込は円形口径に対応しています".into());
        };
        let za = sag(&a.shape, *ra, 0.0)
            .ok_or("レンズ口径端の形状が不正です")?
            .0;
        let zb = relative.translation_mm.0[2]
            + sag(&b.shape, *rb, 0.0)
                .ok_or("レンズ口径端の形状が不正です")?
                .0;
        if zb <= za + 1e-8 {
            return Err("レンズ口径端で厚みが正になる形状が必要です".into());
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

impl Assembly {
    pub fn compose(&self, template: &Scene) -> Result<(Scene, Vec<String>, Vec<Issue>), String> {
        let mut scene = empty_scene(&template.name);
        scene.reference_plane = template.reference_plane.clone();
        scene.settings = template.settings.clone();
        let mut names = vec!["AIR".into()];
        let mut values = vec![serde_json::to_value(air()).unwrap()];
        let mut issues = Vec::new();
        for (index, c) in self
            .components
            .iter()
            .enumerate()
            .filter(|(_, c)| c.enabled)
        {
            if !c.pose.translation_mm.finite()
                || !c.pose.rotation_deg.finite()
                || !c.anchor_mm.finite()
            {
                return Err(format!("{}: 配置が不正です", c.name));
            }
            let surfaces = c.local_surfaces()?;
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
            if c.kind.includes_sources() {
                used.extend(c.project.scene.sources.iter().map(|s| s.medium));
            }
            used.sort_unstable();
            used.dedup();
            let mut map = vec![0; c.project.scene.materials.len()];
            for id in used {
                let material = c
                    .project
                    .scene
                    .materials
                    .get(id)
                    .ok_or("部品の材料番号が不正です")?;
                let value = serde_json::to_value(material).map_err(|e| e.to_string())?;
                if id == 0 && value != values[0] {
                    return Err("部品の媒質0は屈折率1・吸収0の空気で指定してください".into());
                }
                let global = if let Some(i) = values.iter().position(|v| *v == value) {
                    i
                } else {
                    values.push(value);
                    scene.materials.push(material.clone());
                    names.push(format!(
                        "{} / {}",
                        c.name,
                        c.project
                            .material_names
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
                if c.project.allow_uncoated {
                    s.coating = None;
                }
                s.name = format!("{}:{} / {}", index + 1, c.name, s.name);
                s.pose = c.place(&s.pose);
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
            if c.kind.includes_sources() {
                for source in &c.project.scene.sources {
                    let mut s = source.clone();
                    s.origin_mm = c.frame().world_point(s.origin_mm - c.anchor_mm);
                    s.direction = c.frame().world_vector(s.direction);
                    s.medium = map[s.medium];
                    if let Some(p) = &mut s.pupil {
                        p.pose = c.place(&p.pose);
                        if let Some(i) = &mut p.surface {
                            *i += surface_offset;
                        }
                    }
                    if let Some(d) = &mut s.diode {
                        d.rotation_deg = c
                            .frame()
                            .transform_pose(&Pose {
                                rotation_deg: d.rotation_deg,
                                ..Pose::default()
                            })
                            .rotation_deg;
                    }
                    if let Some(w) = c.project.monochromatic_override_um {
                        s.wavelength_um = w;
                        s.spectrum = None;
                    }
                    scene.sources.push(s);
                }
            }
            for issue in &c.project.issues {
                if issue.code == "coating" && c.project.allow_uncoated
                    || issue.code == "spectrum" && c.project.monochromatic_override_um.is_some()
                {
                    continue;
                }
                issues.push(Issue {
                    code: issue.code.clone(),
                    message: format!("{}: {}", c.name, issue.message),
                    blocking: issue.blocking,
                });
            }
        }
        Ok((scene, names, issues))
    }

    pub fn sync_sources(&mut self, scene: &Scene) {
        let mut index = 0;
        for c in self
            .components
            .iter_mut()
            .filter(|c| c.enabled && c.kind.includes_sources())
        {
            let frame = c.frame();
            for source in &mut c.project.scene.sources {
                let Some(flat) = scene.sources.get(index) else {
                    return;
                };
                let medium = source.medium;
                let pupil_surface = source.pupil.as_ref().and_then(|p| p.surface);
                *source = flat.clone();
                source.medium = medium;
                source.origin_mm = frame.local_point(source.origin_mm) + c.anchor_mm;
                source.direction = frame.local_vector(source.direction);
                if let Some(p) = &mut source.pupil {
                    p.pose = frame.inverse_pose(&p.pose);
                    p.pose.translation_mm = p.pose.translation_mm + c.anchor_mm;
                    p.surface = pupil_surface;
                }
                if let Some(d) = &mut source.diode {
                    d.rotation_deg = frame
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

pub fn demo_project() -> Result<Project, String> {
    let lens: Scene = serde_json::from_str(include_str!("../examples/sequential_lens.json"))
        .map_err(|e| e.to_string())?;
    let mut source = lens.clone();
    source.surfaces.clear();
    for s in &mut source.sources {
        s.count = 2048;
    }
    source.name = "円板光源".into();
    source.mode = Mode::NonSequential;
    let mut p = Project::new_assembly();
    p.add_component(Component::imported(
        Project::new(source),
        ComponentKind::Source,
    )?)?;
    let mut optic = Component::imported(Project::new(lens.clone()), ComponentKind::Optic)?;
    optic.name = "両凸レンズ".into();
    optic.pose.translation_mm.0[2] = 10.0;
    p.add_component(optic)?;
    let mut rod = Component::rod(lens.materials[1].clone(), "Silica".into());
    rod.pose.translation_mm.0[2] = 68.5;
    p.add_component(rod)?;
    let mut detector = Component::detector();
    detector.pose.translation_mm.0[2] = 100.5;
    p.add_component(detector)?;
    Ok(p)
}
