use eframe::egui::{self, pos2, vec2, Align, Align2, CursorIcon, FontId, Layout, Margin, Rect, Sense, Stroke, Ui};

use super::theme::{bold, palette, Palette};
use super::{dialogs, pages, toast};
use crate::app::{Myapp, Page};
use crate::state::jobs::ServiceState;

/// 侧栏 → 中央页面 → 对话框层 → 消息条层
pub fn render(app: &mut Myapp, ctx: &egui::Context) {
    let p = palette(ctx);
    sidebar(app, ctx, &p);
    egui::CentralPanel::default()
        .frame(egui::Frame::none().fill(p.bg))
        .show(ctx, |ui| match app.page {
            Page::NewTask => pages::new_task::render(app, ui),
            Page::Tasks => pages::tasks::render(app, ui),
            Page::Accounts => pages::accounts::render(app, ui),
            Page::Settings => pages::settings::render(app, ui),
            Page::About => pages::about::render(app, ui),
        });
    dialogs::show(app, ctx);
    toast::show(app, ctx);
}

fn sidebar(app: &mut Myapp, ctx: &egui::Context, p: &Palette) {
    egui::SidePanel::left("nav")
        .exact_width(168.0)
        .resizable(false)
        .show_separator_line(false)
        .frame(egui::Frame::none().fill(p.sidebar).inner_margin(Margin { left: 10.0, right: 10.0, top: 18.0, bottom: 12.0 }))
        .show(ctx, |ui| {
            let full = ui.max_rect().expand2(vec2(10.0, 0.0));
            ui.painter().vline(full.right() - 0.5, ui.clip_rect().y_range(), Stroke::new(1.0, p.border));

            ui.spacing_mut().item_spacing.y = 2.0;
            ui.horizontal(|ui| {
                ui.add_space(4.0);
                ui.vertical(|ui| {
                    ui.spacing_mut().item_spacing.y = 2.0;
                    ui.label(egui::RichText::new("BiliTicketRush").font(bold(15.0)).color(p.text));
                    ui.label(egui::RichText::new(format!("v{}", app.version)).font(FontId::monospace(11.0)).color(p.text_3));
                });
            });
            ui.add_space(18.0);

            let running = app.board.running.len();
            for (page, label, badge) in [
                (Page::NewTask, "抢票", 0),
                (Page::Tasks, "任务", running),
                (Page::Accounts, "账号", 0),
                (Page::Settings, "设置", 0),
            ] {
                if nav_item(ui, p, label, app.page == page, badge).clicked() {
                    app.page = page;
                }
            }

            ui.with_layout(Layout::bottom_up(Align::Min), |ui| {
                ui.spacing_mut().item_spacing.y = 2.0;
                let (color, text) = match app.service {
                    ServiceState::Connecting => (p.neutral, "正在连接…"),
                    ServiceState::Connected => (p.success, "公告服务已连接"),
                    ServiceState::Offline => (p.danger, "未连接公告服务"),
                };
                ui.horizontal(|ui| {
                    ui.add_space(8.0);
                    let (dot, _) = ui.allocate_exact_size(vec2(7.0, 16.0), Sense::hover());
                    ui.painter().circle_filled(dot.center(), 3.5, color);
                    ui.label(egui::RichText::new(text).size(12.0).color(p.text_3));
                });
                ui.add_space(6.0);
                if nav_item(ui, p, "关于", app.page == Page::About, 0).clicked() {
                    app.page = Page::About;
                }
            });
        });
}

/// 导航项：高 36；选中为 primary_soft 底、primary 字、左侧 3px 竖条
fn nav_item(ui: &mut Ui, p: &Palette, label: &str, selected: bool, badge: usize) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 36.0), Sense::click());
    if ui.is_rect_visible(rect) {
        let painter = ui.painter();
        if selected {
            painter.rect_filled(rect, 6.0, p.primary_soft);
            let bar = Rect::from_min_max(pos2(rect.left(), rect.top() + 8.0), pos2(rect.left() + 3.0, rect.bottom() - 8.0));
            painter.rect_filled(bar, 1.5, p.primary);
        } else if resp.hovered() {
            painter.rect_filled(rect, 6.0, p.sunken);
        }
        if resp.has_focus() {
            painter.rect_stroke(rect.shrink(1.0), 6.0, Stroke::new(2.0, p.primary));
        }
        let (font, color) = if selected { (bold(14.0), p.primary) } else { (FontId::proportional(14.0), p.text_2) };
        painter.text(pos2(rect.left() + 12.0, rect.center().y), Align2::LEFT_CENTER, label, font, color);
        if badge > 0 {
            let galley = painter.layout_no_wrap(badge.to_string(), FontId::monospace(11.0), p.info);
            let w = (galley.size().x + 12.0).max(20.0);
            let chip = Rect::from_center_size(pos2(rect.right() - 10.0 - w / 2.0, rect.center().y), vec2(w, 18.0));
            painter.rect_filled(chip, 9.0, p.info_soft);
            painter.galley(chip.center() - galley.size() / 2.0, galley);
        }
    }
    resp.on_hover_cursor(CursorIcon::PointingHand)
}
