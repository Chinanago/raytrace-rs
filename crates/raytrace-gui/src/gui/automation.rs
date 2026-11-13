//! The UI owns the session. External calls prepare a clone off-thread and commit here.
use super::*;
use raytrace_engine::remote::{Prepared, Server};
use std::path::Path;

impl App {
    pub(super) fn start_automation(&mut self, path: &Path, ctx: &egui::Context) {
        let wake = ctx.clone();
        match Server::start(path, move || wake.request_repaint()) {
            Ok(server) => self.automation = Some(server),
            Err(e) => self.error = Some(format!("GUIの外部接続を開始できません: {e}")),
        }
    }

    pub(super) fn poll_automation(&mut self, ctx: &egui::Context) {
        if let Some(receiver) = &self.automation_work {
            match receiver.try_recv() {
                Ok(prepared) => {
                    self.automation_work = None;
                    self.control = None;
                    self.accept_remote(prepared);
                }
                Err(mpsc::TryRecvError::Empty) => return,
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.automation_work = None;
                    self.control = None;
                    self.error = Some("外部コマンドの作業スレッドが終了しました".into());
                }
            }
        }
        if self.job.is_some() {
            return;
        }
        let Some(pending) = self.automation.as_ref().and_then(Server::try_recv) else {
            return;
        };
        self.end_gesture();
        self.remember_selection();
        let session = self.session.clone();
        self.control = Some(pending.control());
        let (sender, receiver) = mpsc::channel();
        self.automation_work = Some(receiver);
        self.status = "外部コマンドを実行中…".into();
        let wake = ctx.clone();
        std::thread::spawn(move || {
            let prepared = pending.prepare(session);
            let _ = sender.send(prepared);
            wake.request_repaint();
        });
    }

    pub(super) fn accept_remote(&mut self, prepared: Prepared) {
        let revision = self.session.revision();
        let previous = self.session.latest_run().map(|r| r.record.clone());
        if !prepared.commit(&mut self.session) {
            self.status = "外部コマンドは変更を確定しませんでした".into();
            return;
        }
        self.path = self.session.path().map(Path::to_path_buf);
        self.catalogs = self.session.catalogs().to_vec();
        self.sync_compiled();
        if self.session.revision() != revision {
            self.viewing_explicit_record = false;
            self.component = 0;
            self.selected = 0;
            self.source = 0;
            self.detector = 0;
        }
        if let Some(latest) = self.session.latest_run()
            && previous
                .as_ref()
                .is_none_or(|r| !Arc::ptr_eq(r, &latest.record))
        {
            let design = if latest.revision == Some(self.session.revision()) {
                self.session.design().cloned()
            } else {
                Some(Arc::new(latest.record.calculation.design.clone()))
            };
            if let Some(design) = design {
                self.viewing_explicit_record = latest.revision.is_none();
                self.result = Some(ResultView {
                    design,
                    preview_limit: None,
                    scene: latest.record.calculation.scene.clone(),
                    record: latest.record.clone(),
                    viewer_paths: latest.viewer_paths.clone(),
                    texture: None,
                    screen: None,
                    roi_cache: None,
                });
            }
        }
        self.status = "外部コマンドを実行しました".into();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use raytrace_engine::{Context, remote::Client};
    use serde_json::{Value, json};
    use std::{fs, thread, time::Instant};

    fn call(app: &mut App, client: &Client, request: Value, ctx: &egui::Context) -> Value {
        thread::scope(|scope| {
            let response = scope.spawn(|| {
                serde_json::to_value(
                    client.execute(request, &mut Context::new(&TraceControl::default())),
                )
                .unwrap()
            });
            let until = Instant::now() + Duration::from_secs(20);
            while !response.is_finished() {
                app.poll(ctx);
                assert!(Instant::now() < until, "GUI command did not finish");
                thread::sleep(Duration::from_millis(1));
            }
            response.join().unwrap()
        })
    }

    #[test]
    fn external_edits_trace_save_and_gui_undo_share_the_session() {
        let dir =
            std::env::temp_dir().join(format!("raytrace-gui-connection-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let file = dir.join("session.json");
        let ctx = egui::Context::default();
        let mut app = App::demo();
        app.start_automation(&file, &ctx);
        assert!(app.error.is_none(), "{:?}", app.error);
        let client = Client::open(&file).unwrap();
        let revision = app.session.revision();
        let occurrence = app.design().occurrences[1].id.as_str().to_string();
        let path = format!("/occurrences/@{occurrence}/pose/translation_mm/0");
        let before = app.design().occurrences[1].pose.translation_mm.0[0];
        let edited = call(
            &mut app,
            &client,
            json!({"command":"doc.set","path":path,"value":before + 2.0,"expected_revision":revision}),
            &ctx,
        );
        assert_eq!(edited["ok"], true, "{edited}");
        assert_eq!(
            app.design().occurrences[1].pose.translation_mm.0[0],
            before + 2.0
        );
        assert!(app.is_dirty());
        app.undo();
        assert_eq!(app.design().occurrences[1].pose.translation_mm.0[0], before);
        app.redo();
        assert_eq!(
            app.design().occurrences[1].pose.translation_mm.0[0],
            before + 2.0
        );
        let conflict = call(
            &mut app,
            &client,
            json!({"command":"session.undo","expected_revision":revision}),
            &ctx,
        );
        assert_eq!(conflict["error"]["code"], "revision");
        let traced = call(
            &mut app,
            &client,
            json!({"command":"analysis.trace","overrides":{"rays_per_source":64},"threads":2,"preview_paths":8}),
            &ctx,
        );
        assert_eq!(traced["ok"], true, "{traced}");
        let result = app.result.as_ref().unwrap();
        assert!(!result.viewer_paths.is_empty());
        assert!(result.record.report.paths.is_empty());
        assert!(Arc::ptr_eq(
            &result.record,
            &app.session.latest_run().unwrap().record
        ));
        assert!(!app.is_stale());
        let saved = dir.join("design.json");
        assert_eq!(
            call(
                &mut app,
                &client,
                json!({"command":"doc.save","output":saved}),
                &ctx
            )["ok"],
            true
        );
        assert_eq!(app.path.as_deref(), Some(saved.as_path()));
        assert!(!app.is_dirty());
        // A trace started by the GUI also becomes the shared session's latest result.
        app.preview_limit = Some(64);
        app.run(&ctx);
        let until = Instant::now() + Duration::from_secs(20);
        while app.job.is_some() {
            app.poll(&ctx);
            assert!(Instant::now() < until);
            thread::sleep(Duration::from_millis(1));
        }
        assert!(Arc::ptr_eq(
            &app.result.as_ref().unwrap().record,
            &app.session.latest_run().unwrap().record
        ));
        let budget = call(
            &mut app,
            &client,
            json!({"command":"analysis.budget"}),
            &ctx,
        );
        assert_eq!(budget["ok"], true, "{budget}");
        assert_eq!(budget["result"]["stale"], false);
        let output = test_support::frame(&mut app, &ctx, Vec::new());
        assert!(!output.shapes.is_empty());
        drop(app);
        assert!(!file.exists());
        fs::remove_dir_all(dir).unwrap();
    }
}
