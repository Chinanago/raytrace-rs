use super::*;
use raytrace_engine::{
    commands,
    engine::{
        ComponentAddRequest, DocOpenRequest, DocSaveRequest, EngineError, ExportZmxRequest,
        RunOpenRequest, SaveRunRequest,
    },
};

impl App {
    pub(super) fn toolbar(&mut self, ui: &mut egui::Ui) {
        let ctx = &ui.ctx().clone();
        self.update_window_title(ctx);
        egui::Panel::top("toolbar")
            .exact_size(TOOLBAR_HEIGHT)
            .show_separator_line(false)
            .frame(toolbar_frame())
            .show(ui, |ui| {
                let row = ui.max_rect().expand2(Vec2::new(SPACE_4, 0.0));
                let mut selected = egui::Rect::NOTHING;
                ui.with_layout(egui::Layout::left_to_right(egui::Align::Max), |ui| {
                    ui.set_height(TOOLBAR_HEIGHT);
                    ui.spacing_mut().item_spacing.x = -TAB_LINE_WIDTH;
                    let layout = tab(ui, &mut self.view, View::Layout, "光学系", BG_PANEL);
                    let screen = tab(ui, &mut self.view, View::Screen, "スクリーン", BG_CANVAS);
                    selected = if self.view == View::Layout {
                        layout.rect
                    } else {
                        screen.rect
                    };
                    ui.spacing_mut().item_spacing.x = SPACE_2;
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if primary_scope(ui, |ui| {
                            ui.add_enabled(
                                self.job.is_none() || self.control.is_some(),
                                egui::Button::new(
                                    egui::RichText::new(if self.control.is_some() {
                                        "■  中止"
                                    } else if self.viewing_explicit_record {
                                        "モデル光源で計算"
                                    } else {
                                        "▶  計算する"
                                    })
                                    .color(TEXT_ON_PRIMARY)
                                    .font(bold_font(SIZE_BODY)),
                                )
                                .min_size(Vec2::new(PRIMARY_BUTTON_WIDTH, CONTROL_HEIGHT)),
                            )
                        })
                        .clicked()
                        {
                            if let Some(control) = &self.control {
                                control.cancel.store(true, Ordering::Relaxed);
                            } else {
                                self.viewing_explicit_record = false;
                                self.run(ctx);
                            }
                        }
                        if ui.button("計算設定").clicked() {
                            self.details = Some(Details::Calculation);
                        }
                        ui.add_enabled_ui(self.job.is_none(), |ui| {
                            if history_button(ui, self.can_redo(), "↷")
                                .on_hover_text("やり直す (Ctrl+Y)")
                                .clicked()
                            {
                                self.redo();
                            }
                            if history_button(ui, self.can_undo(), "↶")
                                .on_hover_text("元に戻す (Ctrl+Z)")
                                .clicked()
                            {
                                self.undo();
                            }
                            ui.menu_button("ファイル", |ui| self.file_menu(ui, ctx));
                        });
                        if self.study.manifest.is_some() && ui.small_button("走査結果").clicked()
                        {
                            self.study.open = true;
                        }
                    });
                });
                tab_baseline(ui, row, selected);
            });
    }

    pub(super) fn update_window_title(&mut self, ctx: &egui::Context) {
        let name = if self.design().display_name().trim().is_empty() {
            "新規の光学系"
        } else {
            self.design().display_name()
        };
        let title = format!(
            "{}{name} — Raytrace",
            if self.is_dirty() { "*" } else { "" }
        );
        if self.last_window_title.as_ref() != Some(&title) {
            ctx.send_viewport_cmd(egui::ViewportCommand::Title(title.clone()));
            self.last_window_title = Some(title);
        }
    }

    fn file_menu(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        if ui.button("パラメーター走査…").clicked() {
            self.configure_study();
            ui.close();
        }
        if ui.button("新規の光学系").clicked() {
            self.replace_design(None, Some(256), 0);
            self.status = "部品を追加して光学系を組み立てます".into();
            ui.close();
        }
        if ui.button("開く…").clicked() {
            ui.close();
            if let Some(path) = optics_dialog().pick_file() {
                self.load(path, ctx);
            }
        }
        ui.menu_button("最近開いたファイル", |ui| {
            for path in self.preferences.recent_files.clone() {
                let label = path.file_name().unwrap_or_default().to_string_lossy();
                if ui
                    .button(label)
                    .on_hover_text(path.display().to_string())
                    .clicked()
                {
                    self.load(path, ctx);
                    ui.close();
                }
            }
        });
        if ui.button("保存…").clicked() {
            ui.close();
            if let Some(path) = rfd::FileDialog::new()
                .set_file_name("optics.raytrace.json")
                .add_filter("Raytrace", &["json"])
                .save_file()
            {
                self.end_gesture();
                let request = DocSaveRequest {
                    design: None,
                    output: Some(path.clone()),
                };
                match self.session.run(commands::DOC_SAVE, request) {
                    Ok(_) => {
                        self.path = Some(path);
                        self.remember_path();
                        self.status = "プロジェクトを保存しました".into();
                    }
                    Err(e) => self.error = Some(e.to_string()),
                }
            }
        }
        if ui
            .add_enabled(self.result.is_some(), egui::Button::new("計算結果を保存…"))
            .on_hover_text("計算時の部品・材料・光源条件と画素データを保存します")
            .clicked()
        {
            ui.close();
            if let Some(path) = rfd::FileDialog::new()
                .set_file_name("calculation.raytrace-run.json")
                .add_filter("Raytrace calculation", &["json"])
                .save_file()
            {
                let record = self.result.as_ref().unwrap().record(false);
                let request = SaveRunRequest {
                    record: Box::new(record),
                    output: path,
                };
                match Session::new().run(commands::RUN_SAVE, request) {
                    Ok(_) => self.status = "計算時の条件と結果を保存しました".into(),
                    Err(e) => self.error = Some(e.to_string()),
                }
            }
        }
        if ui
            .add_enabled(
                self.result
                    .as_ref()
                    .is_some_and(|r| !r.report.detectors.is_empty()),
                egui::Button::new("スクリーンを書き出す…"),
            )
            .clicked()
        {
            ui.close();
            if let Some(parent) = rfd::FileDialog::new()
                .set_title("スクリーンの出力先")
                .pick_folder()
            {
                let stamp = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_millis();
                self.export_current_screen(&parent.join(format!("screen_{stamp}")), ctx);
            }
        }
        if ui
            .add_enabled(
                !self.design().is_assembly(),
                egui::Button::new("ZMX書出し…"),
            )
            .on_hover_text("組立光学系はJSONプロジェクトで保存します")
            .clicked()
        {
            ui.close();
            if let Some(path) = rfd::FileDialog::new()
                .set_file_name("optics_edited.zmx")
                .add_filter("Zemax", &["zmx"])
                .save_file()
            {
                let request = ExportZmxRequest {
                    model: None,
                    catalogs: Vec::new(),
                    output: path,
                };
                match self.session.run(commands::DOC_EXPORT_ZMX, request) {
                    Ok(_) => self.status = "ZMXを書き出しました".into(),
                    Err(e) => self.error = Some(e.to_string()),
                }
            }
        }
        ui.separator();
        ui.menu_button("サンプルを開く", |ui| {
            if ui.button("光源＋レンズ＋ロッド").clicked() {
                self.replace_design(Some(design::demo_design().expect("assembly demo")), None, 2);
                self.run(ctx);
                ui.close();
            }
            for (name, text) in [
                (
                    "レンズ",
                    include_str!("../../../../examples/sequential_lens.json"),
                ),
                (
                    "ロッド",
                    include_str!("../../../../examples/integrator_rod.json"),
                ),
                (
                    "ミラー",
                    include_str!("../../../../examples/folded_mirror.json"),
                ),
            ] {
                if ui.button(name).clicked() {
                    self.replace_design(
                        Some(Design::from_scene(serde_json::from_str(text).unwrap())),
                        None,
                        0,
                    );
                    self.run(ctx);
                    ui.close();
                }
            }
        });
    }

    /// Start a new document (empty without `design`) and reset what the GUI shows of the
    /// previous one.
    fn replace_design(&mut self, design: Option<Design>, preview: Option<usize>, component: usize) {
        self.viewing_explicit_record = false;
        self.path = None;
        self.loading_path = None;
        self.result = None;
        self.component = component;
        self.source = 0;
        self.selected = 0;
        self.source_selected = false;
        self.detector = 0;
        self.preview_limit = preview;
        self.view = View::Layout;
        self.error = None;
        self.start_document(design);
        self.camera = LayoutCamera::default();
    }

    pub(super) fn add_menu(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        for (label, kind) in [
            ("光源のZMX…", PartKind::Source),
            ("レンズのZMX…", PartKind::Optic),
        ] {
            if ui.button(label).clicked() {
                ui.close();
                if let Some(path) = optics_dialog().pick_file() {
                    self.add_file(path, kind, ctx);
                }
            }
        }
        ui.separator();
        if ui.button("ロッド").clicked() {
            self.add_generated(PartKind::Rod);
            ui.close();
        }
        if ui.button("スクリーン").clicked() {
            self.add_generated(PartKind::Detector);
            ui.close();
        }
    }

    /// Import a file as a part. A copy of the session reads the file on a worker and runs
    /// `component.add`; the session takes the copy over when it is done.
    pub(super) fn add_file(&mut self, path: PathBuf, kind: PartKind, ctx: &egui::Context) {
        self.status = "部品を読込中…".into();
        self.end_gesture();
        let group = self.group();
        self.end_gesture();
        let request = ComponentAddRequest {
            path: Some(path.clone()),
            kind: Some(kind),
            catalogs: self.catalogs.clone(),
            group: Some(group.clone()),
            ..ComponentAddRequest::default()
        };
        let session = self.session.clone();
        self.submit_on(session, ctx, false, None, move |session, context| {
            let not_a_part = || "部品にはZMXまたはプロジェクトJSONを指定してください".to_string();
            let added = if path.is_dir() {
                Err(not_a_part())
            } else {
                session
                    .call(commands::COMPONENT_ADD, request, context)
                    .map(|_| Box::new(std::mem::take(session)))
                    .map_err(|e| {
                        if e.message.starts_with("計算記録は") {
                            not_a_part()
                        } else {
                            e.message
                        }
                    })
            };
            Job::Added(kind, group, added)
        });
    }
    pub(super) fn load(&mut self, path: PathBuf, ctx: &egui::Context) {
        self.status = "ファイルを読込中…".into();
        self.loading_path = Some(path.clone());
        let request = DocOpenRequest {
            path: path.clone(),
            catalogs: self.catalogs.clone(),
            include_design: false,
            expected_revision: None,
        };
        self.submit(ctx, false, None, move |session, context| {
            match session.call(commands::DOC_OPEN, request, context) {
                // The session that opened the file becomes the GUI's session.
                Ok(_) => Job::Loaded(Ok(Box::new(std::mem::take(session)))),
                // A calculation record opens as a result view instead of a design.
                Err(EngineError {
                    code: "run_record", ..
                }) => {
                    let request = RunOpenRequest {
                        path,
                        viewer_paths: true,
                    };
                    match session.call(commands::RUN_OPEN, request, context) {
                        Ok(opened) => Job::LoadedRun(Ok(opened.record)),
                        Err(e) => Job::LoadedRun(Err(e.message)),
                    }
                }
                Err(EngineError {
                    code: "study_directory",
                    ..
                }) => Job::Loaded(Err("走査記録は「走査記録を開く…」から開いてください".into())),
                Err(e) => Job::Loaded(Err(e.message)),
            }
        });
    }
}

fn optics_dialog() -> rfd::FileDialog {
    rfd::FileDialog::new().add_filter("Optics", &["zmx", "ZMX", "json"])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gui::test_support::*;

    fn titles(output: &egui::FullOutput) -> Vec<&str> {
        output.viewport_output[&egui::ViewportId::ROOT]
            .commands
            .iter()
            .filter_map(|cmd| match cmd {
                egui::ViewportCommand::Title(title) => Some(title.as_str()),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn toolbar_has_one_row_without_app_or_project_labels() {
        let ctx = egui::Context::default();
        install(&ctx);
        let mut app = App::demo();
        frame(&mut app, &ctx, vec![]);
        let out = frame(&mut app, &ctx, vec![]);
        let text = visible_text(&out);
        assert!(!text.contains(&"Raytrace"));
        assert!(!text.contains(&"●"));
        assert!(!text.contains(&app.design().display_name()));
        let calculate = text_rect(&out, "▶  計算する").unwrap();
        let mut previous = None;
        for label in [
            "光学系",
            "スクリーン",
            "ファイル",
            "↶",
            "↷",
            "計算設定",
            "▶  計算する",
        ] {
            let rect = text_rect(&out, label).unwrap();
            assert!(rect.y_range().intersects(calculate.y_range()), "{label}");
            assert!(rect.bottom() <= TOOLBAR_HEIGHT);
            if let Some(x) = previous {
                assert!(rect.left() > x, "{label}");
            }
            previous = Some(rect.right());
        }
        let height = egui::PanelState::load(&ctx, ctx.viewport_id().root_ui_id().with("toolbar"))
            .unwrap()
            .outer_rect
            .height();
        assert_eq!(height, TOOLBAR_HEIGHT);
    }

    #[test]
    fn toolbar_tabs_touch_content_and_actions_stay_centered() {
        let ctx = egui::Context::default();
        install(&ctx);
        let mut app = App::demo();
        for view in [View::Layout, View::Screen] {
            app.view = view;
            frame(&mut app, &ctx, vec![]);
            let out = frame(&mut app, &ctx, vec![]);
            let fill = if view == View::Layout {
                BG_PANEL
            } else {
                BG_CANVAS
            };
            let tabs: Vec<_> = out
                .shapes
                .iter()
                .filter_map(|s| match &s.shape {
                    egui::Shape::Rect(r)
                        if r.rect.height() == TAB_HEIGHT
                            && r.rect.bottom() == TOOLBAR_HEIGHT
                            && r.corner_radius == egui::CornerRadius::ZERO =>
                    {
                        Some(r)
                    }
                    _ => None,
                })
                .collect();
            assert_eq!(tabs.len(), 2);
            let active = usize::from(view == View::Screen);
            assert_eq!(tabs[active].fill, fill);
            assert_eq!(tabs[0].rect.top(), TOOLBAR_HEIGHT - TAB_HEIGHT);
            let action =
                egui::PanelState::load(&ctx, ctx.viewport_id().root_ui_id().with("toolbar"))
                    .unwrap();
            assert_eq!(action.outer_rect.height(), TOOLBAR_HEIGHT);
            let button = out
                .shapes
                .iter()
                .find_map(|s| match &s.shape {
                    egui::Shape::Rect(r)
                        if r.fill == PRIMARY && r.rect.width() >= PRIMARY_BUTTON_WIDTH =>
                    {
                        Some(r.rect)
                    }
                    _ => None,
                })
                .unwrap();
            assert_eq!(button.center().y, TOOLBAR_HEIGHT / 2.0);
        }
    }

    #[test]
    fn window_title_tracks_existing_dirty_history_and_only_sends_changes() {
        let ctx = egui::Context::default();
        install(&ctx);
        let mut app = App::demo();
        let name = app.design().display_name().to_string();
        let out = frame(&mut app, &ctx, vec![]);
        assert_eq!(titles(&out), [format!("{name} — Raytrace")]);
        assert!(titles(&frame(&mut app, &ctx, vec![])).is_empty());
        let mut edits = edit::Edits::default();
        edits.set(
            format!(
                "/occurrences/@{}/pose/translation_mm/2",
                app.design().occurrences[2].id.as_str()
            ),
            &70.0,
        );
        assert!(app.apply(edits));
        let out = frame(&mut app, &ctx, vec![]);
        assert_eq!(titles(&out), [format!("*{name} — Raytrace")]);
        app.undo();
        let out = frame(&mut app, &ctx, vec![]);
        assert_eq!(titles(&out), [format!("{name} — Raytrace")]);
        app.redo();
        let out = frame(&mut app, &ctx, vec![]);
        assert_eq!(titles(&out), [format!("*{name} — Raytrace")]);
        let saved = PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../.."))
            .join("target/gui_title_tests")
            .join(std::process::id().to_string())
            .join("saved.raytrace.json");
        app.session
            .run(
                commands::DOC_SAVE,
                DocSaveRequest {
                    design: None,
                    output: Some(saved),
                },
            )
            .unwrap();
        let out = frame(&mut app, &ctx, vec![]);
        assert_eq!(titles(&out), [format!("{name} — Raytrace")]);
        set_document(&mut app, |d| d.set_display_name("別の光学系"));
        let out = frame(&mut app, &ctx, vec![]);
        assert_eq!(titles(&out), ["別の光学系 — Raytrace"]);
        set_document(&mut app, |d| d.set_display_name(""));
        let out = frame(&mut app, &ctx, vec![]);
        assert_eq!(titles(&out), ["新規の光学系 — Raytrace"]);
        assert!(titles(&frame(&mut app, &ctx, vec![])).is_empty());
    }
}
