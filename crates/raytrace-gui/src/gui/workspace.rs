//! The layout workspace keeps placement visible; reports and tracing controls open on demand.
use super::*;

impl App {
    pub(super) fn content(&mut self, ctx: &egui::Context) {
        egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(BG_CANVAS).inner_margin(20.0))
            .show(ctx, |ui| {
                self.import_notification(ui);
                if self.stale {
                    ui.horizontal(|ui| {
                        ui.colored_label(
                            WARNING,
                            "配置・条件を変更しました。計算すると結果が更新されます。",
                        );
                    });
                    ui.add_space(6.0);
                }
                match self.view {
                    View::Layout => self.layout_view(ui),
                    View::Screen => self.screen_view(ui, ctx),
                }
            });
    }
}
