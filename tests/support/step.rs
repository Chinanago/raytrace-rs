//! Hand-authored test geometry, generated at runtime without a CAD application.
use std::collections::HashMap;

pub struct Builder {
    records: Vec<String>,
    points: Vec<[f64; 3]>,
    vertices: Vec<usize>,
    edges: HashMap<(usize, usize), (usize, usize, usize)>,
}
fn refs(ids: &[usize]) -> String {
    format!(
        "({})",
        ids.iter()
            .map(|i| format!("#{i}"))
            .collect::<Vec<_>>()
            .join(",")
    )
}
fn numbers(values: &[f64]) -> String {
    format!(
        "({})",
        values
            .iter()
            .map(|x| format!("{x:.17E}"))
            .collect::<Vec<_>>()
            .join(",")
    )
}
impl Builder {
    pub fn new() -> Self {
        Self {
            records: Vec::new(),
            points: Vec::new(),
            vertices: Vec::new(),
            edges: HashMap::new(),
        }
    }
    pub fn add(&mut self, s: impl Into<String>) -> usize {
        self.records.push(s.into());
        self.records.len()
    }
    pub fn point(&mut self, p: [f64; 3]) -> usize {
        self.add(format!("CARTESIAN_POINT('',{})", numbers(&p)))
    }
    pub fn direction(&mut self, p: [f64; 3]) -> usize {
        self.add(format!("DIRECTION('',{})", numbers(&p)))
    }
    pub fn frame(&mut self, p: [f64; 3], z: [f64; 3], x: [f64; 3]) -> usize {
        let p = self.point(p);
        let z = self.direction(z);
        let x = self.direction(x);
        self.add(format!("AXIS2_PLACEMENT_3D('',#{p},#{z},#{x})"))
    }
    pub fn vertex(&mut self, p: [f64; 3]) -> usize {
        let point = self.point(p);
        let vertex = self.add(format!("VERTEX_POINT('',#{point})"));
        self.points.push(p);
        self.vertices.push(vertex);
        self.points.len() - 1
    }
    pub fn line(&mut self, a: usize, b: usize) -> usize {
        let p = self.point(self.points[a]);
        let d = std::array::from_fn(|i| self.points[b][i] - self.points[a][i]);
        let length = d.iter().map(|x| x * x).sum::<f64>().sqrt();
        let direction = self.direction(d.map(|x| x / length));
        let v = self.add(format!("VECTOR('',#{direction},{length})"));
        self.add(format!("LINE('',#{p},#{v})"))
    }
    pub fn edge(&mut self, a: usize, b: usize, curve: Option<usize>) -> usize {
        let key = if a < b { (a, b) } else { (b, a) };
        if let Some(&(id, _, _)) = self.edges.get(&key) {
            return id;
        }
        let curve = curve.unwrap_or_else(|| self.line(a, b));
        let id = self.add(format!(
            "EDGE_CURVE('',#{},#{},#{curve},.T.)",
            self.vertices[a], self.vertices[b]
        ));
        self.edges.insert(key, (id, a, b));
        id
    }
    pub fn oriented(&mut self, edge: usize, forward: bool) -> usize {
        self.add(format!(
            "ORIENTED_EDGE('',*,*,#{edge},.{}.)",
            if forward { 'T' } else { 'F' }
        ))
    }
    pub fn loop_edges(&mut self, edges: &[(usize, bool)]) -> usize {
        let edges = edges
            .iter()
            .map(|&(e, forward)| self.oriented(e, forward))
            .collect::<Vec<_>>();
        self.add(format!("EDGE_LOOP('',{})", refs(&edges)))
    }
    pub fn face(&mut self, loops: &[usize], surface: usize, sense: bool) -> usize {
        let bounds = loops
            .iter()
            .map(|l| self.add(format!("FACE_BOUND('',#{l},.T.)")))
            .collect::<Vec<_>>();
        self.add(format!(
            "ADVANCED_FACE('',{},#{surface},.{}.)",
            refs(&bounds),
            if sense { 'T' } else { 'F' }
        ))
    }
    pub fn polygon_face(&mut self, vertices: &[usize], surface: Option<usize>) -> usize {
        let mut edges = Vec::new();
        for i in 0..vertices.len() {
            let (a, b) = (vertices[i], vertices[(i + 1) % vertices.len()]);
            let edge = self.edge(a, b, None);
            let (_, start, end) = self.edges[&if a < b { (a, b) } else { (b, a) }];
            edges.push((edge, start == a && end == b));
        }
        let surface = surface.unwrap_or_else(|| {
            let a = self.points[vertices[0]];
            let b = self.points[vertices[1]];
            let c = self.points[*vertices.last().unwrap()];
            let x = std::array::from_fn(|i| b[i] - a[i]);
            let y: [f64; 3] = std::array::from_fn(|i| c[i] - a[i]);
            let z = [
                x[1] * y[2] - x[2] * y[1],
                x[2] * y[0] - x[0] * y[2],
                x[0] * y[1] - x[1] * y[0],
            ];
            let f = self.frame(a, z, x);
            self.add(format!("PLANE('',#{f})"))
        });
        let l = self.loop_edges(&edges);
        self.face(&[l], surface, true)
    }
    pub fn spline_curve(
        &mut self,
        points: &[[f64; 3]],
        weights: &[f64],
        degree: usize,
        mults: &[f64],
        knots: &[f64],
    ) -> usize {
        let points = points.iter().map(|&p| self.point(p)).collect::<Vec<_>>();
        self.add(format!("(BOUNDED_CURVE() B_SPLINE_CURVE({degree},{},.UNSPECIFIED.,.F.,.F.) B_SPLINE_CURVE_WITH_KNOTS({},{},.UNSPECIFIED.) CURVE() GEOMETRIC_REPRESENTATION_ITEM() RATIONAL_B_SPLINE_CURVE({}) REPRESENTATION_ITEM(''))",refs(&points),numbers(mults),numbers(knots),numbers(weights)))
    }
    pub fn spline_surface(
        &mut self,
        points: &[Vec<[f64; 3]>],
        weights: &[Vec<f64>],
        degrees: [usize; 2],
        multiplicities: [&[f64]; 2],
        knots: [&[f64]; 2],
        closed: [bool; 2],
    ) -> usize {
        let points = points
            .iter()
            .map(|row| row.iter().map(|&p| self.point(p)).collect::<Vec<_>>())
            .collect::<Vec<_>>();
        let cp = format!(
            "({})",
            points
                .iter()
                .map(|row| refs(row))
                .collect::<Vec<_>>()
                .join(",")
        );
        let w = format!(
            "({})",
            weights
                .iter()
                .map(|row| numbers(row))
                .collect::<Vec<_>>()
                .join(",")
        );
        self.add(format!("(BOUNDED_SURFACE() B_SPLINE_SURFACE({},{},{cp},.UNSPECIFIED.,.{}.,.{}.,.F.) B_SPLINE_SURFACE_WITH_KNOTS({},{},{},{},.UNSPECIFIED.) GEOMETRIC_REPRESENTATION_ITEM() RATIONAL_B_SPLINE_SURFACE({w}) REPRESENTATION_ITEM('') SURFACE())",degrees[0],degrees[1],if closed[0]{'T'}else{'F'},if closed[1]{'T'}else{'F'},numbers(multiplicities[0]),numbers(multiplicities[1]),numbers(knots[0]),numbers(knots[1])))
    }
    pub fn finish(mut self, faces: &[usize], inch: bool) -> String {
        let shell = self.add(format!("CLOSED_SHELL('',{})", refs(faces)));
        let solid = self.add(format!("MANIFOLD_SOLID_BREP('Owned solid',#{shell})"));
        let metre = self.add("(LENGTH_UNIT() NAMED_UNIT(*) SI_UNIT($,.METRE.))");
        let unit = if inch {
            let measure = self.add(format!(
                "LENGTH_MEASURE_WITH_UNIT(LENGTH_MEASURE(0.0254),#{metre})"
            ));
            self.add(format!(
                "(CONVERSION_BASED_UNIT('INCH',#{measure}) LENGTH_UNIT() NAMED_UNIT(*))"
            ))
        } else {
            self.add("(LENGTH_UNIT() NAMED_UNIT(*) SI_UNIT(.MILLI.,.METRE.))")
        };
        let angle = self.add("(PLANE_ANGLE_UNIT() NAMED_UNIT(*) SI_UNIT($,.RADIAN.))");
        let solid_angle = self.add("(SOLID_ANGLE_UNIT() NAMED_UNIT(*) SI_UNIT($,.STERADIAN.))");
        let context=self.add(format!("(GEOMETRIC_REPRESENTATION_CONTEXT(3) GLOBAL_UNIT_ASSIGNED_CONTEXT((#{unit},#{angle},#{solid_angle})) REPRESENTATION_CONTEXT('',''))"));
        let representation = self.add(format!(
            "ADVANCED_BREP_SHAPE_REPRESENTATION('',(#{solid}),#{context})"
        ));
        let application = self.add("APPLICATION_CONTEXT('mechanical design')");
        self.add(format!("APPLICATION_PROTOCOL_DEFINITION('international standard','automotive_design',2000,#{application})"));
        let product_context = self.add(format!("PRODUCT_CONTEXT('',#{application},'mechanical')"));
        let product = self.add(format!("PRODUCT('owned','owned','',(#{product_context}))"));
        let formation = self.add(format!("PRODUCT_DEFINITION_FORMATION('','',#{product})"));
        let definition_context = self.add(format!(
            "PRODUCT_DEFINITION_CONTEXT('part definition',#{application},'design')"
        ));
        let definition = self.add(format!(
            "PRODUCT_DEFINITION('design','',#{formation},#{definition_context})"
        ));
        let shape = self.add(format!("PRODUCT_DEFINITION_SHAPE('','',#{definition})"));
        self.add(format!(
            "SHAPE_DEFINITION_REPRESENTATION(#{shape},#{representation})"
        ));
        let mut text="ISO-10303-21;\nHEADER;\nFILE_DESCRIPTION(('Owned raytrace test'),'2;1');\nFILE_NAME('owned.step','',(''),(''),'raytrace tests','','');\nFILE_SCHEMA(('AUTOMOTIVE_DESIGN'));\nENDSEC;\nDATA;\n".to_string();
        for (i, s) in self.records.iter().enumerate() {
            text.push_str(&format!("#{}={s};\n", i + 1));
        }
        text.push_str("ENDSEC;\nEND-ISO-10303-21;\n");
        text
    }
}
pub fn box_step(inch: bool) -> String {
    let mut b = Builder::new();
    for p in [
        [-1., -2., 0.],
        [1., -2., 0.],
        [1., 2., 0.],
        [-1., 2., 0.],
        [-1., -2., 3.],
        [1., -2., 3.],
        [1., 2., 3.],
        [-1., 2., 3.],
    ] {
        b.vertex(p);
    }
    let faces = [
        [0, 3, 2, 1],
        [0, 1, 5, 4],
        [1, 2, 6, 5],
        [2, 3, 7, 6],
        [3, 0, 4, 7],
        [4, 5, 6, 7],
    ]
    .map(|v| b.polygon_face(&v, None));
    b.finish(&faces, inch)
}
pub fn nurbs_dome_step() -> String {
    let mut b = Builder::new();
    let r = 3.;
    let q = 0.5f64.sqrt();
    let tip = b.vertex([0., 0., r]);
    let rim = b.vertex([r, 0., 0.]);
    let circle_points = [
        [1., 0.],
        [1., 1.],
        [0., 1.],
        [-1., 1.],
        [-1., 0.],
        [-1., -1.],
        [0., -1.],
        [1., -1.],
        [1., 0.],
    ];
    let circle_weights = [1., q, 1., q, 1., q, 1., q, 1.];
    let profile = [[0., r], [r, r], [r, 0.]];
    let profile_weights = [1., q, 1.];
    let points = profile
        .iter()
        .map(|p| {
            circle_points
                .iter()
                .map(|c| [p[0] * c[0], p[0] * c[1], p[1]])
                .collect::<Vec<_>>()
        })
        .collect::<Vec<_>>();
    let weights = profile_weights
        .iter()
        .map(|w| circle_weights.iter().map(|v| w * v).collect::<Vec<_>>())
        .collect::<Vec<_>>();
    let top = b.spline_surface(
        &points,
        &weights,
        [2, 2],
        [&[3., 3.], &[3., 2., 2., 2., 3.]],
        [&[0., 1.], &[0., 0.25, 0.5, 0.75, 1.]],
        [false, true],
    );
    let meridian = b.spline_curve(
        &[[0., 0., r], [r, 0., r], [r, 0., 0.]],
        &profile_weights,
        2,
        &[3., 3.],
        &[0., 1.],
    );
    let seam = b.edge(tip, rim, Some(meridian));
    let f = b.frame([0., 0., 0.], [0., 0., 1.], [1., 0., 0.]);
    let circle = b.add(format!("CIRCLE('',#{f},{r})"));
    let rim_edge = b.edge(rim, rim, Some(circle));
    let top_loop = b.loop_edges(&[(seam, false), (seam, true), (rim_edge, true)]);
    let top_face = b.face(&[top_loop], top, true);
    let plane = b.add(format!("PLANE('',#{f})"));
    let bottom_loop = b.loop_edges(&[(rim_edge, false)]);
    let bottom = b.face(&[bottom_loop], plane, false);
    b.finish(&[top_face, bottom], false)
}
pub fn owned_cases() -> Vec<(&'static str, String)> {
    vec![
        ("box_mm", box_step(false)),
        ("box_inch", box_step(true)),
        ("rational_periodic_dome", nurbs_dome_step()),
        ("tube", tube_step()),
        ("sphere_cap", sphere_cap_step()),
        ("torus", torus_step()),
        ("cone", cone_step()),
    ]
}
pub fn tube_step() -> String {
    let mut b = Builder::new();
    let mut rings = Vec::new();
    let mut faces = Vec::new();
    for radius in [2.5, 1.] {
        let bottom = b.vertex([radius, 0., 0.]);
        let top = b.vertex([radius, 0., 4.]);
        let f0 = b.frame([0., 0., 0.], [0., 0., 1.], [1., 0., 0.]);
        let f1 = b.frame([0., 0., 4.], [0., 0., 1.], [1., 0., 0.]);
        let c0 = b.add(format!("CIRCLE('',#{f0},{radius})"));
        let c1 = b.add(format!("CIRCLE('',#{f1},{radius})"));
        let e0 = b.edge(bottom, bottom, Some(c0));
        let e1 = b.edge(top, top, Some(c1));
        let seam = b.edge(bottom, top, None);
        let l = b.loop_edges(&[(e0, true), (seam, true), (e1, false), (seam, false)]);
        let surface = b.add(format!("CYLINDRICAL_SURFACE('',#{f0},{radius})"));
        faces.push(b.face(&[l], surface, radius > 2.));
        rings.push((e0, e1, f0, f1));
    }
    for (top, sense) in [(false, false), (true, true)] {
        let outer = if top { rings[0].1 } else { rings[0].0 };
        let inner = if top { rings[1].1 } else { rings[1].0 };
        let f = if top { rings[0].3 } else { rings[0].2 };
        let plane = b.add(format!("PLANE('',#{f})"));
        let a = b.loop_edges(&[(outer, top)]);
        let hole = b.loop_edges(&[(inner, !top)]);
        faces.push(b.face(&[a, hole], plane, sense));
    }
    b.finish(&faces, false)
}
pub fn sphere_cap_step() -> String {
    let mut b = Builder::new();
    let radius = 3.;
    let lower = b.vertex([radius, 0., -2.]);
    let upper = b.vertex([radius, 0., 0.]);
    let f0 = b.frame([0., 0., -2.], [0., 0., 1.], [1., 0., 0.]);
    let f1 = b.frame([0., 0., 0.], [0., 0., 1.], [1., 0., 0.]);
    let c0 = b.add(format!("CIRCLE('',#{f0},{radius})"));
    let c1 = b.add(format!("CIRCLE('',#{f1},{radius})"));
    let e0 = b.edge(lower, lower, Some(c0));
    let e1 = b.edge(upper, upper, Some(c1));
    let seam = b.edge(lower, upper, None);
    let side = b.add(format!("CYLINDRICAL_SURFACE('',#{f1},{radius})"));
    let l = b.loop_edges(&[(e0, true), (seam, true), (e1, false), (seam, false)]);
    let side = b.face(&[l], side, true);
    let plane = b.add(format!("PLANE('',#{f0})"));
    let l = b.loop_edges(&[(e0, false)]);
    let bottom = b.face(&[l], plane, false);
    let center = b.frame([0., 0., -4.], [0., 0., 1.], [1., 0., 0.]);
    let sphere = b.add(format!("SPHERICAL_SURFACE('',#{center},5.)"));
    let l = b.loop_edges(&[(e1, true)]);
    let top = b.face(&[l], sphere, true);
    b.finish(&[side, bottom, top], false)
}
pub fn torus_step() -> String {
    let mut b = Builder::new();
    let vertex = b.vertex([4., 0., 0.]);
    let f = b.frame([0., 0., 0.], [0., 0., 1.], [1., 0., 0.]);
    let major = b.add(format!("CIRCLE('',#{f},4.)"));
    let f2 = b.frame([3., 0., 0.], [0., -1., 0.], [1., 0., 0.]);
    let minor = b.add(format!("CIRCLE('',#{f2},1.)"));
    let e1 = b.edge(vertex, vertex, Some(major));
    let e2 = b.add(format!(
        "EDGE_CURVE('',#{},#{},#{minor},.T.)",
        b.vertices[vertex], b.vertices[vertex]
    ));
    let l = b.loop_edges(&[(e1, true), (e2, true), (e1, false), (e2, false)]);
    let surface = b.add(format!("TOROIDAL_SURFACE('',#{f},3.,1.)"));
    let face = b.face(&[l], surface, true);
    b.finish(&[face], false)
}
pub fn cone_step() -> String {
    let mut b = Builder::new();
    let lower = b.vertex([2.5, 0., 0.]);
    let upper = b.vertex([1.5, 0., 4.]);
    let f0 = b.frame([0., 0., 0.], [0., 0., 1.], [1., 0., 0.]);
    let f1 = b.frame([0., 0., 4.], [0., 0., 1.], [1., 0., 0.]);
    let c0 = b.add(format!("CIRCLE('',#{f0},2.5)"));
    let c1 = b.add(format!("CIRCLE('',#{f1},1.5)"));
    let e0 = b.edge(lower, lower, Some(c0));
    let e1 = b.edge(upper, upper, Some(c1));
    let seam = b.edge(lower, upper, None);
    let f = b.frame([0., 0., 4.], [0., 0., -1.], [1., 0., 0.]);
    let angle = 0.25f64.atan();
    let surface = b.add(format!("CONICAL_SURFACE('',#{f},1.5,{angle})"));
    let l = b.loop_edges(&[(e0, true), (seam, true), (e1, false), (seam, false)]);
    let side = b.face(&[l], surface, true);
    let bottom = b.add(format!("PLANE('',#{f0})"));
    let l = b.loop_edges(&[(e0, false)]);
    let bottom = b.face(&[l], bottom, false);
    let top = b.add(format!("PLANE('',#{f1})"));
    let l = b.loop_edges(&[(e1, true)]);
    let top = b.face(&[l], top, true);
    b.finish(&[side, bottom, top], false)
}
pub fn mapped_box_step() -> String {
    let source = box_step(false);
    let representation = source
        .lines()
        .find(|s| s.contains("=ADVANCED_BREP_SHAPE_REPRESENTATION"))
        .unwrap();
    let id = representation[1..]
        .split('=')
        .next()
        .unwrap()
        .parse::<usize>()
        .unwrap();
    let mut n = source
        .lines()
        .filter_map(|line| {
            line.strip_prefix('#')?
                .split('=')
                .next()?
                .parse::<usize>()
                .ok()
        })
        .max()
        .unwrap();
    let mut records = String::new();
    let mut add = |s: String| {
        n += 1;
        records.push_str(&format!("#{n}={s};\n"));
        n
    };
    let origin = add("CARTESIAN_POINT('',(0.,0.,0.))".into());
    let base = add(format!("AXIS2_PLACEMENT_3D('',#{origin},$,$)"));
    let p = add("CARTESIAN_POINT('',(7.,11.,13.))".into());
    let z = add("DIRECTION('',(0.,1.,0.))".into());
    let x = add("DIRECTION('',(1.,0.,0.))".into());
    let y = add("DIRECTION('',(0.,0.,-1.))".into());
    let target = add(format!(
        "CARTESIAN_TRANSFORMATION_OPERATOR_3D('','',#{x},#{y},#{p},1.,#{z})"
    ));
    let map = add(format!("REPRESENTATION_MAP(#{base},#{id})"));
    let item = add(format!("MAPPED_ITEM('',#{map},#{target})"));
    let context = representation
        .rsplit(",#")
        .next()
        .unwrap()
        .trim_end_matches(");");
    let parent = add(format!("SHAPE_REPRESENTATION('',(#{item}),#{context})"));
    let end = source.rfind("ENDSEC;").unwrap();
    let text = format!("{}{}{}", &source[..end], records, &source[end..]);
    let definition = source
        .lines()
        .find(|line| line.contains("=SHAPE_DEFINITION_REPRESENTATION("))
        .unwrap();
    let updated = definition.replace(&format!(",#{id});"), &format!(",#{parent});"));
    text.replace(definition, &updated)
}
