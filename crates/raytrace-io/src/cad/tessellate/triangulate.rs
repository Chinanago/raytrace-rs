use super::*;
use crate::Result;

pub(super) fn area(points: &[[f64; 2]]) -> f64 {
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
pub(super) fn contains(points: &[[f64; 2]], p: [f64; 2]) -> bool {
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
pub(super) fn triangulate(
    vertices: &[Vertex],
    boundary: &HashSet<(usize, usize)>,
    rings: &[Vec<[f64; 2]>],
    metric: [f64; 2],
) -> Result<Vec<[usize; 3]>> {
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
        return Err(crate::Error::step("STEP: 面の展開範囲が不正です".into()));
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
    .map_err(|e| crate::Error::step(format!("STEP: 面の三角形分割に失敗しました: {e}")))?;
    if conflict || cdt.num_vertices() != vertices.len() {
        return Err(crate::Error::step(
            "STEP: 面境界が交差、または展開点が重複しています".into(),
        ));
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
                    return Err(crate::Error::step(
                        "STEP: 面境界の内外が矛盾しています".into(),
                    ));
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
                return Err(crate::Error::step(
                    "STEP: 面境界が閉じていないか交差しています".into(),
                ));
            }
        }
    }
    let triangles = cdt
        .inner_faces()
        .filter(|f| parity[f.fix().index()] == 1)
        .map(|f| f.vertices().map(|v| v.data().index))
        .collect::<Vec<_>>();
    if triangles.len() > MAX_TRIANGLES {
        return Err(crate::Error::step(
            "STEP: 曲面の分割数が100万三角形を超えました".into(),
        ));
    }
    Ok(triangles)
}
