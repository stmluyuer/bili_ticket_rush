use eframe::egui::{self, Label, RichText};

use crate::app::{Dialog, Myapp};
use crate::ui::modal::{self, ModalAction};
use crate::ui::theme::palette;
use crate::ui::widgets::{button_ex, Kind, Size};

pub fn show(app: &mut Myapp, ctx: &egui::Context) {
    match app.dialog {
        Dialog::ConfirmSignOut { uid, busy } => sign_out(app, ctx, uid, busy),
        Dialog::PolicyBlocked { ref message } => policy_blocked(ctx, message),
        _ => {}
    }
}

fn sign_out(app: &mut Myapp, ctx: &egui::Context, uid: i64, busy: bool) {
    let Some(account) = app.account(uid).cloned() else {
        app.close_dialog();
        return;
    };
    let running = app.running_for(uid);
    let p = palette(ctx);
    let text = if running > 0 {
        format!("该账号有 {running} 个进行中的任务，请先在任务页停止。")
    } else {
        format!("退出 {}？会删除本地保存的登录信息，之后需要重新登录。", account.name)
    };

    let mut confirm = false;
    let mut cancel = false;
    let action = modal::modal(ctx, "confirm_sign_out", "退出登录", 400.0, true, |ui| {
        ui.add(Label::new(RichText::new(text).color(p.text)).wrap(true));
        modal::footer(ui, |ui| {
            let label = if busy { "正在退出…" } else { "退出登录" };
            confirm = button_ex(ui, label, Kind::Danger, Size::Md, busy, running == 0).clicked();
            cancel = button_ex(ui, "取消", Kind::Secondary, Size::Md, false, true).clicked();
        });
        ModalAction::None
    });

    if confirm {
        app.jobs.spawn_sign_out(account);
        app.dialog = Dialog::ConfirmSignOut { uid, busy: true };
    } else if cancel || action == ModalAction::Close {
        app.close_dialog();
    }
}

/// 策略不允许运行：不可关闭，只能退出
fn policy_blocked(ctx: &egui::Context, message: &str) {
    let p = palette(ctx);
    modal::modal(ctx, "policy_blocked", "无法运行", 440.0, false, |ui| {
        ui.add(Label::new(RichText::new(message).color(p.text)).wrap(true));
        modal::footer(ui, |ui| {
            if button_ex(ui, "退出", Kind::Primary, Size::Md, false, true).clicked() {
                std::process::exit(1);
            }
        });
        ModalAction::None
    });
}
