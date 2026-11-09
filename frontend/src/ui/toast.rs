use std::time::Instant;

use eframe::egui::{self, pos2, vec2, Align2, CursorIcon, FontId, Id, Margin, Order, RichText, Sense, Stroke};

use super::theme::{palette, Tone};
use crate::app::{Myapp, Page};

#[derive(Clone, Debug, PartialEq)]
pub enum ToastAction {
    GoTasks,
    /// 打开任务页日志面板，并按任务标签筛选（None 为全部）
    OpenLog { label: Option<String> },
}

struct Toast {
    id: u64,
    tone: Tone,
    title: String,
    detail: Option<String>,
    action: Option<(String, ToastAction)>,
    count: u32,
    created: Instant,
    /// 剩余显示秒数，悬停时不减少
    left: f32,
    last_tick: Instant,
    hovered: bool,
}

#[derive(Default)]
pub struct Toasts {
    items: Vec<Toast>,
    next_id: u64,
}

const MAX_TOASTS: usize = 4;
const FADE_SECS: f32 = 0.2;

impl Toasts {
    pub fn push(&mut self, tone: Tone, title: impl Into<String>, detail: Option<String>, action: Option<(String, ToastAction)>) {
        let title = title.into();
        let now = Instant::now();
        let duration = match tone {
            Tone::Warning => 6.0,
            Tone::Danger => 8.0,
            _ => 4.0,
        };
        if let Some(index) = self.items.iter().position(|t| t.title == title && t.created.elapsed().as_secs() < 10) {
            let mut t = self.items.remove(index);
            t.count += 1;
            t.created = now;
            t.left = duration;
            t.last_tick = now;
            t.hovered = false;
            self.items.push(t);
        } else {
            self.next_id += 1;
            self.items.push(Toast {
            id: self.next_id,
            tone,
            title,
            detail,
            action,
            count: 1,
            created: now,
            left: duration,
            last_tick: now,
            hovered: false,
            });
        }
        let mut excess = self.items.iter().filter(|t| t.left > 0.0).count().saturating_sub(MAX_TOASTS);
        for t in &mut self.items {
            if excess > 0 && t.left > 0.0 {
                t.left = 0.0;
                excess -= 1;
            }
        }
    }

    pub fn info(&mut self, title: impl Into<String>) {
        self.push(Tone::Info, title, None, None);
    }

    pub fn success(&mut self, title: impl Into<String>) {
        self.push(Tone::Success, title, None, None);
    }

    pub fn neutral(&mut self, title: impl Into<String>) {
        self.push(Tone::Neutral, title, None, None);
    }

    pub fn warn(&mut self, title: impl Into<String>) {
        self.push(Tone::Warning, title, None, None);
    }

    pub fn danger(&mut self, title: impl Into<String>) {
        self.push(Tone::Danger, title, None, None);
    }
}

/// 右下角堆叠，新的在下
pub fn show(app: &mut Myapp, ctx: &egui::Context) {
    let now = Instant::now();
    for t in &mut app.toasts.items {
        if !t.hovered {
            t.left -= now.duration_since(t.last_tick).as_secs_f32();
        }
        t.last_tick = now;
    }
    // 消息隐藏后仍保留到去重窗口结束，避免 4/8 秒消失后在 10 秒内重新计为一条。
    app.toasts.items.retain(|t| t.left > 0.0 || t.created.elapsed().as_secs() < 10);
    if !app.toasts.items.iter().any(|t| t.left > 0.0) {
        return;
    }

    let p = palette(ctx);
    let mut dismissed = Vec::new();
    let mut triggered = None;
    egui::Area::new(Id::new("toasts"))
        .order(Order::Foreground)
        .anchor(Align2::RIGHT_BOTTOM, vec2(-16.0, -16.0))
        .show(ctx, |ui| {
            ui.set_width(320.0);
            ui.spacing_mut().item_spacing.y = 8.0;
            for t in app.toasts.items.iter_mut().filter(|t| t.left > 0.0) {
                let alpha = (t.left / FADE_SECS).clamp(0.0, 1.0);
                let (tone_color, _) = p.tone(t.tone);
                let frame = egui::Frame::none()
                    .fill(p.surface.gamma_multiply(alpha))
                    .stroke(Stroke::new(1.0, p.border.gamma_multiply(alpha)))
                    .rounding(10.0)
                    .shadow(egui::epaint::Shadow { extrusion: 12.0, color: egui::Color32::from_black_alpha((40.0 * alpha) as u8) })
                    .inner_margin(Margin { left: 16.0, right: 10.0, top: 10.0, bottom: 10.0 })
                    .show(ui, |ui| {
                        ui.set_width(320.0 - 26.0);
                        ui.horizontal_top(|ui| {
                            ui.vertical(|ui| {
                                ui.set_width(320.0 - 26.0 - 24.0);
                                ui.spacing_mut().item_spacing.y = 2.0;
                                let title = if t.count > 1 { format!("{} ×{}", t.title, t.count) } else { t.title.clone() };
                                ui.add(egui::Label::new(RichText::new(title).color(p.text.gamma_multiply(alpha))).wrap(true));
                                if let Some(detail) = &t.detail {
                                    let mut job = egui::text::LayoutJob::simple(
                                        detail.clone(), FontId::proportional(12.0), p.text_2.gamma_multiply(alpha), ui.available_width(),
                                    );
                                    job.wrap.max_rows = 2;
                                    ui.label(job);
                                }
                                if let Some((label, action)) = &t.action {
                                    let resp = ui.add(
                                        egui::Label::new(RichText::new(label).color(p.primary.gamma_multiply(alpha)))
                                            .sense(Sense::click()),
                                    );
                                    if resp.on_hover_cursor(CursorIcon::PointingHand).clicked() {
                                        triggered = Some(action.clone());
                                        dismissed.push(t.id);
                                    }
                                }
                            });
                            let (rect, close) = ui.allocate_exact_size(vec2(20.0, 20.0), Sense::click());
                            let color = if close.hovered() { p.text } else { p.text_3 };
                            ui.painter().text(rect.center(), Align2::CENTER_CENTER, "×", FontId::proportional(16.0), color.gamma_multiply(alpha));
                            if close.on_hover_cursor(CursorIcon::PointingHand).clicked() {
                                dismissed.push(t.id);
                            }
                        });
                    });
                let rect = frame.response.rect;
                // 左缘语气色竖条
                let bar = egui::Rect::from_min_max(pos2(rect.left() + 1.0, rect.top() + 8.0), pos2(rect.left() + 4.0, rect.bottom() - 8.0));
                ui.painter().rect_filled(bar, 1.5, tone_color.gamma_multiply(alpha));
                t.hovered = ui.rect_contains_pointer(rect);
            }
        });
    if app.toasts.items.iter().any(|t| t.left > 0.0 && t.left < FADE_SECS) {
        ctx.request_repaint();
    }
    for t in &mut app.toasts.items {
        if dismissed.contains(&t.id) {
            t.left = 0.0;
            t.hovered = false;
        }
    }
    match triggered {
        Some(ToastAction::GoTasks) => app.page = Page::Tasks,
        Some(ToastAction::OpenLog { label }) => {
            app.page = Page::Tasks;
            app.log_panel_open = true;
            app.log_filter.label = label;
        }
        None => {}
    }
}
