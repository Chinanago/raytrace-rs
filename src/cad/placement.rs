use super::{
    geometry::{frame, point, Transform},
    step::{field, Table, Value},
};
use crate::math::V3;
use std::collections::{HashMap, HashSet};

struct Representation {
    items: Vec<usize>,
    children: Vec<(usize, Transform)>,
}
fn operator(table: &Table, id: usize) -> Result<Transform, String> {
    let e = table.get(id)?;
    if e.has("AXIS2_PLACEMENT_3D") {
        return frame(table, id);
    }
    let a = e
        .part("CARTESIAN_TRANSFORMATION_OPERATOR_3D_NON_UNIFORM")
        .or_else(|| e.part("CARTESIAN_TRANSFORMATION_OPERATOR_3D"))
        .ok_or_else(|| format!("STEP #{id}: 未対応の配置変換 {}", e.kind()))?;
    let offset = if matches!(a.get(1), Some(Value::Text(_))) {
        2
    } else {
        1
    };
    let origin = point(table, field(a, offset + 2)?.reference()?)?;
    let base = Transform::identity();
    let mut axes = base.axes;
    for (i, axis) in axes.iter_mut().enumerate() {
        let index = offset + if i < 2 { i } else { 4 };
        match field(a, index)? {
            Value::Empty => (),
            v => {
                let values = field(table.args(v.reference()?, "DIRECTION")?, 1)?.numbers()?;
                if values.len() != 3 {
                    return Err("STEP: 配置変換の軸が3次元ではありません".into());
                }
                let d = V3([values[0], values[1], values[2]]);
                let n = d.norm();
                if n < 1e-15 || !n.is_finite() {
                    return Err("STEP: 配置変換の方向が不正です".into());
                }
                *axis = d / n;
            }
        }
    }
    if axes[0].dot(axes[1]).abs() > 1e-9
        || axes[0].dot(axes[2]).abs() > 1e-9
        || axes[1].dot(axes[2]).abs() > 1e-9
    {
        return Err("STEP: 直交しない配置変換は未対応です".into());
    }
    let scale = match field(a, offset + 3)? {
        Value::Empty => 1.,
        v => v.number()?,
    };
    let mut scales = [scale; 3];
    if e.has("CARTESIAN_TRANSFORMATION_OPERATOR_3D_NON_UNIFORM") {
        for (i, index) in [(1, offset + 5), (2, offset + 6)] {
            if !matches!(field(a, index)?, Value::Empty) {
                scales[i] = field(a, index)?.number()?;
            }
        }
    }
    if scales.iter().any(|s| *s <= 0. || !s.is_finite()) {
        return Err("STEP: 配置の倍率が不正です".into());
    }
    Ok(Transform {
        origin,
        axes: std::array::from_fn(|i| axes[i] * scales[i]),
    })
}

pub(super) fn solid_placement(table: &Table) -> Result<(usize, Transform), String> {
    let solids = table
        .entities
        .iter()
        .filter(|(_, e)| e.has("MANIFOLD_SOLID_BREP"))
        .map(|(&id, _)| id)
        .collect::<Vec<_>>();
    if solids.len() != 1 {
        return Err(format!(
            "STEP: 単一の閉じたソリッドが必要です（{}個）",
            solids.len()
        ));
    }
    let mut reps = HashMap::new();
    for (&id, e) in &table.entities {
        for kind in [
            "SHAPE_REPRESENTATION",
            "ADVANCED_BREP_SHAPE_REPRESENTATION",
            "FACETED_BREP_SHAPE_REPRESENTATION",
            "MANIFOLD_SURFACE_SHAPE_REPRESENTATION",
        ] {
            if let Some(a) = e.part(kind) {
                reps.insert(
                    id,
                    Representation {
                        items: field(a, 1)?.refs()?,
                        children: Vec::new(),
                    },
                );
                break;
            }
        }
    }
    if reps.is_empty() {
        return Ok((solids[0], Transform::identity()));
    }
    let mut relationships = Vec::new();
    for (&id, e) in &table.entities {
        if !e.has("SHAPE_REPRESENTATION_RELATIONSHIP") {
            continue;
        }
        let a = e
            .part("REPRESENTATION_RELATIONSHIP")
            .or_else(|| e.part("SHAPE_REPRESENTATION_RELATIONSHIP"))
            .unwrap();
        let (r1, r2) = (field(a, 2)?.reference()?, field(a, 3)?.reference()?);
        if !reps.contains_key(&r1) || !reps.contains_key(&r2) {
            continue;
        }
        if let Some(t) = e.part("REPRESENTATION_RELATIONSHIP_WITH_TRANSFORMATION") {
            let t = table.args(field(t, 0)?.reference()?, "ITEM_DEFINED_TRANSFORMATION")?;
            let (f1, f2) = (field(t, 2)?.reference()?, field(t, 3)?.reference()?);
            let transform = frame(table, f1)?.compose(frame(table, f2)?.inverse()?);
            if reps[&r2].items.contains(&f1) && reps[&r1].items.contains(&f2) {
                relationships.push((r2, r1, transform));
            } else if reps[&r1].items.contains(&f1) && reps[&r2].items.contains(&f2) {
                relationships.push((r1, r2, transform));
            } else {
                return Err(format!("STEP #{id}: 表現間の配置の基準を特定できません"));
            }
        } else {
            relationships.push((r1, r2, Transform::identity()));
        }
    }
    for (&parent, rep) in &reps {
        for &item in &rep.items {
            if let Some(a) = table.get(item)?.part("MAPPED_ITEM") {
                let map = table.args(field(a, 1)?.reference()?, "REPRESENTATION_MAP")?;
                let origin = frame(table, field(map, 0)?.reference()?)?;
                let child = field(map, 1)?.reference()?;
                if !reps.contains_key(&child) {
                    return Err("STEP: 配置の参照先が形状表現ではありません".into());
                }
                let target = operator(table, field(a, 2)?.reference()?)?;
                relationships.push((parent, child, target.compose(origin.inverse()?)));
            }
        }
    }
    let mut children = HashSet::new();
    for (parent, child, transform) in relationships {
        if parent == child {
            return Err("STEP: 配置の参照が循環しています".into());
        }
        children.insert(child);
        reps.get_mut(&parent)
            .unwrap()
            .children
            .push((child, transform));
    }
    for rep in reps.values_mut() {
        rep.children.sort_by_key(|(id, _)| *id);
    }
    let mut roots = reps
        .keys()
        .filter(|id| !children.contains(*id))
        .copied()
        .collect::<Vec<_>>();
    roots.sort();
    if roots.is_empty() {
        return Err("STEP: 配置の参照が循環しています".into());
    }
    let mut found = Vec::new();
    let mut visited = HashSet::new();
    for root in roots {
        walk(
            table,
            &reps,
            root,
            Transform::identity(),
            &mut HashSet::new(),
            &mut visited,
            &mut found,
        )?;
    }
    if visited.len() != reps.len() {
        return Err("STEP: 配置の参照に到達できない循環があります".into());
    }
    let mut distinct = Vec::new();
    for (id, t) in found {
        if !distinct
            .iter()
            .any(|(other_id, other): &(usize, Transform)| {
                *other_id == id
                    && (other.origin - t.origin).norm() < 1e-10
                    && other
                        .axes
                        .iter()
                        .zip(t.axes)
                        .all(|(a, b)| (*a - b).norm() < 1e-12)
            })
        {
            distinct.push((id, t));
        }
    }
    if distinct.len() != 1 {
        return Err(format!(
            "STEP: 単一の配置済みソリッドが必要です（{}個）",
            distinct.len()
        ));
    }
    Ok(distinct[0])
}
fn walk(
    table: &Table,
    reps: &HashMap<usize, Representation>,
    id: usize,
    transform: Transform,
    active: &mut HashSet<usize>,
    visited: &mut HashSet<usize>,
    found: &mut Vec<(usize, Transform)>,
) -> Result<(), String> {
    if active.len() > 64 || !active.insert(id) {
        return Err("STEP: 配置の参照が循環、または深さ64を超えています".into());
    }
    visited.insert(id);
    let rep = &reps[&id];
    for &item in &rep.items {
        let e = table.get(item)?;
        if e.has("MANIFOLD_SOLID_BREP") {
            found.push((item, transform));
        } else if !(e.has("AXIS2_PLACEMENT_3D") || e.has("MAPPED_ITEM")) {
            return Err(format!("STEP #{item}: 未対応の形状表現 {}", e.kind()));
        }
    }
    for &(child, t) in &rep.children {
        walk(
            table,
            reps,
            child,
            transform.compose(t),
            active,
            visited,
            found,
        )?;
    }
    active.remove(&id);
    Ok(())
}
