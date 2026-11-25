mod feed;

use egui::{Color32, Sense, Stroke, Ui, pos2, vec2};
use feed::{Book, Snapshot};
use std::{sync::mpsc, time::Duration};

pub struct Dashboard {
    rx: mpsc::Receiver<Snapshot>,
    data: Snapshot,
}
impl Dashboard {
    pub fn new(ctx: &egui::Context) -> Self {
        let (tx, rx) = mpsc::sync_channel(1);
        feed::spawn(tx, ctx.clone());
        Self { rx, data: Snapshot::default() }
    }
}
impl eframe::App for Dashboard {
    fn ui(&mut self, ui: &mut Ui, _frame: &mut eframe::Frame) {
        while let Ok(data) = self.rx.try_recv() { self.data = data; }
        ui.ctx().request_repaint_after(Duration::from_secs(1));
        let now = chrono::Utc::now().timestamp();
        let active = (self.data.window..self.data.window + 300).contains(&now);
        ui.horizontal(|ui| {
            ui.heading("POLYMARKET");
            ui.label("ETH / 5 MIN");
            if active { ui.monospace(format!("{}s to close", self.data.window + 300 - now)); }
        });
        if !active {
            ui.label("Waiting for ETH market");
            return;
        }
        if let Some(start) = chrono::DateTime::from_timestamp(self.data.window, 0) {
            ui.label(format!("{} · {}", start.with_timezone(&chrono::Local).format("%b %d %H:%M %Z"), self.data.status));
        }
        egui::ScrollArea::vertical().show(ui, |ui| {
            ui.label("Best ask (thick) · Best bid (thin) · Up (green) · Down (blue)");
            let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 220.), Sense::hover());
            let plot = ui.painter_at(rect);
            let point = |t: f32, p: f32| pos2(
                rect.left() + 42. + (rect.width() - 58.) * t / 300.,
                rect.bottom() - 23. - (rect.height() - 34.) * p,
            );
            for p in [0., 0.25, 0.5, 0.75, 1.] {
                plot.line_segment([point(0.,p),point(300.,p)], Stroke::new(1., Color32::DARK_GRAY));
                plot.text(pos2(rect.left(), point(0.,p).y), egui::Align2::LEFT_CENTER,
                    format!("{:.0}¢", 100. * p), egui::FontId::monospace(11.), ui.visuals().text_color());
            }
            for t in [0.,60.,120.,180.,240.,300.] {
                plot.text(point(t,0.) + vec2(0.,5.), egui::Align2::CENTER_TOP,
                    format!("{t:.0}s"), egui::FontId::monospace(10.), ui.visuals().text_color());
            }
            ui.columns(2, |columns| {
                for (index, (side, color)) in [("Up", Color32::LIGHT_GREEN), ("Down", Color32::LIGHT_BLUE)].into_iter().enumerate() {
                    let history = &self.data.history[index];
                    for &(t, _, ask) in history { plot.circle_filled(point(t,ask), 1.5, color); }
                    for points in history.windows(2) {
                        let (a,b) = (points[0], points[1]);
                        if b.0 - a.0 <= 9. {
                            plot.line_segment([point(a.0,a.1),point(b.0,b.1)], Stroke::new(1.,color));
                            plot.line_segment([point(a.0,a.2),point(b.0,b.2)], Stroke::new(2.5,color));
                        }
                    }
                    let ui = &mut columns[index];
                    ui.colored_label(color, side);
                    if let Some(book) = &self.data.books[index] { depth(ui, side, book, now); }
                    else { ui.label("Waiting for quotes"); }
                }
            });
        });
    }
}
fn depth(ui: &mut Ui, side: &str, book: &Book, now: i64) {
    let age = (now as f64 - book.seconds().unwrap_or(0.)).max(0.);
    ui.monospace(format!("{age:.0}s old{}", if age > 10. { " · STALE" } else { "" }));
    if let (Some(bid), Some(ask)) = (book.bids.first(), book.asks.first()) {
        ui.monospace(format!("Spread {:.2}¢", (ask.price().unwrap() - bid.price().unwrap()) * 100.));
    }
    egui::Grid::new(side).striped(true).show(ui, |ui| {
        for label in ["Bid ¢","Bid shares","Ask ¢","Ask shares"] { ui.strong(label); }
        ui.end_row();
        for i in 0..book.bids.len().max(book.asks.len()) {
            for level in [book.bids.get(i), book.asks.get(i)] {
                if let Some(level) = level {
                    ui.monospace(format!("{:.2}", level.price().unwrap() * 100.));
                    ui.monospace(&level.size);
                } else { ui.label("—"); ui.label("—"); }
            }
            ui.end_row();
        }
    });
}
