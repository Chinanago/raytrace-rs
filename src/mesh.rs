//! Triangle BVH for imported closed optical CAD solids.
use crate::{math::V3, model::Shape};
use std::sync::Arc;
#[derive(Clone, Debug)]
struct Node {
    min: V3,
    max: V3,
    range: std::ops::Range<usize>,
    children: Option<[usize; 2]>,
}
#[derive(Clone, Debug)]
struct Data {
    vertices: Vec<V3>,
    normals: Vec<V3>,
    triangles: Vec<[u32; 3]>,
    order: Vec<usize>,
    nodes: Vec<Node>,
}
#[derive(Clone, Debug)]
pub struct MeshAccel(Arc<Data>);
fn intersects(min: V3, max: V3, p: V3, d: V3, limit: f64) -> bool {
    let mut lo: f64 = 1e-8;
    let mut hi = limit;
    for a in 0..3 {
        if d.0[a].abs() < 1e-15 {
            if p.0[a] < min.0[a] - 1e-10 || p.0[a] > max.0[a] + 1e-10 {
                return false;
            }
        } else {
            let t0 = (min.0[a] - p.0[a]) / d.0[a];
            let t1 = (max.0[a] - p.0[a]) / d.0[a];
            lo = lo.max(t0.min(t1));
            hi = hi.min(t0.max(t1));
        }
    }
    lo <= hi + 1e-10
}
impl Data {
    fn build(&mut self, start: usize, end: usize) -> usize {
        let mut min = V3([f64::INFINITY; 3]);
        let mut max = V3([f64::NEG_INFINITY; 3]);
        for k in start..end {
            for i in self.triangles[self.order[k]] {
                for a in 0..3 {
                    min.0[a] = min.0[a].min(self.vertices[i as usize].0[a]);
                    max.0[a] = max.0[a].max(self.vertices[i as usize].0[a]);
                }
            }
        }
        let index = self.nodes.len();
        self.nodes.push(Node {
            min,
            max,
            range: start..end,
            children: None,
        });
        if end - start > 8 {
            let axis = (0..3)
                .max_by(|&a, &b| (max.0[a] - min.0[a]).total_cmp(&(max.0[b] - min.0[b])))
                .unwrap();
            let triangles = &self.triangles;
            let vertices = &self.vertices;
            self.order[start..end].sort_unstable_by(|&a, &b| {
                let centre = |i: usize| {
                    triangles[i]
                        .iter()
                        .map(|&v| vertices[v as usize].0[axis])
                        .sum::<f64>()
                };
                centre(a).total_cmp(&centre(b))
            });
            let mid = (start + end) / 2;
            let left = self.build(start, mid);
            let right = self.build(mid, end);
            self.nodes[index].children = Some([left, right]);
        }
        index
    }
}
impl MeshAccel {
    pub fn from_shape(shape: &Shape) -> Option<Self> {
        let Shape::TriangleMesh {
            vertices,
            triangles,
            normals,
        } = shape
        else {
            return None;
        };
        let mut data = Data {
            vertices: vertices.clone(),
            normals: normals.clone(),
            triangles: triangles.clone(),
            order: (0..triangles.len()).collect(),
            nodes: Vec::new(),
        };
        data.build(0, triangles.len());
        Some(Self(Arc::new(data)))
    }
    pub fn hit(&self, p: V3, d: V3) -> Option<(f64, V3)> {
        let data = &self.0;
        // A balanced tree over <=1 million triangles has depth below 32.
        let mut stack = [0usize; 64];
        let mut stack_len = 1;
        let mut nearest = f64::INFINITY;
        let mut normal = None;
        while stack_len > 0 {
            stack_len -= 1;
            let i = stack[stack_len];
            let node = &data.nodes[i];
            if !intersects(node.min, node.max, p, d, nearest) {
                continue;
            }
            if let Some(children) = node.children {
                stack[stack_len] = children[0];
                stack[stack_len + 1] = children[1];
                stack_len += 2;
                continue;
            }
            for k in node.range.clone() {
                let ids = data.triangles[data.order[k]];
                let a = data.vertices[ids[0] as usize];
                let e1 = data.vertices[ids[1] as usize] - a;
                let e2 = data.vertices[ids[2] as usize] - a;
                let h = d.cross(e2);
                let determinant = e1.dot(h);
                if determinant.abs() < 1e-14 {
                    continue;
                }
                let inv = 1.0 / determinant;
                let s = p - a;
                let u = inv * s.dot(h);
                if !(-1e-10..=1.0 + 1e-10).contains(&u) {
                    continue;
                }
                let q = s.cross(e1);
                let v = inv * d.dot(q);
                if v < -1e-10 || u + v > 1.0 + 1e-10 {
                    continue;
                }
                let t = inv * e2.dot(q);
                if t > 1e-8 && t < nearest {
                    nearest = t;
                    normal = Some(
                        (data.normals[ids[0] as usize] * (1.0 - u - v)
                            + data.normals[ids[1] as usize] * u
                            + data.normals[ids[2] as usize] * v)
                            .unit(),
                    );
                }
            }
        }
        normal.map(|n| (nearest, n))
    }
}
