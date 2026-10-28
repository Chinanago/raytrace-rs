//! Persistent editor identities. The tracing core continues to use dense indices.
use crate::{Result, project::Project};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

#[derive(Clone, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct EntityId(pub String);
impl EntityId {
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}
impl std::fmt::Display for EntityId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Identity {
    pub next_id: u64,
    pub surfaces: Vec<EntityId>,
    pub sources: Vec<EntityId>,
}
impl Default for Identity {
    fn default() -> Self {
        Self {
            next_id: 1,
            surfaces: Vec::new(),
            sources: Vec::new(),
        }
    }
}
impl Identity {
    pub fn allocate(&mut self, prefix: &str) -> Result<EntityId> {
        let n = self.next_id;
        if n == 0 || n >= (1_u64 << 53) {
            return Err(crate::Error::Project("編集IDの上限に達しました".into()));
        }
        self.next_id += 1;
        Ok(EntityId(format!("{prefix}{n}")))
    }
    fn resize(&mut self, sources: usize, surfaces: usize) -> Result<()> {
        self.sources.truncate(sources);
        self.surfaces.truncate(surfaces);
        while self.sources.len() < sources {
            let id = self.allocate("l")?;
            self.sources.push(id);
        }
        while self.surfaces.len() < surfaces {
            let id = self.allocate("s")?;
            self.surfaces.push(id);
        }
        Ok(())
    }
}

/// Addresses follow the order of the compiled Scene, including enabled components only.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Bindings {
    pub surfaces: Vec<String>,
    pub sources: Vec<String>,
    pub detectors: Vec<String>,
}

impl Project {
    pub fn ensure_identity(&mut self) -> Result<()> {
        if let Some(a) = &mut self.assembly {
            self.identity.resize(0, 0)?;
            let mut seen = HashSet::new();
            for c in &mut a.components {
                if c.id.is_empty() {
                    c.id = self.identity.allocate("c")?;
                }
                if !seen.insert(c.id.clone()) {
                    return Err(crate::Error::Project("部品IDが重複しています".into()));
                }
                let surfaces = c.surface_count();
                c.project
                    .identity
                    .resize(c.project.scene.sources.len(), surfaces)?;
            }
        } else {
            self.identity
                .resize(self.scene.sources.len(), self.scene.surfaces.len())?;
        }
        Ok(())
    }

    /// Reject corrupt v4 identities instead of silently changing a saved reference.
    pub fn validate_identity(&self) -> Result<()> {
        fn check(
            ids: &[EntityId],
            prefix: &str,
            next: u64,
            seen: &mut HashSet<String>,
        ) -> Result<()> {
            for id in ids {
                let n =
                    id.0.strip_prefix(prefix)
                        .and_then(|v| v.parse::<u64>().ok());
                if n.is_none_or(|n| n == 0 || n >= next) || !seen.insert(id.0.clone()) {
                    return Err(crate::Error::Project(
                        "保存された編集IDが不正または重複しています".into(),
                    ));
                }
            }
            Ok(())
        }
        let mut seen = HashSet::new();
        if self.identity.next_id == 0 || self.identity.next_id > (1_u64 << 53) {
            return Err(crate::Error::Project("編集IDのカウンターが不正です".into()));
        }
        if let Some(a) = &self.assembly {
            if !self.identity.sources.is_empty() || !self.identity.surfaces.is_empty() {
                return Err(crate::Error::Project(
                    "組立光学系のIDは部品内で指定してください".into(),
                ));
            }
            for c in &a.components {
                check(
                    std::slice::from_ref(&c.id),
                    "c",
                    self.identity.next_id,
                    &mut seen,
                )?;
                let mut local = HashSet::new();
                let ids = &c.project.identity;
                if ids.next_id == 0
                    || ids.next_id > (1_u64 << 53)
                    || ids.sources.len() != c.project.scene.sources.len()
                    || ids.surfaces.len() != c.surface_count()
                {
                    return Err(crate::Error::Project(
                        "部品の編集IDと形状の対応が不正です".into(),
                    ));
                }
                check(&ids.sources, "l", ids.next_id, &mut local)?;
                check(&ids.surfaces, "s", ids.next_id, &mut local)?;
            }
        } else {
            if self.identity.sources.len() != self.scene.sources.len()
                || self.identity.surfaces.len() != self.scene.surfaces.len()
            {
                return Err(crate::Error::Project(
                    "編集IDと光学系の対応が不正です".into(),
                ));
            }
            check(
                &self.identity.sources,
                "l",
                self.identity.next_id,
                &mut seen,
            )?;
            check(
                &self.identity.surfaces,
                "s",
                self.identity.next_id,
                &mut seen,
            )?;
        }
        Ok(())
    }

    pub fn bindings(&self) -> Result<Bindings> {
        self.validate_identity()?;
        let mut b = Bindings {
            surfaces: Vec::new(),
            sources: Vec::new(),
            detectors: Vec::new(),
        };
        if let Some(a) = &self.assembly {
            for c in a.components.iter().filter(|c| c.enabled) {
                if c.rod.is_some() {
                    b.surfaces.extend(
                        c.project
                            .identity
                            .surfaces
                            .iter()
                            .map(|id| format!("{}/{}", c.id, id)),
                    );
                } else {
                    for (id, s) in c
                        .project
                        .identity
                        .surfaces
                        .iter()
                        .zip(&c.project.scene.surfaces)
                    {
                        let key = format!("{}/{}", c.id, id);
                        if matches!(
                            s.interaction,
                            raytrace_core::model::Interaction::Detector { .. }
                        ) {
                            b.detectors.push(key.clone());
                        }
                        b.surfaces.push(key);
                    }
                }
                if matches!(
                    c.kind,
                    crate::assembly::ComponentKind::Source | crate::assembly::ComponentKind::System
                ) {
                    b.sources.extend(
                        c.project
                            .identity
                            .sources
                            .iter()
                            .map(|id| format!("{}/{}", c.id, id)),
                    );
                }
            }
        } else {
            b.sources = self
                .identity
                .sources
                .iter()
                .map(ToString::to_string)
                .collect();
            for (id, s) in self.identity.surfaces.iter().zip(&self.scene.surfaces) {
                if matches!(
                    s.interaction,
                    raytrace_core::model::Interaction::Detector { .. }
                ) {
                    b.detectors.push(id.to_string());
                }
                b.surfaces.push(id.to_string());
            }
        }
        Ok(b)
    }
}
