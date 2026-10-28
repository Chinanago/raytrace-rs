use super::*;
use raytrace_engine::{Command, Error, Outcome, Query, Request};
pub(super) type PreparedCall = (
    raytrace_engine::bridge::Call,
    raytrace_engine::Result<raytrace_engine::PreparedEdit>,
);

pub(super) struct PendingWait {
    call: raytrace_engine::bridge::Call,
    deadline: Instant,
}
impl App {
    pub(super) fn poll_automation(&mut self, ctx: &egui::Context) {
        if let Some(rx) = &self.automation_prepare {
            match rx.try_recv() {
                Ok((call, result)) => {
                    self.automation_prepare = None;
                    let reset = matches!(call.request.command, Command::Open { .. });
                    let result = if Instant::now() > call.expires_at {
                        Err(Error::new("expired", "読込操作の待機時間を超えました"))
                    } else if self.job.is_some() {
                        Err(Error::new("busy", "GUIの読込が進行中です"))
                    } else {
                        self.commit_edit();
                        result
                            .and_then(|p| self.engine.apply_prepared(p))
                            .and_then(|o| self.adopt_automation_outcome(o, reset, false, false))
                    };
                    let _ = call.reply.send(result);
                }
                Err(mpsc::TryRecvError::Disconnected) => self.automation_prepare = None,
                Err(mpsc::TryRecvError::Empty) => (),
            }
        }
        // Apply requests only on the UI thread, between complete GUI edits.
        for _ in 0..16 {
            let call = self.automation.as_ref().and_then(|b| b.next_call());
            let Some(call) = call else {
                break;
            };
            if Instant::now() > call.expires_at {
                let _ = call
                    .reply
                    .send(Err(Error::new("expired", "操作の待機時間を超えました")));
                continue;
            }
            if call.request.command.needs_preparation() {
                let preparation =
                    (|| -> raytrace_engine::Result<raytrace_engine::EditPreparation> {
                        if self.automation_prepare.is_some() || self.job.is_some() {
                            return Err(Error::new("busy", "GUIの読込が進行中です"));
                        }
                        if call.request.expected_revision.is_none() {
                            return Err(Error::new(
                                "revision_required",
                                "GUIの編集にはexpected_revisionを指定してください",
                            ));
                        }
                        self.commit_edit();
                        self.engine.set_catalogs(self.catalogs.clone());
                        self.engine.prepare_edit(call.request.clone())
                    })();
                match preparation {
                    Err(e) => {
                        let _ = call.reply.send(Err(e));
                    }
                    Ok(preparation) => {
                        let (tx, rx) = mpsc::channel();
                        self.automation_prepare = Some(rx);
                        let context = ctx.clone();
                        std::thread::spawn(move || {
                            let result = preparation.run();
                            let _ = tx.send((call, result));
                            context.request_repaint();
                        });
                    }
                }
                continue;
            }
            if let Command::Wait { timeout_ms, .. } = call.request.command {
                if timeout_ms > 60_000 {
                    let _ = call.reply.send(Err(Error::new(
                        "invalid_value",
                        "待機は1回60,000 msまでです",
                    )));
                } else {
                    self.automation_waits.push(PendingWait {
                        deadline: Instant::now() + Duration::from_millis(timeout_ms),
                        call,
                    });
                }
                continue;
            }
            let result = self.apply_automation(call.request);
            let _ = call.reply.send(result);
        }
        let waits = std::mem::take(&mut self.automation_waits);
        for wait in waits {
            let Command::Wait { job, .. } = &wait.call.request.command else {
                unreachable!()
            };
            let result = self.engine.apply(Request {
                expected_revision: wait.call.request.expected_revision,
                command: Command::Query {
                    query: Query::Job { job: job.clone() },
                },
            });
            let pending = result.as_ref().is_ok_and(|o| o.data["status"] == "running")
                && Instant::now() < wait.deadline;
            if pending {
                self.automation_waits.push(wait);
            } else {
                let _ = wait.call.reply.send(result);
            }
        }
        self.poll_automation_result();
        if !self.automation_waits.is_empty()
            || self.automation_job.is_some()
            || self.automation_prepare.is_some()
        {
            ctx.request_repaint_after(Duration::from_millis(50));
        }
    }
    pub(super) fn apply_automation(
        &mut self,
        request: Request,
    ) -> raytrace_engine::Result<Outcome> {
        let mutation = request.command.is_edit()
            || matches!(
                request.command,
                Command::NewAssembly { .. }
                    | Command::Open { .. }
                    | Command::Undo { .. }
                    | Command::Redo { .. }
            );
        if mutation && request.expected_revision.is_none() {
            return Err(Error::new(
                "revision_required",
                "GUIの編集にはquery.projectで得たexpected_revisionを指定してください",
            ));
        }
        if self.job.is_some()
            && !matches!(
                request.command,
                Command::Query { .. } | Command::Cancel { .. }
            )
        {
            return Err(Error::new(
                "busy",
                "GUIの読込・計算が完了してから操作してください",
            ));
        }
        if !matches!(
            request.command,
            Command::Query { .. } | Command::Cancel { .. }
        ) {
            self.commit_edit();
        }
        if request
            .expected_revision
            .is_some_and(|r| r != self.engine.revision())
        {
            return Err(Error::new(
                "revision_conflict",
                format!("現在のrevisionは{}です", self.engine.revision()),
            ));
        }
        if matches!(request.command, Command::Undo { .. } | Command::Redo { .. }) {
            let undo = matches!(request.command, Command::Undo { .. });
            if undo && !self.engine.can_undo() || !undo && !self.engine.can_redo() {
                return Err(Error::new("history_empty", "復元する編集がありません"));
            }
            if undo {
                self.undo();
            } else {
                self.redo();
            }
            return Ok(Outcome {
                revision: self.engine.revision(),
                changed: true,
                created: Vec::new(),
                data: serde_json::Value::Null,
            });
        }
        let reset = matches!(
            request.command,
            Command::Open { .. } | Command::NewAssembly { .. }
        );
        let save = matches!(request.command, Command::Save { .. });
        let trace = matches!(request.command, Command::Trace { .. });
        self.engine.set_catalogs(self.catalogs.clone());
        let outcome = self.engine.apply(request)?;
        self.adopt_automation_outcome(outcome, reset, save, trace)
    }
    fn adopt_automation_outcome(
        &mut self,
        outcome: Outcome,
        reset: bool,
        save: bool,
        trace: bool,
    ) -> raytrace_engine::Result<Outcome> {
        if outcome.changed {
            self.project = self.engine.project().clone();
            if reset {
                self.path = None;
                self.viewing_explicit_record = false;
                self.component = 0;
                self.selected = 0;
                self.source = 0;
                self.reset_presentation_history(false);
            } else {
                self.history.begin(outcome.revision);
                self.commit_edit();
            }
            self.dirty = true;
            self.stale = self
                .result
                .as_ref()
                .is_some_and(|r| r.revision != self.engine.revision());
            self.status = "外部操作で光学系を更新しました".into();
        }
        if save {
            self.path = outcome.data["path"].as_str().map(PathBuf::from);
            self.mark_saved();
        }
        if trace && let Some(id) = outcome.created.first() {
            self.automation_job = Some(id.clone());
            self.control = Some(self.engine.job_control(id)?);
            self.status = "光線を追跡中…".into();
        }
        Ok(outcome)
    }
    fn poll_automation_result(&mut self) {
        let Some(job) = self.automation_job.clone() else {
            return;
        };
        let result = self.engine.apply(Request {
            expected_revision: None,
            command: Command::Query {
                query: Query::Job { job: job.clone() },
            },
        });
        match result {
            Ok(outcome) if outcome.data["status"] == "running" => (),
            Ok(outcome) => {
                self.automation_job = None;
                self.control = None;
                match self.engine.completed_record(&job) {
                    Ok(Some((revision, record))) => {
                        let paths = if record.report.paths.is_empty() {
                            jobs::record_viewer_paths(&record).unwrap_or_default()
                        } else {
                            record.report.paths.clone()
                        };
                        let mut report = record.report;
                        report.paths = paths;
                        self.result = Some(ResultView {
                            revision,
                            scene: record.calculation.scene.clone(),
                            calculation: record.calculation,
                            report,
                            seconds: record.trace_seconds,
                            texture: None,
                            screen: None,
                            roi_cache: None,
                        });
                        self.stale = revision != self.engine.revision();
                        self.status = "計算完了".into();
                    }
                    _ if outcome.data["status"] == "cancelled" => {
                        self.status = "計算を中止しました".into()
                    }
                    _ => self.error = Some(outcome.data["error"].to_string()),
                }
            }
            Err(error) => {
                self.automation_job = None;
                self.control = None;
                self.error = Some(error.to_string());
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn gui_additions_after_undo_keep_discarded_ids_reserved() {
        let mut app = App::demo();
        app.append_component(Component::detector());
        app.commit_edit();
        let previous = app
            .project
            .assembly
            .as_ref()
            .unwrap()
            .components
            .last()
            .unwrap()
            .id
            .clone();
        app.undo();
        app.append_component(Component::detector());
        app.commit_edit();
        let next = app
            .project
            .assembly
            .as_ref()
            .unwrap()
            .components
            .last()
            .unwrap()
            .id
            .clone();
        assert_ne!(previous, next);
        app.undo();
        let a = app.project.assembly.as_mut().unwrap();
        let mut copy = a.components[0].clone();
        copy.id = Default::default();
        a.components.push(copy);
        app.refresh_components();
        app.commit_edit();
        assert_ne!(
            previous,
            app.project
                .assembly
                .as_ref()
                .unwrap()
                .components
                .last()
                .unwrap()
                .id
        );
        assert_ne!(
            next,
            app.project
                .assembly
                .as_ref()
                .unwrap()
                .components
                .last()
                .unwrap()
                .id
        );
    }
    #[test]
    fn opening_another_gui_model_invalidates_an_older_client_revision() {
        let mut app = App::demo();
        let old = app.engine.revision();
        app.project = Project::new_assembly();
        app.reset_history(true);
        assert_ne!(app.engine.revision(), old);
        let result = app.apply_automation(Request {
            expected_revision: Some(old),
            command: Command::SetProperty {
                target: "project".into(),
                change: raytrace_engine::PropertyChange::Name("stale edit".into()),
            },
        });
        assert_eq!(result.unwrap_err().code, "revision_conflict");
        assert_ne!(app.project.scene.name, "stale edit");
    }
    #[test]
    fn json_edit_and_gui_undo_share_a_session_and_detect_stale_revisions() {
        let mut app = App::demo();
        let before = serde_json::to_value(&app.project).unwrap();
        let id = app.project.assembly.as_ref().unwrap().components[2]
            .id
            .to_string();
        let command = Command::SetProperty {
            target: id,
            change: raytrace_engine::PropertyChange::PositionMm([0., 0., 80.]),
        };
        assert!(
            app.apply_automation(Request {
                expected_revision: None,
                command: command.clone()
            })
            .is_err()
        );
        app.apply_automation(Request {
            expected_revision: Some(0),
            command: command.clone(),
        })
        .unwrap();
        assert_eq!(
            app.project.assembly.as_ref().unwrap().components[2]
                .pose
                .translation_mm
                .0[2],
            80.
        );
        assert!(
            app.apply_automation(Request {
                expected_revision: Some(0),
                command
            })
            .is_err()
        );
        app.undo();
        assert_eq!(serde_json::to_value(&app.project).unwrap(), before);
        app.redo();
        assert_eq!(
            app.project.assembly.as_ref().unwrap().components[2]
                .pose
                .translation_mm
                .0[2],
            80.
        );
    }
}
