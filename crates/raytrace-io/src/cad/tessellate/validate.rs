use super::*;

pub(super) fn check_boundary(
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

pub(super) fn validate_closed(
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
