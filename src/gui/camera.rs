use super::*;

#[derive(Clone, Copy, Debug, serde::Serialize)]
pub(super) struct Viewport {
    center: [f64; 2],
    pitch: [f64; 2],
}
impl Viewport {
    fn from_grid(g: [f64; 4], size: Vec2) -> Self {
        Self {
            center: [g[0] + g[2] * 0.5, g[1] + g[3] * 0.5],
            pitch: [g[2] / size.x.max(1.0) as f64, g[3] / size.y.max(1.0) as f64],
        }
    }
    fn grid(self, size: Vec2, equal: bool) -> [f64; 4] {
        let pitch = if equal {
            [self.pitch[0].max(self.pitch[1]); 2]
        } else {
            self.pitch
        };
        let span = [pitch[0] * size.x as f64, pitch[1] * size.y as f64];
        [
            self.center[0] - span[0] * 0.5,
            self.center[1] - span[1] * 0.5,
            span[0],
            span[1],
        ]
    }
}

#[derive(Clone, Copy)]
pub(super) enum CameraRequest {
    All,
    Selected,
    Zoom(f64),
}
#[derive(Default, serde::Serialize)]
pub(super) struct LayoutCamera {
    views: [Option<Viewport>; 3],
    #[serde(skip)]
    pub request: Option<CameraRequest>,
}
impl LayoutCamera {
    pub fn grid(
        &mut self,
        projection: usize,
        auto: [f64; 4],
        focus: [f64; 4],
        size: Vec2,
        equal: bool,
    ) -> [f64; 4] {
        match self.request.take() {
            Some(CameraRequest::All) => self.views[projection] = None,
            Some(CameraRequest::Selected) => {
                self.views[projection] = Some(Viewport::from_grid(focus, size))
            }
            Some(CameraRequest::Zoom(factor)) => {
                let g = self.views[projection].map_or(auto, |v| v.grid(size, equal));
                self.views[projection] =
                    Some(Viewport::from_grid(zoom_grid(g, [0.5, 0.5], factor), size));
            }
            None => (),
        }
        self.views[projection].map_or(auto, |v| v.grid(size, equal))
    }
    pub fn set(&mut self, projection: usize, grid: [f64; 4], size: Vec2) {
        self.views[projection] = Some(Viewport::from_grid(grid, size));
    }
}
pub(super) fn zoom_grid(g: [f64; 4], anchor: [f64; 2], factor: f64) -> [f64; 4] {
    let factor = factor
        .clamp(0.2, 5.0)
        .max(1e-9 / g[2].min(g[3]))
        .min(1e16 / g[2].max(g[3]));
    [
        g[0] + g[2] * anchor[0] * (1.0 - factor),
        g[1] + g[3] * anchor[1] * (1.0 - factor),
        g[2] * factor,
        g[3] * factor,
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn zoom_preserves_the_world_point_under_the_cursor_and_resize_preserves_scale() {
        let g = [100.0, -50.0, 200.0, 100.0];
        let anchor = [0.2, 0.75];
        let z = zoom_grid(g, anchor, 0.5);
        for a in 0..2 {
            assert!((g[a] + g[a + 2] * anchor[a] - z[a] - z[a + 2] * anchor[a]).abs() < 1e-12);
        }
        let v = Viewport::from_grid(z, Vec2::new(800.0, 400.0));
        let resized = v.grid(Vec2::new(1000.0, 600.0), true);
        assert_eq!(resized[2] / 1000.0, resized[3] / 600.0);
        assert_eq!(resized[0] + resized[2] * 0.5, z[0] + z[2] * 0.5);
    }
}
