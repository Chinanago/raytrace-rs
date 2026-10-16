use super::{
    geometry::{self, Curve, Surface, Transform},
    step::{field, Table},
};
use crate::{math::V3, model::Shape};
use std::collections::{BTreeSet, HashMap, HashSet};

const MAX_TRIANGLES: usize = 1_000_000;
#[derive(Clone, Copy)]
struct Vertex {
    uv: [f64; 2],
    point: V3,
    normal: V3,
}
#[derive(Clone, Copy)]
struct BoundaryPoint {
    point: V3,
    edge: usize,
    occurrence: usize,
    repeated: bool,
}
#[derive(Clone, Copy)]
struct ChartSample {
    parameter: f64,
    point: V3,
    uv: [f64; 2],
}
fn unwrap_near(surface: &Surface, uv: &mut [f64; 2], near: [f64; 2]) {
    for (i, period) in surface.periods().into_iter().enumerate() {
        if let Some(period) = period {
            uv[i] += ((near[i] - uv[i]) / period).round() * period;
        }
    }
}
struct Refinement<'a> {
    surface: &'a Surface,
    sign: f64,
    boundary: &'a HashSet<(usize, usize)>,
    rings: &'a [Vec<[f64; 2]>],
    metric: [f64; 2],
}
pub(super) struct Mesher<'a> {
    table: &'a Table,
    tolerance: f64,
    edges: HashMap<usize, Vec<V3>>,
    parameters: HashMap<usize, Vec<f64>>,
    curves: HashMap<usize, Curve>,
}
fn key(a: usize, b: usize) -> (usize, usize) {
    if a < b {
        (a, b)
    } else {
        (b, a)
    }
}
fn segment_distance(p: V3, a: V3, b: V3) -> f64 {
    let edge = b - a;
    let d = edge.dot(edge);
    if d == 0. {
        return (p - a).norm();
    }
    (p - (a + edge * ((p - a).dot(edge) / d).clamp(0., 1.))).norm()
}
fn triangle_distance(p: V3, a: V3, b: V3, c: V3) -> f64 {
    let ab = b - a;
    let ac = c - a;
    let n = ab.cross(ac);
    let n2 = n.dot(n);
    if n2 > 0. {
        let q = p - n * (p - a).dot(n) / n2;
        let u = (b - q).cross(c - q).dot(n) / n2;
        let v = (c - q).cross(a - q).dot(n) / n2;
        if u >= 0. && v >= 0. && u + v <= 1. {
            return (p - q).norm();
        }
    }
    segment_distance(p, a, b)
        .min(segment_distance(p, b, c))
        .min(segment_distance(p, c, a))
}
fn area(points: &[[f64; 2]]) -> f64 {
    let origin = points[0];
    (0..points.len())
        .map(|i| {
            let a = points[i];
            let b = points[(i + 1) % points.len()];
            (a[0] - origin[0]) * (b[1] - origin[1]) - (a[1] - origin[1]) * (b[0] - origin[0])
        })
        .sum::<f64>()
        * 0.5
}
fn contains(points: &[[f64; 2]], p: [f64; 2]) -> bool {
    let mut inside = false;
    for i in 0..points.len() {
        let a = points[i];
        let b = points[(i + 1) % points.len()];
        if (a[1] > p[1]) != (b[1] > p[1])
            && p[0] < (b[0] - a[0]) * (p[1] - a[1]) / (b[1] - a[1]) + a[0]
        {
            inside = !inside;
        }
    }
    inside
}
#[derive(Clone, Copy)]
struct ParameterVertex {
    position: spade::Point2<f64>,
    index: usize,
}
impl spade::HasPosition for ParameterVertex {
    type Scalar = f64;
    fn position(&self) -> spade::Point2<f64> {
        self.position
    }
}
fn triangulate(
    vertices: &[Vertex],
    boundary: &HashSet<(usize, usize)>,
    rings: &[Vec<[f64; 2]>],
    metric: [f64; 2],
) -> Result<Vec<[usize; 3]>, String> {
    use spade::{ConstrainedDelaunayTriangulation, Triangulation};
    let lo = std::array::from_fn::<_, 2, _>(|a| {
        rings[0].iter().map(|p| p[a]).fold(f64::INFINITY, f64::min)
    });
    let span = std::array::from_fn::<_, 2, _>(|a| {
        rings[0]
            .iter()
            .map(|p| p[a])
            .fold(f64::NEG_INFINITY, f64::max)
            - lo[a]
    });
    if span.iter().any(|x| !x.is_finite() || *x <= 0.) {
        return Err("STEP: 面の展開範囲が不正です".into());
    }
    let points = vertices
        .iter()
        .enumerate()
        .map(|(index, v)| ParameterVertex {
            position: spade::Point2::new(
                (v.uv[0] - lo[0]) / span[0] * metric[0],
                (v.uv[1] - lo[1]) / span[1] * metric[1],
            ),
            index,
        })
        .collect();
    let mut edges = boundary.iter().map(|&(a, b)| [a, b]).collect::<Vec<_>>();
    edges.sort();
    let mut conflict = false;
    let cdt = ConstrainedDelaunayTriangulation::<ParameterVertex>::try_bulk_load_cdt(
        points,
        edges,
        |_| conflict = true,
    )
    .map_err(|e| format!("STEP: 面の三角形分割に失敗しました: {e}"))?;
    if conflict || cdt.num_vertices() != vertices.len() {
        return Err("STEP: 面境界が交差、または展開点が重複しています".into());
    }
    // Crossing a trim constraint toggles inside/outside. This also retains
    // extremely thin faces whose floating-point centroid rounds onto an edge.
    let mut graph = vec![[0usize; 3]; cdt.num_all_faces()];
    let mut parity = vec![2u8; graph.len()];
    parity[0] = 0;
    let mut queue = std::collections::VecDeque::new();
    for face in cdt.inner_faces() {
        let i = face.fix().index();
        for (j, e) in face.adjacent_edges().into_iter().enumerate() {
            let neighbor = e.rev().face().fix().index();
            let constrained = e.is_constraint_edge() as usize;
            graph[i][j] = neighbor * 2 + constrained;
            if neighbor == 0 {
                let value = constrained as u8;
                if parity[i] != 2 && parity[i] != value {
                    return Err("STEP: 面境界の内外が矛盾しています".into());
                }
                if parity[i] == 2 {
                    parity[i] = value;
                    queue.push_back(i);
                }
            }
        }
    }
    while let Some(i) = queue.pop_front() {
        for &edge in &graph[i] {
            let neighbor = edge / 2;
            let value = parity[i] ^ (edge as u8 & 1);
            if parity[neighbor] == 2 {
                parity[neighbor] = value;
                queue.push_back(neighbor);
            } else if parity[neighbor] != value {
                return Err("STEP: 面境界が閉じていないか交差しています".into());
            }
        }
    }
    let triangles = cdt
        .inner_faces()
        .filter(|f| parity[f.fix().index()] == 1)
        .map(|f| f.vertices().map(|v| v.data().index))
        .collect::<Vec<_>>();
    if triangles.len() > MAX_TRIANGLES {
        return Err("STEP: 曲面の分割数が100万三角形を超えました".into());
    }
    Ok(triangles)
}

impl<'a> Mesher<'a> {
    pub fn new(table: &'a Table, tolerance: f64) -> Self {
        Self {
            table,
            tolerance,
            edges: HashMap::new(),
            parameters: HashMap::new(),
            curves: HashMap::new(),
        }
    }
    fn vertex_point(&self, id: usize) -> Result<V3, String> {
        geometry::point(
            self.table,
            field(self.table.args(id, "VERTEX_POINT")?, 1)?.reference()?,
        )
    }
    fn edge(&mut self, id: usize) -> Result<&[V3], String> {
        if !self.edges.contains_key(&id) {
            let args = self.table.args(id, "EDGE_CURVE")?;
            let start = self.vertex_point(field(args, 1)?.reference()?)?;
            let end = self.vertex_point(field(args, 2)?.reference()?)?;
            let curve = geometry::read_curve(self.table, field(args, 3)?.reference()?)?;
            let forward = field(args, 4)?.boolean()?;
            let mut t0 = curve.parameter(start);
            let mut t1 = curve.parameter(end);
            if let Curve::Spline(c) = &curve {
                let [a, b] = c.basis.range();
                if (start - end).norm() < 1e-10
                    && (c.evaluate(a).0 - start).norm() < 1e-8
                    && (c.evaluate(b).0 - end).norm() < 1e-8
                {
                    (t0, t1) = if forward { (a, b) } else { (b, a) };
                }
            }
            if curve.periodic() {
                if forward && t1 <= t0 + 1e-12 {
                    t1 += std::f64::consts::TAU;
                }
                if !forward && t1 >= t0 - 1e-12 {
                    t1 -= std::f64::consts::TAU;
                }
            } else if (forward && t1 < t0 - 1e-12) || (!forward && t1 > t0 + 1e-12) {
                return Err(format!("STEP #{id}: 境界曲線の向きと端点が一致しません"));
            }
            let endpoint_tol = (self.tolerance * 0.01).max(1e-8);
            if (curve.evaluate(t0).0 - start).norm() > endpoint_tol
                || (curve.evaluate(t1).0 - end).norm() > endpoint_tol
            {
                return Err(format!("STEP #{id}: 境界の端点が曲線上にありません"));
            }
            let mut cuts = vec![t0, t1];
            cuts.extend(
                curve
                    .breaks()
                    .into_iter()
                    .filter(|t| *t > t0.min(t1) && *t < t0.max(t1)),
            );
            cuts.sort_by(f64::total_cmp);
            cuts.dedup();
            if t1 < t0 {
                cuts.reverse();
            }
            let mut points = vec![(t0, start)];
            for pair in cuts.windows(2) {
                self.subdivide_curve(&curve, pair[0], pair[1], 0, &mut points)?;
            }
            if let Some(p) = points.last_mut() {
                p.1 = end;
            }
            if points.len() < 2 || points.len() > 65_536 {
                return Err(format!("STEP #{id}: 境界の分割数が範囲外です"));
            }
            let (parameters, points) = points.into_iter().unzip();
            self.edges.insert(id, points);
            self.parameters.insert(id, parameters);
            self.curves.insert(id, curve);
        }
        Ok(&self.edges[&id])
    }
    fn subdivide_curve(
        &self,
        c: &Curve,
        a: f64,
        b: f64,
        depth: usize,
        out: &mut Vec<(f64, V3)>,
    ) -> Result<(), String> {
        let (p, q) = (c.evaluate(a).0, c.evaluate(b).0);
        let error = [0.2, 0.4, 0.5, 0.6, 0.8]
            .into_iter()
            .map(|t| segment_distance(c.evaluate(a + (b - a) * t).0, p, q))
            .fold(0., f64::max);
        if error > self.tolerance * 0.02 {
            if depth >= 24 || out.len() > 65_536 {
                return Err("STEP: 曲線の分割が上限に達しました".into());
            }
            let m = a + (b - a) * 0.5;
            self.subdivide_curve(c, a, m, depth + 1, out)?;
            self.subdivide_curve(c, m, b, depth + 1, out)?;
        } else {
            out.push((b, q));
        }
        Ok(())
    }
    fn refine_shared_edges(&mut self, id: usize) -> Result<(), String> {
        let a = self.table.args(id, "ADVANCED_FACE")?;
        let mut surface = geometry::read_surface(self.table, field(a, 2)?.reference()?)?;
        if matches!(surface, Surface::Plane(_)) {
            return Ok(());
        }
        let bounds = field(a, 1)?.refs()?;
        let boundaries = bounds
            .iter()
            .map(|&id| self.boundary(id))
            .collect::<Result<Vec<_>, _>>()?;
        let all = boundaries
            .iter()
            .flatten()
            .map(|p| p.point)
            .collect::<Vec<_>>();
        surface.prepare_chart(&all)?;
        let edges = boundaries
            .iter()
            .flatten()
            .map(|p| p.edge)
            .collect::<BTreeSet<_>>();
        for id in edges {
            let curve = &self.curves[&id];
            let parameters = &self.parameters[&id];
            let points = &self.edges[&id];
            let mut refined = vec![(parameters[0], points[0])];
            let tolerance = (self.tolerance * 0.1).max(1e-7);
            let mut previous = surface.project(points[0], None, tolerance)?;
            for i in 0..points.len() - 1 {
                let mut next = surface.project(points[i + 1], Some(previous), tolerance)?;
                unwrap_near(&surface, &mut next, previous);
                self.subdivide_chart(
                    curve,
                    &surface,
                    [
                        ChartSample {
                            parameter: parameters[i],
                            point: points[i],
                            uv: previous,
                        },
                        ChartSample {
                            parameter: parameters[i + 1],
                            point: points[i + 1],
                            uv: next,
                        },
                    ],
                    0,
                    &mut refined,
                )?;
                previous = next;
            }
            let (parameters, points) = refined.into_iter().unzip();
            self.parameters.insert(id, parameters);
            self.edges.insert(id, points);
        }
        Ok(())
    }
    fn subdivide_chart(
        &self,
        curve: &Curve,
        surface: &Surface,
        samples: [ChartSample; 2],
        depth: usize,
        out: &mut Vec<(f64, V3)>,
    ) -> Result<(), String> {
        let [a, b] = samples;
        let middle = std::array::from_fn(|i| (a.uv[i] + b.uv[i]) * 0.5);
        let fit = (surface.point(a.uv) - a.point)
            .norm()
            .max((surface.point(b.uv) - b.point).norm());
        let target = (self.tolerance * 0.025).max(fit * 4.);
        let error = segment_distance(surface.point(middle), a.point, b.point);
        let angle = surface.normal(a.uv)?.dot(surface.normal(b.uv)?);
        if error > target || angle < 0.1f64.cos() {
            if depth >= 24 || out.len() > 65_536 {
                return Err("STEP: 曲面境界の分割が上限に達しました".into());
            }
            let parameter = a.parameter + (b.parameter - a.parameter) * 0.5;
            let point = curve.evaluate(parameter).0;
            let mut uv = surface.project(point, Some(middle), (self.tolerance * 0.1).max(1e-7))?;
            unwrap_near(surface, &mut uv, middle);
            let mid = ChartSample {
                parameter,
                point,
                uv,
            };
            self.subdivide_chart(curve, surface, [a, mid], depth + 1, out)?;
            self.subdivide_chart(curve, surface, [mid, b], depth + 1, out)?;
        } else {
            out.push((b.parameter, b.point));
        }
        Ok(())
    }
    fn boundary(&mut self, id: usize) -> Result<Vec<BoundaryPoint>, String> {
        let e = self.table.get(id)?;
        let args = e
            .part("FACE_OUTER_BOUND")
            .or_else(|| e.part("FACE_BOUND"))
            .ok_or_else(|| format!("STEP #{id}: 未対応の面境界 {}", e.kind()))?;
        let loop_id = field(args, 1)?.reference()?;
        let sense = field(args, 2)?.boolean()?;
        let edges = field(self.table.args(loop_id, "EDGE_LOOP")?, 1)?.refs()?;
        let mut counts = HashMap::new();
        for &oe in &edges {
            let edge = field(self.table.args(oe, "ORIENTED_EDGE")?, 3)?.reference()?;
            *counts.entry(edge).or_insert(0) += 1;
        }
        let mut seen = HashMap::new();
        let mut points: Vec<BoundaryPoint> = Vec::new();
        for edge in edges {
            let oe = self.table.args(edge, "ORIENTED_EDGE")?;
            let forward = field(oe, 4)?.boolean()?;
            let edge_id = field(oe, 3)?.reference()?;
            let mut samples = self.edge(edge_id)?.to_vec();
            if !forward {
                samples.reverse();
            }
            if let Some(last) = points.last() {
                if (last.point - samples[0]).norm() > (self.tolerance * 0.001).max(1e-8) {
                    return Err(format!("STEP #{id}: 境界の辺がつながっていません"));
                }
            }
            let occurrence = *seen.entry(edge_id).or_insert(0);
            *seen.get_mut(&edge_id).unwrap() += 1;
            points.extend(samples.into_iter().map(|point| BoundaryPoint {
                point,
                edge: edge_id,
                occurrence,
                repeated: counts[&edge_id] > 1,
            }));
        }
        if points.len() < 4
            || (points[0].point - points[points.len() - 1].point).norm()
                > (self.tolerance * 0.001).max(1e-8)
        {
            return Err(format!("STEP #{id}: 閉じていない面境界です"));
        }
        if !sense {
            points.reverse();
            for p in &mut points {
                p.occurrence = counts[&p.edge] - 1 - p.occurrence;
            }
        }
        Ok(points)
    }
    fn face(&mut self, id: usize) -> Result<(Vec<Vertex>, Vec<[usize; 3]>), String> {
        let a = self.table.args(id, "ADVANCED_FACE")?;
        let bounds = field(a, 1)?.refs()?;
        let mut surface = geometry::read_surface(self.table, field(a, 2)?.reference()?)?;
        let sign = if field(a, 3)?.boolean()? { 1. } else { -1. };
        if bounds.is_empty() {
            return Err(format!("STEP #{id}: 面境界がありません"));
        }
        let boundaries = bounds
            .iter()
            .map(|&id| self.boundary(id))
            .collect::<Result<Vec<_>, _>>()?;
        let all: Vec<_> = boundaries.iter().flatten().map(|p| p.point).collect();
        surface.prepare_chart(&all)?;
        let mut loops = Vec::new();
        let periods = surface.periods();
        let mut seam_axes = HashMap::new();
        for p in boundaries.iter().flatten().filter(|p| p.repeated) {
            if seam_axes.contains_key(&p.edge) {
                continue;
            }
            let samples = &self.edges[&p.edge];
            let mut projected = Vec::new();
            for i in [samples.len() / 4, samples.len() / 2, samples.len() * 3 / 4] {
                projected.push(surface.project(
                    samples[i],
                    None,
                    (self.tolerance * 0.1).max(1e-7),
                )?);
            }
            let axis = (0..2).find(|&i| {
                periods[i].is_some_and(|period| {
                    projected.iter().all(|p| {
                        let d = p[i] - projected[0][i];
                        (d - (d / period).round() * period).abs() < period * 1e-5
                    })
                })
            });
            seam_axes.insert(p.edge, axis);
        }
        let mut seam_bases = HashMap::new();
        for boundary in boundaries {
            let mut winding = [0.; 2];
            if boundary.iter().any(|p| p.repeated) && periods.iter().any(Option::is_some) {
                let mut projected = Vec::new();
                for p in boundary.iter().filter(|p| !p.repeated) {
                    projected.push(surface.project(
                        p.point,
                        None,
                        (self.tolerance * 0.1).max(1e-7),
                    )?);
                }
                for i in 0..2 {
                    if let Some(period) = periods[i] {
                        for j in 0..projected.len() {
                            let d = projected[(j + 1) % projected.len()][i] - projected[j][i];
                            winding[i] += d - (d / period).round() * period;
                        }
                    }
                }
            }
            let mut vertices: Vec<Vertex> = Vec::new();
            for bp in boundary {
                let p = bp.point;
                if bp.repeated && seam_axes[&bp.edge].is_none() {
                    continue;
                }
                let previous = vertices.last().map(|v| v.uv);
                let mut uv = surface.project(p, previous, (self.tolerance * 0.1).max(1e-7))?;
                let forced_axis = if bp.repeated {
                    seam_axes[&bp.edge]
                } else {
                    None
                };
                if let Some(axis) = forced_axis {
                    let period = periods[axis].unwrap();
                    let base = seam_bases.entry(bp.edge).or_insert_with(|| {
                        if let Some(prev) = previous {
                            uv[axis] + ((prev[axis] - uv[axis]) / period).round() * period
                        } else {
                            uv[axis]
                        }
                    });
                    uv[axis] = if bp.occurrence == 0 {
                        *base
                    } else {
                        surface.seam_opposite(axis, *base, previous, winding[axis])
                    };
                }
                if let Some(previous) = previous {
                    for i in 0..2 {
                        if Some(i) != forced_axis {
                            if let Some(period) = periods[i] {
                                uv[i] += ((previous[i] - uv[i]) / period).round() * period;
                            }
                        }
                    }
                }
                let normal = surface.normal(uv)? * sign;
                let duplicate = vertices.last().is_some_and(|v| {
                    let scale = uv
                        .iter()
                        .chain(v.uv.iter())
                        .map(|x| x.abs())
                        .fold(1., f64::max);
                    (v.uv[0] - uv[0]).hypot(v.uv[1] - uv[1]) < scale * 1e-11
                        && (v.point - p).norm() < 1e-9
                });
                if !duplicate {
                    vertices.push(Vertex {
                        uv,
                        point: p,
                        normal,
                    });
                }
            }
            if vertices.len() > 1 {
                let a = vertices[0];
                let b = vertices[vertices.len() - 1];
                let scale =
                    a.uv.iter()
                        .chain(b.uv.iter())
                        .map(|x| x.abs())
                        .fold(1., f64::max);
                if (a.uv[0] - b.uv[0]).hypot(a.uv[1] - b.uv[1]) < scale * 1e-11
                    && (a.point - b.point).norm() < 1e-9
                {
                    vertices.pop();
                }
            }
            if vertices.len() < 3 {
                return Err(format!("STEP #{id}: 展開した境界が退化しています"));
            }
            loops.push(vertices);
        }
        loops.sort_by(|a, b| {
            area(&b.iter().map(|v| v.uv).collect::<Vec<_>>())
                .abs()
                .total_cmp(&area(&a.iter().map(|v| v.uv).collect::<Vec<_>>()).abs())
        });
        let center = std::array::from_fn::<_, 2, _>(|i| {
            loops[0].iter().map(|v| v.uv[i]).sum::<f64>() / loops[0].len() as f64
        });
        for hole in loops.iter_mut().skip(1) {
            let c = std::array::from_fn::<_, 2, _>(|i| {
                hole.iter().map(|v| v.uv[i]).sum::<f64>() / hole.len() as f64
            });
            for i in 0..2 {
                if let Some(period) = periods[i] {
                    let offset = ((center[i] - c[i]) / period).round() * period;
                    for v in hole.iter_mut() {
                        v.uv[i] += offset;
                    }
                }
            }
        }
        let outer = loops[0].iter().map(|v| v.uv).collect::<Vec<_>>();
        let mut expected_area = area(&outer).abs();
        if expected_area < 1e-24 {
            return Err(format!("STEP #{id}: 面の展開面積がゼロです"));
        }
        for hole in loops.iter().skip(1) {
            if !contains(&outer, hole[0].uv) {
                return Err(format!(
                    "STEP #{id}: 内側の境界が外側の境界に収まっていません"
                ));
            }
            expected_area -= area(&hole.iter().map(|v| v.uv).collect::<Vec<_>>()).abs();
        }
        // A collapsed parametric edge at a pole needs parameter samples even
        // though all of its spatial points coincide. Otherwise the CDT keeps
        // creating thin wedges against the entire unsplit pole interval.
        for ring in &mut loops {
            let original = std::mem::take(ring);
            for j in 0..original.len() {
                let a = original[j];
                let b = original[(j + 1) % original.len()];
                ring.push(a);
                let uv = std::array::from_fn(|i| (a.uv[i] + b.uv[i]) * 0.5);
                let epsilon = (self.tolerance * 1e-6).max(1e-10);
                if (a.point - b.point).norm() < epsilon
                    && (surface.point(uv) - a.point).norm() < epsilon
                {
                    for k in 1..64 {
                        let uv =
                            std::array::from_fn(|i| a.uv[i] + (b.uv[i] - a.uv[i]) * k as f64 / 64.);
                        ring.push(Vertex {
                            uv,
                            point: surface.point(uv),
                            normal: surface.normal(uv)? * sign,
                        });
                    }
                }
            }
        }
        let rings = loops
            .iter()
            .map(|ring| ring.iter().map(|v| v.uv).collect::<Vec<_>>())
            .collect::<Vec<_>>();
        let mut vertices = Vec::new();
        let mut boundary_edges = HashSet::new();
        for ring in loops {
            let start = vertices.len();
            let n = ring.len();
            for j in 0..n {
                boundary_edges.insert(key(start + j, start + (j + 1) % n));
            }
            vertices.extend(ring);
        }
        let bounds = std::array::from_fn::<_, 2, _>(|a| {
            let lo = rings[0].iter().map(|p| p[a]).fold(f64::INFINITY, f64::min);
            let hi = rings[0]
                .iter()
                .map(|p| p[a])
                .fold(f64::NEG_INFINITY, f64::max);
            [lo, hi]
        });
        let mut lengths = [Vec::new(), Vec::new()];
        for i in 0..5 {
            for j in 0..5 {
                let uv = [
                    bounds[0][0] + (bounds[0][1] - bounds[0][0]) * (i as f64 + 0.5) / 5.,
                    bounds[1][0] + (bounds[1][1] - bounds[1][0]) * (j as f64 + 0.5) / 5.,
                ];
                if !contains(&rings[0], uv) || rings.iter().skip(1).any(|r| contains(r, uv)) {
                    continue;
                }
                let (_, du, dv) = surface.sample(uv);
                for (a, d) in [du, dv].into_iter().enumerate() {
                    let n = d.norm() * (bounds[a][1] - bounds[a][0]);
                    if n.is_finite() && n > 0. {
                        lengths[a].push(n);
                    }
                }
            }
        }
        let mut metric = [1.; 2];
        for (a, v) in lengths.iter_mut().enumerate() {
            v.sort_by(f64::total_cmp);
            if !v.is_empty() {
                metric[a] = v[v.len() / 2];
            }
        }
        let size = metric[0].max(metric[1]);
        metric = metric.map(|x| x / size);
        let mut triangles = triangulate(&vertices, &boundary_edges, &rings, metric)?;
        let actual_area: f64 = triangles
            .iter()
            .map(|t| area(&t.map(|i| vertices[i].uv)).abs())
            .sum();
        if expected_area <= 0. || (actual_area - expected_area).abs() > expected_area * 1e-8 {
            return Err(format!("STEP #{id}: 三角形と境界の面積が一致しません"));
        }
        check_boundary(&triangles, &boundary_edges, "初期分割")?;
        if !matches!(surface, Surface::Plane(_)) {
            self.refine(
                Refinement {
                    surface: &surface,
                    sign,
                    boundary: &boundary_edges,
                    rings: &rings,
                    metric,
                },
                &mut vertices,
                &mut triangles,
            )?;
        }
        check_boundary(&triangles, &boundary_edges, "曲面分割")?;

        if sign < 0. {
            for t in &mut triangles {
                t.swap(1, 2);
            }
        }
        Ok((vertices, triangles))
    }
    fn refine(
        &self,
        context: Refinement<'_>,
        vertices: &mut Vec<Vertex>,
        triangles: &mut Vec<[usize; 3]>,
    ) -> Result<(), String> {
        let Refinement {
            surface,
            sign,
            boundary,
            rings,
            metric,
        } = context;
        for _ in 0..28 {
            let mut split = BTreeSet::new();
            let mut centers = Vec::new();
            let mut acceptable = true;
            for (ti, tri) in triangles.iter().enumerate() {
                for j in 0..3 {
                    let (a, b) = (tri[j], tri[(j + 1) % 3]);
                    if boundary.contains(&key(a, b)) {
                        continue;
                    }
                    let uv = std::array::from_fn(|i| (vertices[a].uv[i] + vertices[b].uv[i]) * 0.5);
                    let wraps = surface.periods().iter().enumerate().any(|(i, p)| {
                        p.is_some_and(|p| {
                            (vertices[a].uv[i] - vertices[b].uv[i]).abs() > p * 0.125 + p * 1e-12
                        })
                    });
                    let error =
                        segment_distance(surface.point(uv), vertices[a].point, vertices[b].point);
                    let angle = vertices[a].normal.dot(vertices[b].normal);
                    if wraps || error > self.tolerance * 0.1 || angle < 0.1f64.cos() {
                        acceptable = false;
                    }
                    // Refine a wider band than the acceptance threshold to
                    // avoid a succession of almost-critical Delaunay edge flips.
                    if wraps || error > self.tolerance * 0.07 || angle < 0.08f64.cos() {
                        split.insert(key(a, b));
                    }
                }
                let uv = std::array::from_fn(|i| {
                    tri.iter().map(|&j| vertices[j].uv[i]).sum::<f64>() / 3.
                });
                let [a, b, c] = tri.map(|j| vertices[j].point);
                let error = triangle_distance(surface.point(uv), a, b, c);
                if error > self.tolerance * 0.1 {
                    acceptable = false;
                }
                if error > self.tolerance * 0.07 {
                    centers.push(ti);
                }
            }
            centers.retain(|&i| {
                !(0..3).any(|j| split.contains(&key(triangles[i][j], triangles[i][(j + 1) % 3])))
            });
            if acceptable {
                return Ok(());
            }
            for &(a, b) in &split {
                let uv = std::array::from_fn(|i| (vertices[a].uv[i] + vertices[b].uv[i]) * 0.5);
                vertices.push(Vertex {
                    uv,
                    point: surface.point(uv),
                    normal: surface.normal(uv)? * sign,
                });
            }
            for i in centers {
                let uv = std::array::from_fn(|a| {
                    triangles[i].iter().map(|&j| vertices[j].uv[a]).sum::<f64>() / 3.
                });
                vertices.push(Vertex {
                    uv,
                    point: surface.point(uv),
                    normal: surface.normal(uv)? * sign,
                });
            }
            *triangles = triangulate(vertices, boundary, rings, metric)?;
        }

        Err("STEP: 曲面の分割が収束しません".into())
    }

    pub fn solid(&mut self, id: usize, transform: Transform, scale: f64) -> Result<Shape, String> {
        let root = self.table.args(id, "MANIFOLD_SOLID_BREP")?;
        let faces = field(
            self.table
                .args(field(root, 1)?.reference()?, "CLOSED_SHELL")?,
            1,
        )?
        .refs()?;
        // Finish shared trim samples before any face is triangulated.
        for &id in &faces {
            self.refine_shared_edges(id)
                .map_err(|e| format!("STEP face #{id}: {e}"))?;
        }
        let (mut vertices, mut normals, mut triangles) = (Vec::new(), Vec::new(), Vec::new());
        for id in faces {
            let (v, t) = self.face(id).map_err(|e| format!("STEP face #{id}: {e}"))?;
            let offset = vertices.len();
            for v in v {
                vertices.push(transform.point(v.point) * scale);
                normals.push(transform.normal(v.normal)?);
            }
            triangles.extend(t.into_iter().map(|t| t.map(|i| (i + offset) as u32)));
            if triangles.len() > MAX_TRIANGLES {
                return Err("STEP: 分割数が100万三角形を超えました".into());
            }
        }
        if transform.axes[0].dot(transform.axes[1].cross(transform.axes[2])) < 0. {
            for t in &mut triangles {
                t.swap(1, 2);
            }
        }
        validate_closed(&vertices, &mut normals, &mut triangles)?;
        Ok(Shape::TriangleMesh {
            vertices,
            normals,
            triangles,
        })
    }
}
fn check_boundary(
    triangles: &[[usize; 3]],
    expected: &HashSet<(usize, usize)>,
    stage: &str,
) -> Result<(), String> {
    let mut counts = HashMap::new();
    for t in triangles {
        for j in 0..3 {
            *counts.entry(key(t[j], t[(j + 1) % 3])).or_insert(0) += 1;
        }
    }
    let actual: HashSet<_> = counts
        .into_iter()
        .filter(|(_, n)| *n == 1)
        .map(|(e, _)| e)
        .collect();
    if actual != *expected {
        return Err(format!(
            "STEP: {stage}で境界が変化しました（不足 {}、追加 {}）",
            expected.difference(&actual).count(),
            actual.difference(expected).count()
        ));
    }
    Ok(())
}

fn validate_closed(
    vertices: &[V3],
    normals: &mut [V3],
    triangles: &mut Vec<[u32; 3]>,
) -> Result<(), String> {
    if vertices.is_empty()
        || vertices.iter().any(|p| !p.finite())
        || normals.iter().any(|p| !p.finite())
    {
        return Err("STEP: 空または不正なメッシュです".into());
    }
    let min = V3(std::array::from_fn(|a| {
        vertices
            .iter()
            .map(|p| p.0[a])
            .fold(f64::INFINITY, f64::min)
    }));
    let max = V3(std::array::from_fn(|a| {
        vertices
            .iter()
            .map(|p| p.0[a])
            .fold(f64::NEG_INFINITY, f64::max)
    }));
    let epsilon = (max - min).norm() * 1e-10 + 1e-9;
    let origin = (min + max) * 0.5;
    let mut cells: HashMap<[i64; 3], Vec<(usize, V3)>> = HashMap::new();
    let mut unions = (0..vertices.len()).collect::<Vec<_>>();
    let mut comparisons = 0usize;
    for (index, &p) in vertices.iter().enumerate() {
        let cell = (p - origin).0.map(|v| (v / epsilon).round() as i64);
        let mut duplicate = false;
        for x in -1..=1 {
            for y in -1..=1 {
                for z in -1..=1 {
                    if let Some(items) = cells.get(&[cell[0] + x, cell[1] + y, cell[2] + z]) {
                        for &(id, q) in items {
                            comparisons += 1;
                            if comparisons > 100_000_000 {
                                return Err("STEP: 縮退点の接続検査が上限に達しました".into());
                            }
                            if p == q {
                                duplicate = true;
                            }
                            if (p - q).norm() <= epsilon {
                                let a = root(&mut unions, index);
                                let b = root(&mut unions, id);
                                unions[a] = b;
                            }
                        }
                    }
                }
            }
        }
        if !duplicate {
            cells.entry(cell).or_default().push((index, p));
        }
    }
    let welded = (0..vertices.len())
        .map(|i| root(&mut unions, i))
        .collect::<Vec<_>>();
    triangles.retain(|t| {
        let [a, b, c] = t.map(|i| welded[i as usize]);
        a != b && b != c && c != a
    });
    let mut edges: HashMap<(usize, usize), (usize, i32)> = HashMap::new();
    let mut parents = (0..vertices.len()).collect::<Vec<_>>();
    fn root(parents: &mut [usize], mut i: usize) -> usize {
        while parents[i] != i {
            parents[i] = parents[parents[i]];
            i = parents[i];
        }
        i
    }
    let mut volume = 0.;
    for t in triangles.iter() {
        let [a, b, c] = t.map(|i| vertices[i as usize] - origin);
        volume += a.dot(b.cross(c)) / 6.;
        for j in 0..3 {
            let (a, b) = (welded[t[j] as usize], welded[t[(j + 1) % 3] as usize]);
            let ra = root(&mut parents, a);
            let rb = root(&mut parents, b);
            parents[ra] = rb;
            let entry = edges.entry(key(a, b)).or_default();
            entry.0 += 1;
            entry.1 += if a < b { 1 } else { -1 };
        }
    }
    let open = edges.values().filter(|(count, _)| *count != 2).count();
    let reversed = edges
        .values()
        .filter(|(count, balance)| *count == 2 && *balance != 0)
        .count();
    if open > 0 || reversed > 0 {
        return Err(format!("STEP: 閉じたソリッドを復元できません（未接続・非多様体の辺 {open}、向きの不一致 {reversed}）"));
    }
    let components = triangles
        .iter()
        .flatten()
        .map(|&i| root(&mut parents, welded[i as usize]))
        .collect::<HashSet<_>>();
    if components.len() != 1 {
        return Err("STEP: 接続した単一のソリッドが必要です".into());
    }
    if !volume.is_finite() || volume.abs() < epsilon.powi(3) {
        return Err("STEP: ソリッドの体積がゼロまたは不正です".into());
    }
    if volume < 0. {
        for t in triangles {
            t.swap(1, 2);
        }
        for n in normals {
            *n = -*n;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn vertex_welding_joins_transitive_tolerance_neighbors() {
        let corners = [
            V3([0., 0., 0.]),
            V3([1., 0., 0.]),
            V3([0., 1., 0.]),
            V3([0., 0., 1.]),
        ];
        let faces = [[0, 2, 1], [0, 1, 3], [0, 3, 2], [1, 2, 3]];
        let mut points = Vec::new();
        let mut triangles = Vec::new();
        for (j, face) in faces.iter().enumerate() {
            triangles.push(std::array::from_fn(|k| (points.len() + k) as u32));
            for &i in face {
                let mut p = corners[i];
                if i == 0 {
                    p.0[0] = j as f64 * 0.85e-9;
                }
                points.push(p);
            }
        }
        let mut normals = vec![V3([1., 0., 0.]); points.len()];
        validate_closed(&points, &mut normals, &mut triangles).unwrap();
        // Two disjoint closed shells must fail the single-solid contract.
        let offset = points.len() as u32;
        let mut second = points
            .iter()
            .map(|p| *p + V3([5., 0., 0.]))
            .collect::<Vec<_>>();
        points.append(&mut second);
        triangles.extend(triangles.clone().into_iter().map(|t| t.map(|i| i + offset)));
        normals.resize(points.len(), V3([1., 0., 0.]));
        assert!(validate_closed(&points, &mut normals, &mut triangles)
            .unwrap_err()
            .contains("単一"));
    }
}
