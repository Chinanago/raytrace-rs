//! Conservative world-space broad phase; the original surface solvers remain authoritative.
use crate::{
    geometry::{intersect, search_start, Hit, Miss, RAY_EPS_MM},
    math::V3,
    model::PreparedSurface,
};

pub(crate) type Selection = Result<Option<(usize, Hit)>, (Miss, Option<usize>)>;

#[derive(Clone, Copy)]
struct Bounds {
    min: V3,
    max: V3,
    padding: f64,
}
impl Bounds {
    fn new(min: V3, max: V3) -> Self {
        let scale = min
            .0
            .into_iter()
            .chain(max.0)
            .map(f64::abs)
            .fold(1.0, f64::max);
        let span = (0..3).map(|a| max.0[a] - min.0[a]).fold(0.0, f64::max);
        Self {
            min,
            max,
            padding: 512.0 * f64::EPSILON * scale + 1e-10 * (1.0 + span),
        }
    }
    fn surface(surface: &PreparedSurface) -> Option<Self> {
        let corners: [V3; 8] = std::array::from_fn(|i| {
            surface.frame.world_point(V3(std::array::from_fn(|axis| {
                surface.local_bounds[(i >> axis) & 1].0[axis]
            })))
        });
        if corners.iter().any(|p| !p.finite()) {
            return None;
        }
        Some(Self::new(
            V3(std::array::from_fn(|a| {
                corners.iter().map(|p| p.0[a]).fold(f64::INFINITY, f64::min)
            })),
            V3(std::array::from_fn(|a| {
                corners
                    .iter()
                    .map(|p| p.0[a])
                    .fold(f64::NEG_INFINITY, f64::max)
            })),
        ))
    }
    fn union(self, other: Self) -> Self {
        Self::new(
            V3(std::array::from_fn(|a| self.min.0[a].min(other.min.0[a]))),
            V3(std::array::from_fn(|a| self.max.0[a].max(other.max.0[a]))),
        )
    }
    fn entry(self, ray: &BoundsRay, limit: f64) -> Option<f64> {
        // Cover local solver tolerances, inverse-frame roundoff and mesh edge tolerances.
        // These margins only admit extra candidates; they never alter the actual hit test.
        let padding = self.padding + ray.padding;
        let mut low = RAY_EPS_MM;
        let mut high = limit;
        for a in 0..3 {
            if ray.direction.0[a] == 0.0 {
                if ray.origin.0[a] < self.min.0[a] - padding
                    || ray.origin.0[a] > self.max.0[a] + padding
                {
                    return None;
                }
            } else {
                let near = self.min.0[a] - padding - ray.origin.0[a];
                let far = self.max.0[a] + padding - ray.origin.0[a];
                let (t0, t1) = if ray.inverse.0[a].is_finite() {
                    (near * ray.inverse.0[a], far * ray.inverse.0[a])
                } else {
                    // A nonzero subnormal direction may have an infinite reciprocal.
                    (near / ray.direction.0[a], far / ray.direction.0[a])
                };
                low = low.max(t0.min(t1));
                high = high.min(t0.max(t1));
            }
        }
        (low <= high).then_some(low)
    }
}

struct BoundsRay {
    origin: V3,
    direction: V3,
    inverse: V3,
    padding: f64,
}
impl BoundsRay {
    fn new(origin: V3, direction: V3) -> Self {
        Self {
            origin,
            direction,
            inverse: V3(direction.0.map(|d| if d == 0.0 { 0.0 } else { 1.0 / d })),
            padding: 512.0 * f64::EPSILON * origin.0.into_iter().map(f64::abs).fold(0.0, f64::max),
        }
    }
}

struct Node {
    bounds: Bounds,
    range: std::ops::Range<usize>,
    children: Option<[usize; 2]>,
}

pub(crate) struct SurfaceIndex {
    order: Vec<usize>,
    nodes: Vec<Node>,
}
impl SurfaceIndex {
    pub(crate) fn new(surfaces: &[PreparedSurface]) -> Option<Self> {
        // Small systems are cheaper to scan directly. Keep preparation/trace APIs unchanged.
        if surfaces.len() < 16 {
            return None;
        }
        let bounds: Vec<_> = surfaces
            .iter()
            .map(Bounds::surface)
            .collect::<Option<_>>()?;
        let mut index = Self {
            order: (0..surfaces.len()).collect(),
            nodes: Vec::with_capacity(surfaces.len()),
        };
        index.build(&bounds, 0, surfaces.len());
        Some(index)
    }
    fn build(&mut self, surfaces: &[Bounds], start: usize, end: usize) -> usize {
        let bounds = self.order[start + 1..end]
            .iter()
            .fold(surfaces[self.order[start]], |b, &i| b.union(surfaces[i]));
        let index = self.nodes.len();
        self.nodes.push(Node {
            bounds,
            range: start..end,
            children: None,
        });
        if end - start > 4 {
            let axis = (0..3)
                .max_by(|&a, &b| {
                    (bounds.max.0[a] - bounds.min.0[a])
                        .total_cmp(&(bounds.max.0[b] - bounds.min.0[b]))
                })
                .unwrap();
            let mid = (start + end) / 2;
            self.order[start..end].select_nth_unstable_by(mid - start, |&a, &b| {
                let centre =
                    |i: usize| 0.5 * surfaces[i].min.0[axis] + 0.5 * surfaces[i].max.0[axis];
                centre(a).total_cmp(&centre(b)).then(a.cmp(&b))
            });
            let left = self.build(surfaces, start, mid);
            let right = self.build(surfaces, mid, end);
            self.nodes[index].children = Some([left, right]);
        }
        index
    }
    fn visit(&self, p: V3, d: V3, mut test: impl FnMut(usize) -> f64) {
        let ray = BoundsRay::new(p, d);
        let Some(entry) = self.nodes[0].bounds.entry(&ray, f64::INFINITY) else {
            return;
        };
        // A balanced tree over any addressable surface array fits in this stack.
        let mut stack = [(0usize, 0.0); 64];
        stack[0] = (0, entry);
        let mut len = 1;
        let mut limit = f64::INFINITY;
        while len != 0 {
            len -= 1;
            let (i, entry) = stack[len];
            if entry > limit {
                continue;
            }
            let node = &self.nodes[i];
            if let Some([left, right]) = node.children {
                let a = self.nodes[left].bounds.entry(&ray, limit);
                let b = self.nodes[right].bounds.entry(&ray, limit);
                match (a, b) {
                    (Some(a), Some(b)) => {
                        let (near, far) = if a <= b {
                            ((left, a), (right, b))
                        } else {
                            ((right, b), (left, a))
                        };
                        stack[len] = far;
                        stack[len + 1] = near;
                        len += 2;
                    }
                    (Some(a), None) => {
                        stack[len] = (left, a);
                        len += 1;
                    }
                    (None, Some(b)) => {
                        stack[len] = (right, b);
                        len += 1;
                    }
                    (None, None) => (),
                }
            } else {
                for k in node.range.clone() {
                    limit = test(self.order[k]);
                }
            }
        }
    }
}

pub(crate) fn nearest(
    surfaces: &[PreparedSurface],
    index: Option<&SurfaceIndex>,
    p: V3,
    d: V3,
) -> Selection {
    let mut hit: Option<(usize, Hit)> = None;
    let mut numerical: Option<(usize, f64)> = None;
    let mut test = |i: usize| {
        let surface = &surfaces[i];
        match intersect(surface, p, d) {
            Ok(candidate)
                if hit.as_ref().is_none_or(|(j, h)| {
                    candidate.distance_mm < h.distance_mm
                        || candidate.distance_mm == h.distance_mm && i < *j
                }) =>
            {
                hit = Some((i, candidate))
            }
            Err(Miss::Numerical) => {
                if let Some(start) = search_start(surface, p, d) {
                    if numerical
                        .is_none_or(|(j, previous)| start < previous || start == previous && i < j)
                    {
                        numerical = Some((i, start));
                    }
                }
            }
            _ => (),
        }
        hit.as_ref().map_or(f64::INFINITY, |(_, h)| h.distance_mm)
    };
    if let Some(index) = index {
        index.visit(p, d, &mut test);
    } else {
        for i in 0..surfaces.len() {
            test(i);
        }
    }
    if let Some((i, _)) =
        numerical.filter(|(_, start)| hit.as_ref().is_none_or(|(_, h)| *start < h.distance_mm))
    {
        Err((Miss::Numerical, Some(i)))
    } else {
        Ok(hit)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{math::Frame, model::Scene};
    use serde_json::json;

    fn fixture() -> Vec<PreparedSurface> {
        let surfaces: Vec<_> = (0..80).map(|i| json!({
            "name": format!("surface_{i}"),
            "pose": {"translation_mm": [(i%8) as f64*3.0, (i/8) as f64*3.0, 10.0 + (i%3) as f64], "rotation_deg": [i%11, i%7, i%13]},
            "shape": if i%4 == 0 { json!({"type":"sphere", "radius_mm":8.0}) } else if i%4 == 1 {json!({"type":"even_asphere", "radius_mm":10.0,"conic":-0.8,"coefficients":[0.0001]})} else {json!({"type":"plane"})},
            "aperture": {"type":"circle","radius_mm":1.4},
            "interaction": {"type":"absorb"}
        })).collect();
        serde_json::from_value::<Scene>(json!({
            "name":"index regression", "reference_plane":"launch", "mode":"non_sequential",
            "materials":[{"type":"constant", "n":1}],"surfaces":surfaces
        }))
        .unwrap()
        .prepare()
        .unwrap()
        .surfaces
    }

    fn assert_same(a: Selection, b: Selection) {
        match (a, b) {
            (Ok(Some((i, a))), Ok(Some((j, b)))) => {
                assert_eq!(i, j);
                assert_eq!(a.distance_mm.to_bits(), b.distance_mm.to_bits());
                assert_eq!(a.point, b.point);
                assert_eq!(a.normal, b.normal);
                assert_eq!(a.geometric_normal, b.geometric_normal);
                assert_eq!(a.local_point, b.local_point);
            }
            (Ok(None), Ok(None)) => (),
            (Err((Miss::Numerical, i)), Err((Miss::Numerical, j))) => assert_eq!(i, j),
            (a, b) => panic!("different selections: {a:?} and {b:?}"),
        }
    }

    #[test]
    fn indexed_selection_matches_linear_for_transforms_edges_and_distant_origins() {
        let surfaces = fixture();
        let index = SurfaceIndex::new(&surfaces).unwrap();
        for i in 0..10_000 {
            let p = V3([
                (i % 101) as f64 * 0.31 - 4.0,
                (i % 89) as f64 * 0.41 - 4.0,
                -3.0,
            ]);
            let d = V3([
                ((i % 7) as f64 - 3.0) * 0.03,
                ((i % 11) as f64 - 5.0) * 0.02,
                1.0,
            ])
            .unit();
            assert_same(
                nearest(&surfaces, None, p, d),
                nearest(&surfaces, Some(&index), p, d),
            );
        }
        for surface in &surfaces {
            let frame: &Frame = &surface.frame;
            for x in [-1.4, -1.4 + 1e-12, 0.0, 1.4 - 1e-12, 1.4] {
                for distance in [1.0, 1e6, 1e12] {
                    let p = frame.world_point(V3([x, 0.0, -distance]));
                    let d = frame.world_vector(V3([0.0, 0.0, 1.0]));
                    assert_same(
                        nearest(&surfaces, None, p, d),
                        nearest(&surfaces, Some(&index), p, d),
                    );
                }
            }
        }
    }

    #[test]
    fn coincident_hits_keep_original_surface_order() {
        let mut surfaces = fixture();
        for (i, surface) in surfaces.iter_mut().enumerate() {
            *surface = fixture()[0].clone();
            surface.spec.name = format!("coincident_{i}");
        }
        let index = SurfaceIndex::new(&surfaces).unwrap();
        let p = V3([0.0, 0.0, 0.0]);
        let d = V3([0.0, 0.0, 1.0]);
        assert_eq!(
            nearest(&surfaces, Some(&index), p, d).unwrap().unwrap().0,
            0
        );
        assert_same(
            nearest(&surfaces, None, p, d),
            nearest(&surfaces, Some(&index), p, d),
        );
    }
}
