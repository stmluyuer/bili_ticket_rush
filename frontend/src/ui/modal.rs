use eframe::egui::{self, pos2, vec2, Align, Align2, CursorIcon, FontId, Id, Layout, Margin, Order, Sense, Shape, Stroke, Ui};

use super::theme::{bold, palette, Tone};
use super::widgets;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ModalAction { None, Close }

const PAD: f32 = 20.0;

pub fn modal(ctx: &egui::Context, id: &str, title: &str, width: f32, closable: bool, body: impl FnOnce(&mut Ui) -> ModalAction) -> ModalAction {
    modal_with_badge(ctx, id, title, None, width, closable, body)
}

/// 遮罩 + 居中对话框。内容放在 Foreground 层的 Area 里：egui 0.23 的 Window 固定在 Middle 层，
/// 点击同层的遮罩会把遮罩提到窗口上面
pub fn modal_with_badge(
    ctx: &egui::Context,
    id: &str,
    title: &str,
    badge: Option<(String, Tone)>,
    width: f32,
    closable: bool,
    body: impl FnOnce(&mut Ui) -> ModalAction,
) -> ModalAction {
    let p = palette(ctx);
    let screen = ctx.screen_rect();
    egui::Area::new(Id::new(id).with("scrim"))
        .order(Order::Middle)
        .fixed_pos(screen.min)
        .show(ctx, |ui| {
            ui.painter().rect_filled(screen, 0.0, p.scrim);
            // 吞掉遮罩上的点击，避免穿透到页面
            ui.allocate_rect(screen, Sense::click());
        });

    let mut action = ModalAction::None;
    egui::Area::new(Id::new(id))
        .order(Order::Foreground)
        .anchor(Align2::CENTER_CENTER, vec2(0.0, 0.0))
        .show(ctx, |ui| {
            egui::Frame::none()
                .fill(p.surface)
                .stroke(Stroke::new(1.0, p.border))
                .rounding(12.0)
                .shadow(egui::epaint::Shadow { extrusion: 24.0, color: egui::Color32::from_black_alpha(48) })
                .show(ui, |ui| {
                    ui.set_width(width);
                    ui.spacing_mut().item_spacing.y = 0.0;
                    egui::Frame::none()
                        .inner_margin(Margin { left: PAD, right: 14.0, top: 14.0, bottom: 12.0 })
                        .show(ui, |ui| {
                            ui.set_width(width - PAD - 14.0);
                            ui.horizontal(|ui| {
                                ui.set_min_height(28.0);
                                ui.label(egui::RichText::new(title).font(bold(17.0)).color(p.text));
                                if let Some((text, tone)) = badge {
                                    widgets::pill(ui, text, tone);
                                }
                                if closable {
                                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                        let (rect, resp) = ui.allocate_exact_size(vec2(28.0, 28.0), Sense::click());
                                        if resp.hovered() {
                                            ui.painter().rect_filled(rect, 6.0, p.sunken);
                                        }
                                        let color = if resp.hovered() { p.text } else { p.text_3 };
                                        ui.painter().text(rect.center(), Align2::CENTER_CENTER, "×", FontId::proportional(18.0), color);
                                        if resp.on_hover_cursor(CursorIcon::PointingHand).clicked() {
                                            action = ModalAction::Close;
                                        }
                                    });
                                }
                            });
                        });
                    let (line, _) = ui.allocate_exact_size(vec2(width, 1.0), Sense::hover());
                    ui.painter().rect_filled(line, 0.0, p.border);
                    egui::Frame::none().inner_margin(PAD).show(ui, |ui| {
                        ui.set_width(width - PAD * 2.0);
                        ui.spacing_mut().item_spacing.y = 8.0;
                        if body(ui) == ModalAction::Close {
                            action = ModalAction::Close;
                        }
                    });
                });
        });
    action
}

/// 对话框底栏：sunken 底，横向撑满并贴住对话框下缘。必须是 body 里最后添加的内容
pub fn footer(ui: &mut Ui, add: impl FnOnce(&mut Ui)) {
    let p = palette(ui.ctx());
    ui.add_space(8.0);
    let bg = ui.painter().add(Shape::Noop);
    let top = ui.cursor().top();
    ui.add_space(16.0);
    // 外包一层 horizontal：直接在纵向布局里用居中的横向子布局会占满剩余高度
    let row = ui.horizontal(|ui| {
        ui.set_min_height(36.0);
        ui.with_layout(Layout::right_to_left(Align::Center), add);
    });
    let max = ui.max_rect();
    let rect = egui::Rect::from_min_max(
        pos2(max.left() - PAD, top),
        pos2(max.right() + PAD, row.response.rect.bottom() + PAD),
    );
    let rounding = egui::Rounding { nw: 0.0, ne: 0.0, sw: 12.0, se: 12.0 };
    ui.painter().set(bg, Shape::Vec(vec![
        Shape::rect_filled(rect, rounding, p.sunken),
        Shape::line_segment([rect.left_top(), rect.right_top()], Stroke::new(1.0, p.border)),
    ]));
}
