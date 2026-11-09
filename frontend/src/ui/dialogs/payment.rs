use std::time::SystemTime;

use eframe::egui::{self, load::SizedTexture, pos2, vec2, Color32, Rect, RichText, Sense, Shape, Stroke, Ui};

use crate::app::{Dialog, Myapp};
use crate::state::tasks::PaymentInfo;
use crate::ui::modal::{self, ModalAction};
use crate::ui::pages::accounts::open_order_detail;
use crate::ui::theme::{bold, palette, Palette, Tone};
use crate::ui::widgets::{self, Kind, Size};

const WIDTH: f32 = 660.0;
const LEFT_W: f32 = 236.0;
const QR_SIZE: f32 = 196.0;

enum Action { Prev, Next, CopyOrder(String), CopyLink, OpenOrder(String), AccountOrders, Dismiss }

pub fn show(app: &mut Myapp, ctx: &egui::Context) {
    let len = app.board.payments.len();
    if len == 0 {
        app.dialog = Dialog::None;
        return;
    }
    app.board.payment_index = app.board.payment_index.min(len - 1);
    let index = app.board.payment_index;
    let pay = app.board.payments[index].clone();
    let badge = app.board.seq_of(&pay.task_id).map(|seq| (format!("No.{:04}", seq), Tone::Success));
    let p = palette(ctx);

    let mut action = None;
    let result = modal::modal_with_badge(ctx, "payment_dialog", "抢到了，请尽快付款", badge, WIDTH, true, |ui| {
        if len > 1 {
            ui.horizontal(|ui| {
                widgets::small(ui, format!("另有 {} 条付款提醒", len - 1), p.text_2);
                if widgets::button_ex(ui, "上一笔", Kind::Ghost, Size::Sm, false, index > 0).clicked() {
                    action = Some(Action::Prev);
                }
                if widgets::button_ex(ui, "下一笔", Kind::Ghost, Size::Sm, false, index + 1 < len).clicked() {
                    action = Some(Action::Next);
                }
            });
            ui.add_space(4.0);
        }

        ui.horizontal_top(|ui| {
            ui.spacing_mut().item_spacing.x = 0.0;
            let left = ui.vertical(|ui| {
                ui.set_width(LEFT_W);
                qr_column(ui, &p, &pay);
            });
            let right = ui.vertical(|ui| {
                ui.add_space(4.0);
                ui.horizontal_top(|ui| {
                    ui.add_space(24.0);
                    ui.vertical(|ui| {
                        ui.spacing_mut().item_spacing = vec2(8.0, 8.0);
                        if let Some(a) = info_column(ui, &p, &pay) {
                            action = Some(a);
                        }
                    });
                });
            });
            // 左栏右边的 2px 虚线分隔，高度取两栏中较高者
            let x = left.response.rect.right();
            let bottom = left.response.rect.bottom().max(right.response.rect.bottom());
            ui.painter().extend(Shape::dashed_line(
                &[pos2(x, left.response.rect.top()), pos2(x, bottom)],
                Stroke::new(2.0, p.border),
                6.0,
                4.0,
            ));
        });

        modal::footer(ui, |ui| {
            // 从右到左添加
            if widgets::button_ex(ui, "关闭付款提醒", Kind::Primary, Size::Lg, false, true).clicked() {
                action = Some(Action::Dismiss);
            }
            if let Some(order_id) = &pay.order_id {
                if widgets::button_ex(ui, "在浏览器打开订单", Kind::Secondary, Size::Lg, false, true).clicked() {
                    action = Some(Action::OpenOrder(order_id.clone()));
                }
            } else if widgets::button_ex(ui, "查看原账号订单", Kind::Secondary, Size::Lg, false, true).clicked() {
                action = Some(Action::AccountOrders);
            }
            if widgets::button_ex(ui, "复制支付链接", Kind::Secondary, Size::Lg, false, true).clicked() {
                action = Some(Action::CopyLink);
            }
        });
        ModalAction::None
    });

    match action {
        Some(Action::Prev) => app.board.payment_index = index.saturating_sub(1),
        Some(Action::Next) => app.board.payment_index = (index + 1).min(len - 1),
        Some(Action::CopyOrder(order_id)) => {
            widgets::copy_text(ctx, order_id);
            app.toasts.success("已复制订单号");
        }
        Some(Action::CopyLink) => {
            widgets::copy_text(ctx, pay.code_url.clone());
            app.toasts.success("已复制支付链接");
        }
        Some(Action::OpenOrder(order_id)) => open_order_detail(ctx, &order_id),
        Some(Action::AccountOrders) => app.open_account_orders(pay.uid),
        Some(Action::Dismiss) => {
            app.board.dismiss_payment_reminder();
            if app.board.payments.is_empty() {
                app.close_dialog();
            }
        }
        None => {}
    }
    // × 关闭只隐藏，付款项留在队列里，可从任务卡“去付款”再打开
    if result == ModalAction::Close {
        app.close_dialog();
    }
}

/// 白底二维码方块（深色主题也用白底黑码，保证能扫）
fn qr_column(ui: &mut Ui, p: &Palette, pay: &PaymentInfo) {
    ui.vertical_centered(|ui| {
        let texture = widgets::qr_texture(ui.ctx(), "payment", &pay.code_url);
        let box_size = QR_SIZE + 16.0;
        let (rect, _) = ui.allocate_exact_size(vec2(box_size, box_size), Sense::hover());
        ui.painter().rect_filled(rect, 8.0, Color32::WHITE);
        let qr_rect = Rect::from_center_size(rect.center(), vec2(QR_SIZE, QR_SIZE));
        egui::Image::from_texture(SizedTexture::from_handle(&texture)).paint_at(ui, qr_rect);
        ui.add_space(2.0);
        widgets::small(ui, "使用微信或支付宝扫码", p.text_2);
    });
}

fn info_column(ui: &mut Ui, p: &Palette, pay: &PaymentInfo) -> Option<Action> {
    let mut action = None;
    ui.add(egui::Label::new(RichText::new(&pay.project_name).color(p.text_2)).wrap(true));
    ui.add(
        egui::Label::new(
            RichText::new(format!("{} · {} × {}", pay.screen_name, pay.ticket_name, pay.count)).color(p.text),
        )
        .wrap(true),
    );

    ui.vertical(|ui| {
        ui.spacing_mut().item_spacing.y = 0.0;
        widgets::small(ui, "应付", p.text_3);
        ui.label(RichText::new(widgets::money(pay.amount_fen)).font(bold(26.0)).color(p.text));
    });

    if let Some(order_id) = &pay.order_id {
        ui.horizontal(|ui| {
            ui.label(RichText::new(format!("订单号 {order_id}")).color(p.text));
            if widgets::button_ex(ui, "复制", Kind::Ghost, Size::Sm, false, true).clicked() {
                action = Some(Action::CopyOrder(order_id.clone()));
            }
        });
    } else {
        widgets::small(ui, "订单号暂不可用", p.text_3);
    }

    let at = chrono::DateTime::<chrono::Local>::from(pay.at).format("%H:%M:%S");
    let passed = SystemTime::now().duration_since(pay.at).map(|d| d.as_secs()).unwrap_or(0);
    widgets::small(
        ui,
        format!("确认于 {} · 已过去 {:02}:{:02}", at, passed / 60, passed % 60),
        p.text_3,
    );

    widgets::callout(ui, "超时未付款的订单会被自动取消", Tone::Warning, None);
    action
}
