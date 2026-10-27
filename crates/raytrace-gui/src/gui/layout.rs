use super::components::kind_label;
use super::*;
use raytrace_core::geometry::sag;

const LAYOUT_HELP: &str = "丸印をドラッグして配置 · ホイールで拡大 · 背景をドラッグして表示を移動";

impl App {
    pub(super) fn layout_view(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            egui::ComboBox::from_id_salt("projection")
                .width(116.0)
                .selected_text(["Z–X 側面", "Z–Y 上面", "X–Y 正面"][self.projection])
                .show_ui(ui, |ui| {
                    for (i, t) in ["Z–X 側面", "Z–Y 上面", "X–Y 正面"].iter().enumerate()
                    {
                        ui.selectable_value(&mut self.projection, i, *t);
                    }
                });
            checkbox(ui, &mut self.show_rays, "光線")
                .on_hover_text("光路は表示用のサンプルです（光源ごとの上限は計算設定で確認）");
            if ui.small_button("選択へ").clicked() {
                self.camera.request = Some(CameraRequest::Selected);
            }
            if ui.small_button("全体").on_hover_text(LAYOUT_HELP).clicked() {
                self.camera.request = Some(CameraRequest::All);
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.menu_button("表示", |ui| {
                    checkbox(ui, &mut self.equal_scale, "縦横を同じ縮尺にする");
                    if ui.button("拡大").clicked() {
                        self.camera.request = Some(CameraRequest::Zoom(0.5));
                        ui.close();
                    }
                    if ui.button("縮小").clicked() {
                        self.camera.request = Some(CameraRequest::Zoom(2.0));
                        ui.close();
                    }
                });
                if self.job.is_some() {
                    ui.spinner();
                }
            });
        });
        ui.add_space(10.0);
        let markers: Vec<_> = self
            .project
            .assembly
            .as_ref()
            .map(|a| {
                a.components
                    .iter()
                    .enumerate()
                    .map(|(i, c)| PartMarker {
                        index: i,
                        label: format!("{} {}", i + 1, kind_label(c.kind)),
                        position: c.pose.translation_mm,
                        enabled: c.enabled,
                    })
                    .collect()
            })
            .unwrap_or_default();
        // Geometry always comes from the editor; rays belong to a completed revision.
        let paths = visible_paths(self.result.as_ref(), self.stale, self.show_rays);
        let placement = draw_paths(
            ui,
            &self.project.scene,
            paths,
            self.projection,
            (ui.available_height() - 30.0).max(120.0),
            &markers,
            if self.project.assembly.is_some() {
                self.component
            } else {
                self.selected
            },
            self.job.is_none(),
            self.equal_scale,
            &mut self.camera,
        );
        if let Some((index, delta)) = placement {
            if self.component != index {
                self.source = 0;
            }
            self.component = index;
            if delta.norm() > 0.0 {
                if let Some(c) = self
                    .project
                    .assembly
                    .as_mut()
                    .and_then(|a| a.components.get_mut(index))
                {
                    c.pose.translation_mm = c.pose.translation_mm + delta;
                }
                self.refresh_components();
            }
        }
    }
}

struct PartMarker {
    index: usize,
    label: String,
    position: V3,
    enabled: bool,
}
#[allow(clippy::too_many_arguments)]
fn draw_paths(
    ui: &mut egui::Ui,
    scene: &Scene,
    paths: &[raytrace_core::trace::PathEvent],
    projection: usize,
    height: f32,
    markers: &[PartMarker],
    selected: usize,
    editable: bool,
    equal_scale: bool,
    camera: &mut LayoutCamera,
) -> Option<(usize, V3)> {
    let (rect, response) = ui.allocate_exact_size(
        Vec2::new(ui.available_width(), height),
        egui::Sense::click_and_drag(),
    );
    let response = response.on_hover_text(LAYOUT_HELP);
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 6.0, BG_CANVAS);
    let axes = [(2, 0), (2, 1), (0, 1)][projection];
    let mut lines: Vec<Vec<V3>> = Vec::new();
    let mut points = Vec::new();
    let mut focus_points = Vec::new();
    let selected_prefix = format!("{}:", selected + 1);
    for (surface_index, s) in scene.surfaces.iter().enumerate() {
        let point_start = points.len();
        let f = Frame::new(&s.pose);
        let (hx, hy) = s.aperture.bounds();
        if let Shape::ConicalSide {
            radius_start_mm,
            radius_end_mm,
            length_mm,
        } = &s.shape
        {
            for theta in [
                0.0,
                std::f64::consts::FRAC_PI_2,
                std::f64::consts::PI,
                3.0 * std::f64::consts::FRAC_PI_2,
            ] {
                let line = vec![
                    f.world_point(V3([
                        radius_start_mm * theta.cos(),
                        radius_start_mm * theta.sin(),
                        0.0,
                    ])),
                    f.world_point(V3([
                        radius_end_mm * theta.cos(),
                        radius_end_mm * theta.sin(),
                        *length_mm,
                    ])),
                ];
                points.extend_from_slice(&line);
                lines.push(line);
            }
        } else if let Shape::TriangleMesh {
            vertices,
            triangles,
            ..
        } = &s.shape
        {
            let step = (triangles.len() / 100).max(1);
            for t in triangles.iter().step_by(step) {
                let line = vec![
                    f.world_point(vertices[t[0] as usize]),
                    f.world_point(vertices[t[1] as usize]),
                    f.world_point(vertices[t[2] as usize]),
                    f.world_point(vertices[t[0] as usize]),
                ];
                points.extend_from_slice(&line);
                lines.push(line);
            }
        } else {
            let extent = if axes.1 == 1 { hy } else { hx };
            let line: Vec<_> = (0..65)
                .filter_map(|i| {
                    let u = -extent + 2.0 * extent * i as f64 / 64.0;
                    let (x, y) = if axes.0 == 0 {
                        match s.aperture {
                            Aperture::Circle { radius_mm } => {
                                let t = std::f64::consts::TAU * i as f64 / 64.0;
                                (radius_mm * t.cos(), radius_mm * t.sin())
                            }
                            Aperture::Rectangle {
                                half_width_mm: x,
                                half_height_mm: y,
                            } => {
                                let t = i as f64 / 16.0;
                                if t <= 1.0 {
                                    (-x + 2.0 * x * t, -y)
                                } else if t <= 2.0 {
                                    (x, -y + 2.0 * y * (t - 1.0))
                                } else if t <= 3.0 {
                                    (x - 2.0 * x * (t - 2.0), y)
                                } else {
                                    (-x, y - 2.0 * y * (t - 3.0))
                                }
                            }
                        }
                    } else if axes.1 == 1 {
                        (0.0, u)
                    } else {
                        (u, 0.0)
                    };
                    Some(f.world_point(V3([x, y, sag(&s.shape, x, y)?.0])))
                })
                .collect();
            points.extend_from_slice(&line);
            lines.push(line);
        }
        if (markers.is_empty() && surface_index == selected)
            || (!markers.is_empty() && s.name.starts_with(&selected_prefix))
        {
            focus_points.extend_from_slice(&points[point_start..]);
        }
    }
    for p in paths {
        points.push(p.start_mm);
        points.push(p.end_mm);
    }
    points.extend(
        markers
            .iter()
            .filter(|m| m.position.finite())
            .map(|m| m.position),
    );
    if points.is_empty() {
        painter.text(
            rect.center(),
            egui::Align2::CENTER_CENTER,
            "光学面を読み込んでください",
            egui::FontId::proportional(SIZE_CAPTION),
            TEXT_MUTED,
        );
        return None;
    }
    let mut low = [f64::INFINITY; 2];
    let mut high = [f64::NEG_INFINITY; 2];
    for p in &points {
        for (a, k) in [axes.0, axes.1].into_iter().enumerate() {
            low[a] = low[a].min(p.0[k]);
            high[a] = high[a].max(p.0[k]);
        }
    }
    let inner = rect.shrink2(Vec2::new(64.0, 44.0));
    let grid_id = ui.scope_id().with(("part_grid", projection));
    let active_id = grid_id.with("active");
    if focus_points.is_empty() {
        let center = markers
            .iter()
            .find(|m| m.index == selected)
            .map_or(points[0], |m| m.position);
        focus_points.extend([center - V3([1.0; 3]), center + V3([1.0; 3])]);
    }
    let mut focus_low = [f64::INFINITY; 2];
    let mut focus_high = [f64::NEG_INFINITY; 2];
    for p in &focus_points {
        for (a, k) in [axes.0, axes.1].into_iter().enumerate() {
            focus_low[a] = focus_low[a].min(p.0[k]);
            focus_high[a] = focus_high[a].max(p.0[k]);
        }
    }
    let auto = plot_bounds(low, high, inner.size(), equal_scale);
    let focus = plot_bounds(focus_low, focus_high, inner.size(), equal_scale);
    let requested_view = camera.request.is_some();
    let mut grid = camera.grid(projection, auto, focus, inner.size(), equal_scale);
    let to_screen = |g: [f64; 4], p: V3| {
        Pos2::new(
            inner.left() + ((p.0[axes.0] - g[0]) / g[2]) as f32 * inner.width(),
            inner.bottom() - ((p.0[axes.1] - g[1]) / g[3]) as f32 * inner.height(),
        )
    };
    if response.drag_started_by(egui::PointerButton::Primary)
        && editable
        && let Some(pointer) = ui.ctx().input(|i| i.pointer.press_origin())
        && let Some(m) = markers
            .iter()
            .filter(|m| m.position.finite() && to_screen(grid, m.position).distance(pointer) < 22.0)
            .min_by(|a, b| {
                to_screen(grid, a.position)
                    .distance(pointer)
                    .total_cmp(&to_screen(grid, b.position).distance(pointer))
            })
    {
        ui.ctx().data_mut(|d| {
            d.insert_temp(active_id, m.index);
            d.insert_temp(grid_id, grid);
        });
    }
    let active = ui.ctx().data_mut(|d| d.get_temp::<usize>(active_id));
    if response.dragged_by(egui::PointerButton::Primary) && active.is_some() {
        if let Some(old) = ui.ctx().data_mut(|d| d.get_temp::<[f64; 4]>(grid_id)) {
            grid = old;
        }
    } else if response.dragged() {
        let delta = ui.ctx().input(|i| i.pointer.delta());
        grid[0] -= delta.x as f64 / inner.width() as f64 * grid[2];
        grid[1] += delta.y as f64 / inner.height() as f64 * grid[3];
        camera.set(projection, grid, inner.size());
    }
    if !response.dragged_by(egui::PointerButton::Primary) {
        ui.ctx().data_mut(|d| {
            d.remove_temp::<[f64; 4]>(grid_id);
            d.remove_temp::<usize>(active_id);
        });
    }
    if !requested_view
        && response.hovered()
        && !ui.ctx().input(|i| i.pointer.any_down())
        && let Some(pointer) = response.hover_pos().filter(|p| inner.contains(*p))
    {
        let scroll = ui.ctx().input(|i| i.smooth_scroll_delta.y);
        if scroll != 0.0 {
            grid = camera::zoom_grid(
                grid,
                [
                    (pointer.x - inner.left()) as f64 / inner.width() as f64,
                    (inner.bottom() - pointer.y) as f64 / inner.height() as f64,
                ],
                (-scroll as f64 * 0.003).exp(),
            );
            camera.set(projection, grid, inner.size());
        }
    }
    let map = |p: V3| {
        Pos2::new(
            inner.left() + ((p.0[axes.0] - grid[0]) / grid[2]) as f32 * inner.width(),
            inner.bottom() - ((p.0[axes.1] - grid[1]) / grid[3]) as f32 * inner.height(),
        )
    };
    for i in 0..=4 {
        let y = inner.top() + inner.height() * i as f32 / 4.0;
        painter.line_segment(
            [Pos2::new(inner.left(), y), Pos2::new(inner.right(), y)],
            Stroke::new(0.5_f32, GRID),
        );
        painter.text(
            Pos2::new(inner.left() - 8.0, y),
            egui::Align2::RIGHT_CENTER,
            format!("{:.1}", grid[1] + grid[3] * (1.0 - i as f64 / 4.0)),
            egui::FontId::proportional(SIZE_CAPTION),
            TEXT_MUTED,
        );
        let x = inner.left() + inner.width() * i as f32 / 4.0;
        painter.line_segment(
            [Pos2::new(x, inner.top()), Pos2::new(x, inner.bottom())],
            Stroke::new(0.5_f32, GRID),
        );
    }
    let max = paths
        .iter()
        .map(|p| p.incident_weight)
        .fold(0.0, f64::max)
        .max(1e-300);
    let geometry_painter = painter.with_clip_rect(inner);
    for p in paths {
        let t = p.incident_weight / max;
        if t < 0.002 {
            continue;
        }
        geometry_painter.line_segment(
            [map(p.start_mm), map(p.end_mm)],
            Stroke::new(1.0_f32, viz::ray(t)),
        );
    }
    for line in lines {
        geometry_painter.add(egui::Shape::line(
            line.into_iter().map(map).collect(),
            Stroke::new(1.0_f32, viz::GEOMETRY),
        ));
    }
    for m in markers.iter().filter(|m| m.position.finite()) {
        let p = map(m.position);
        if !inner.contains(p) {
            continue;
        }
        let color = if m.enabled {
            viz::MARKER
        } else {
            viz::MARKER_MUTED
        };
        geometry_painter.circle_filled(p, if m.index == selected { 6.0 } else { 4.0 }, color);
        if m.index == selected {
            geometry_painter.circle_stroke(p, 9.0, Stroke::new(1.0_f32, viz::MARKER));
        }
        let label = painter.layout_no_wrap(
            m.label.clone(),
            egui::FontId::proportional(SIZE_CAPTION),
            color,
        );
        let size = label.size();
        let left = if p.x + 8.0 + size.x < inner.right() - 6.0 {
            p.x + 8.0
        } else {
            p.x - size.x - 8.0
        };
        let top = if m.index % 2 == 0 {
            p.y + 12.0
        } else {
            p.y - 12.0 - size.y
        }
        .clamp(rect.top() + 22.0, rect.bottom() - 22.0 - size.y);
        let pos = Pos2::new(left, top);
        geometry_painter.rect_filled(
            egui::Rect::from_min_size(pos, size).expand(2.0),
            2.0,
            BG_CANVAS,
        );
        geometry_painter.galley(pos, label, color);
    }
    painter.text(
        Pos2::new(rect.center().x, rect.bottom() - 4.0),
        egui::Align2::CENTER_BOTTOM,
        format!(
            "{}  [{:.2} … {:.2}] mm",
            ["X", "Y", "Z"][axes.0],
            grid[0],
            grid[0] + grid[2]
        ),
        egui::FontId::proportional(SIZE_CAPTION),
        TEXT_MUTED,
    );
    painter.text(
        rect.left_top() + Vec2::new(8.0, 7.0),
        egui::Align2::LEFT_TOP,
        format!("{} [mm]", ["X", "Y", "Z"][axes.1]),
        egui::FontId::proportional(SIZE_CAPTION),
        TEXT_MUTED,
    );
    let mut action = None;
    if editable && response.clicked_by(egui::PointerButton::Primary) {
        let pointer = ui
            .ctx()
            .input(|i| i.pointer.press_origin())
            .or_else(|| response.interact_pointer_pos());
        if let Some(p) = pointer
            && let Some(m) = markers
                .iter()
                .filter(|m| m.position.finite() && map(m.position).distance(p) < 22.0)
                .min_by(|a, b| {
                    map(a.position)
                        .distance(p)
                        .total_cmp(&map(b.position).distance(p))
                })
        {
            action = Some((m.index, V3::default()));
        }
    }
    if editable
        && response.dragged_by(egui::PointerButton::Primary)
        && let Some(index) = ui.ctx().data_mut(|d| d.get_temp::<usize>(active_id))
    {
        let pixel = ui.ctx().input(|i| i.pointer.delta());
        let mut delta = V3::default();
        delta.0[axes.0] = pixel.x as f64 / inner.width() as f64 * grid[2];
        delta.0[axes.1] = -pixel.y as f64 / inner.height() as f64 * grid[3];
        action = Some((index, delta));
    }
    action
}

fn plot_bounds(low: [f64; 2], high: [f64; 2], size: Vec2, equal_scale: bool) -> [f64; 4] {
    let mut span = [(high[0] - low[0]).max(1e-3), (high[1] - low[1]).max(1e-3)];
    if equal_scale {
        let units_per_pixel =
            (span[0] / size.x.max(1.0) as f64).max(span[1] / size.y.max(1.0) as f64);
        span = [
            units_per_pixel * size.x as f64,
            units_per_pixel * size.y as f64,
        ];
    }
    span = [span[0] * 1.12, span[1] * 1.12];
    [
        (low[0] + high[0] - span[0]) * 0.5,
        (low[1] + high[1] - span[1]) * 0.5,
        span[0],
        span[1],
    ]
}
fn visible_paths(
    result: Option<&ResultView>,
    stale: bool,
    show_rays: bool,
) -> &[raytrace_core::trace::PathEvent] {
    if stale || !show_rays {
        &[]
    } else {
        result
            .map(|r| r.report.paths.as_slice())
            .unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn equal_scale_preserves_lengths_in_both_axes_and_contains_the_scene() {
        let size = Vec2::new(800.0, 500.0);
        for (low, high) in [([0.0, -2.0], [100.0, 3.0]), ([-1.0, -50.0], [2.0, 90.0])] {
            let grid = plot_bounds(low, high, size, true);
            assert!((grid[2] / size.x as f64 - grid[3] / size.y as f64).abs() < 1e-12);
            for a in 0..2 {
                assert!(grid[a] <= low[a]);
                assert!(grid[a] + grid[a + 2] >= high[a]);
            }
        }
    }

    #[test]
    fn drag_handle_changes_the_component_in_the_selected_projection() {
        let ctx = egui::Context::default();
        install(&ctx);
        let scene = assembly::empty_scene("drag check");
        let markers = [
            PartMarker {
                index: 0,
                label: "A".into(),
                position: V3([0.0, 0.0, 0.0]),
                enabled: true,
            },
            PartMarker {
                index: 1,
                label: "B".into(),
                position: V3([0.0, 0.0, 10.0]),
                enabled: true,
            },
        ];
        let mut camera = LayoutCamera::default();
        let mut frame = |events| {
            let mut action = None;
            let input = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    Pos2::ZERO,
                    Vec2::new(640.0, 480.0),
                )),
                events,
                ..Default::default()
            };
            let output = run_ui(&ctx, input, |ui| {
                egui::CentralPanel::default().show(ui, |ui| {
                    action = draw_paths(
                        ui,
                        &scene,
                        &[],
                        0,
                        300.0,
                        &markers,
                        0,
                        true,
                        true,
                        &mut camera,
                    );
                });
            });
            (output, action)
        };
        let (output, _) = frame(vec![]);
        let p = output
            .shapes
            .iter()
            .find_map(|s| match &s.shape {
                egui::Shape::Circle(c) if c.radius == 6.0 => Some(c.center),
                _ => None,
            })
            .unwrap();
        frame(vec![
            egui::Event::PointerMoved(p),
            egui::Event::PointerButton {
                pos: p,
                button: egui::PointerButton::Primary,
                pressed: true,
                modifiers: egui::Modifiers::NONE,
            },
        ]);
        let (_, action) = frame(vec![egui::Event::PointerMoved(p + Vec2::new(40.0, 0.0))]);
        let (part, delta) = action.expect("dragging a marker must produce a placement edit");
        assert_eq!(part, 0);
        assert!(delta.0[2] > 0.0);
        assert_eq!(delta.0[0], 0.0);
        assert_eq!(delta.0[1], 0.0);
    }
    use crate::gui::test_support::*;
    #[test]
    fn zoom_and_background_pan_change_only_the_view_and_all_restores_auto_fit() {
        let mut app = App::demo();
        let ctx = egui::Context::default();
        install(&ctx);
        for _ in 0..3 {
            frame(&mut app, &ctx, vec![]);
        }
        let original = serde_json::to_value(&app.project).unwrap();
        let p = Pos2::new(850.0, 250.0);
        frame(
            &mut app,
            &ctx,
            vec![
                egui::Event::PointerMoved(p),
                egui::Event::MouseWheel {
                    phase: egui::TouchPhase::Move,
                    source: egui::MouseWheelSource::Unknown,
                    unit: egui::MouseWheelUnit::Point,
                    delta: Vec2::new(0.0, 100.0),
                    modifiers: egui::Modifiers::NONE,
                },
            ],
        );
        let zoomed = serde_json::to_value(&app.camera).unwrap();
        assert!(zoomed["views"][0].is_object());
        frame(
            &mut app,
            &ctx,
            vec![egui::Event::PointerButton {
                pos: p,
                button: egui::PointerButton::Primary,
                pressed: true,
                modifiers: egui::Modifiers::NONE,
            }],
        );
        frame(
            &mut app,
            &ctx,
            vec![egui::Event::PointerMoved(p + Vec2::new(50.0, 20.0))],
        );
        frame(
            &mut app,
            &ctx,
            vec![egui::Event::PointerButton {
                pos: p + Vec2::new(50.0, 20.0),
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: egui::Modifiers::NONE,
            }],
        );
        let panned = serde_json::to_value(&app.camera).unwrap();
        assert_ne!(zoomed["views"][0]["center"], panned["views"][0]["center"]);
        assert_eq!(original, serde_json::to_value(&app.project).unwrap());
        assert!(!app.history.can_undo());
        app.camera.request = Some(CameraRequest::All);
        frame(&mut app, &ctx, vec![]);
        assert!(serde_json::to_value(&app.camera).unwrap()["views"][0].is_null());
    }
}
