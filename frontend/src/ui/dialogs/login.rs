use std::time::Instant;

use eframe::egui::{self, load::SizedTexture, vec2, Color32, Direction, Id, Label, Layout, Rect, RichText, Sense, Ui};

use common::taskmanager::{LoginSmsRequest, SubmitLoginSmsRequest, TaskRequest};

use crate::app::{Dialog, Myapp};
use crate::ui::dialogs::{LoginMethod, LoginState, QrState};
use crate::ui::modal::{self, ModalAction};
use crate::ui::theme::{palette, Palette};
use crate::ui::widgets::{self, button_ex, FieldOpts, Filter, Kind, Size};

const QR_SIZE: f32 = 200.0;

pub fn show(app: &mut Myapp, ctx: &egui::Context) {
    let mut st = match std::mem::replace(&mut app.dialog, Dialog::None) {
        Dialog::Login(st) => st,
        other => {
            app.dialog = other;
            return;
        }
    };
    let action = modal::modal(ctx, "login_dialog", "登录 B 站账号", 440.0, true, |ui| {
        body(app, &mut st, ui);
        ModalAction::None
    });
    if matches!(app.dialog, Dialog::None) {
        app.dialog = Dialog::Login(st);
        // 放回后再关闭，close_dialog 会顺带取消扫码轮询
        if action == ModalAction::Close {
            app.close_dialog();
        }
    }
}

fn body(app: &mut Myapp, st: &mut LoginState, ui: &mut Ui) {
    let p = palette(ui.ctx());
    let methods = [(LoginMethod::Qr, "扫码"), (LoginMethod::Sms, "短信"), (LoginMethod::Cookie, "Cookie")];
    if widgets::segmented(ui, &mut st.method, &methods) {
        app.cancel_login_requests(st);
        st.error = None;
    }
    if st.method == LoginMethod::Qr && st.qr == QrState::Idle {
        app.start_login_qr(st);
    }
    ui.add_space(8.0);
    match st.method {
        LoginMethod::Qr => qr_section(app, st, ui, &p),
        LoginMethod::Sms => sms_section(app, st, ui, &p),
        LoginMethod::Cookie => cookie_section(app, st, ui, &p),
    }
}

fn qr_section(app: &mut Myapp, st: &mut LoginState, ui: &mut Ui, p: &Palette) {
    // 过期状态不带地址，记住最后一张二维码，过期时淡化显示
    let last_url_id = Id::new("login_qr_last_url");
    let mut refresh = false;
    ui.vertical_centered(|ui| {
        ui.add_space(4.0);
        match &st.qr {
            QrState::Idle | QrState::Loading => {
                widgets::skeleton(ui, QR_SIZE, QR_SIZE);
                widgets::small(ui, "正在获取二维码…", p.text_3);
            }
            QrState::Ready { url } => {
                ui.data_mut(|d| d.insert_temp(last_url_id, url.clone()));
                qr_box(ui, url, 1.0);
                if st.busy {
                    ui.add(egui::Spinner::new().size(16.0));
                    ui.label(RichText::new("已扫码，正在登录…").color(p.text_2));
                } else if let Some(error) = &st.error {
                    ui.add(Label::new(RichText::new(error).color(p.danger)).wrap(true));
                    refresh = button_ex(ui, "刷新二维码", Kind::Secondary, Size::Md, false, true).clicked();
                } else {
                    ui.label(RichText::new("打开哔哩哔哩 App 扫码登录").color(p.text_2));
                }
            }
            QrState::Expired => {
                let last = ui.data(|d| d.get_temp::<String>(last_url_id)).unwrap_or_default();
                let rect = if last.is_empty() {
                    let (rect, _) = ui.allocate_exact_size(vec2(QR_SIZE, QR_SIZE), Sense::hover());
                    ui.painter().rect_filled(rect, 8.0, p.sunken);
                    rect
                } else {
                    qr_box(ui, &last, 0.3)
                };
                // 按钮叠在二维码中间；用独立子 Ui，避免把父布局的光标拉回二维码内部
                let mut over = ui.child_ui(
                    Rect::from_center_size(rect.center(), vec2(QR_SIZE, 40.0)),
                    Layout::centered_and_justified(Direction::TopDown),
                );
                refresh = button_ex(&mut over, "刷新二维码", Kind::Primary, Size::Lg, false, true).clicked();
                widgets::small(ui, "二维码已过期", p.text_3);
            }
            QrState::Failed(error) => {
                ui.add_space(24.0);
                ui.add(Label::new(RichText::new(format!("二维码登录失败：{error}")).color(p.danger)).wrap(true));
                refresh = button_ex(ui, "重试", Kind::Secondary, Size::Md, false, true).clicked();
                ui.add_space(24.0);
            }
        }
        ui.add_space(4.0);
    });
    if refresh {
        st.error = None;
        app.start_login_qr(st);
    }
}

/// 白底方块里的二维码（深色主题也用白底黑码，便于识别）；opacity 用于过期时淡化
fn qr_box(ui: &mut Ui, url: &str, opacity: f32) -> Rect {
    let (rect, _) = ui.allocate_exact_size(vec2(QR_SIZE, QR_SIZE), Sense::hover());
    ui.painter().rect_filled(rect, 8.0, Color32::WHITE.gamma_multiply(opacity));
    let texture = widgets::qr_texture(ui.ctx(), "login", url);
    egui::Image::from_texture(SizedTexture::from_handle(&texture))
        .tint(Color32::WHITE.gamma_multiply(opacity))
        .paint_at(ui, rect.shrink(8.0));
    rect
}

fn sms_section(app: &mut Myapp, st: &mut LoginState, ui: &mut Ui, p: &Palette) {
    let cooldown = st
        .sms_cooldown_until
        .map_or(0, |until| until.saturating_duration_since(Instant::now()).as_secs_f32().ceil() as u64);

    let mut send = false;
    ui.vertical(|ui| {
        ui.spacing_mut().item_spacing.y = 4.0;
        widgets::small(ui, "手机号", p.text_2);
        ui.horizontal(|ui| {
            let field_w = ui.available_width() - 136.0 - 8.0;
            let phone = widgets::text_field(ui, &mut st.phone, FieldOpts {
                hint: "11 位手机号",
                prefix: Some("+86"),
                filter: Filter::Digits,
                width: Some(field_w),
                id: Some("login_phone"),
                ..Default::default()
            });
            st.phone.truncate(11);
            if phone.changed() {
                st.captcha_key.clear();
                st.sms_code.clear();
                st.sms_task = None;
                st.sms_login_task = None;
                st.account_job = None;
                st.sms_busy = false;
                st.busy = false;
            }
            let (text, busy, enabled) = if st.sms_busy {
                ("发送中…".to_owned(), true, false)
            } else if cooldown > 0 {
                (format!("{cooldown} 秒后重新获取"), false, false)
            } else {
                ("获取验证码".to_owned(), false, st.phone.len() == 11)
            };
            send = button_ex(ui, &text, Kind::Secondary, Size::Md, busy, enabled).clicked();
        });
    });
    if send {
        let request = TaskRequest::LoginSmsRequest(LoginSmsRequest {
            phone: st.phone.clone(),
            client: app.client.clone(),
            custom_config: app.custom_config.clone(),
            local_captcha: app.local_captcha.clone(),
        });
        match app.task_manager.submit_task(request) {
            Ok(task_id) => {
                log::info!("短信验证码发送中... 任务ID: {}", task_id);
                st.sms_task = Some(task_id);
                st.sms_busy = true;
                st.error = None;
            }
            Err(e) => {
                log::error!("提交短信任务失败: {}", e);
                st.error = Some(format!("验证码发送失败：{e}"));
            }
        }
    }

    ui.add_space(4.0);
    let code = widgets::text_field(ui, &mut st.sms_code, FieldOpts {
        label: Some("验证码"),
        hint: "短信验证码",
        filter: Filter::Digits,
        id: Some("login_sms_code"),
        ..Default::default()
    });
    let entered = code.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));

    ui.add_space(4.0);
    if let Some(error) = &st.error {
        ui.add(Label::new(RichText::new(error).color(p.danger)).wrap(true));
    }
    // captcha_key 只有发送短信成功后才有，没有它无法提交登录
    let ready = st.phone.len() == 11 && !st.sms_code.is_empty() && !st.captcha_key.is_empty();
    let text = if st.busy { "登录中…" } else { "登录" };
    let clicked = button_ex(ui, text, Kind::Primary, Size::Xl, st.busy, ready).clicked();
    if (clicked || entered) && ready && !st.busy {
        let request = TaskRequest::SubmitLoginSmsRequest(SubmitLoginSmsRequest {
            phone: st.phone.clone(),
            code: st.sms_code.clone(),
            captcha_key: st.captcha_key.clone(),
            client: app.client.clone(),
        });
        match app.task_manager.submit_task(request) {
            Ok(task_id) => {
                log::info!("短信验证码登录中... 任务ID: {}", task_id);
                st.sms_login_task = Some(task_id);
                st.busy = true;
                st.error = None;
            }
            Err(e) => {
                log::error!("提交短信登录任务失败: {}", e);
                st.error = Some(format!("登录失败：{e}"));
            }
        }
    }
}

fn cookie_section(app: &mut Myapp, st: &mut LoginState, ui: &mut Ui, p: &Palette) {
    widgets::text_field(ui, &mut st.cookie, FieldOpts {
        label: Some("Cookie"),
        hint: "粘贴完整 Cookie",
        multiline_rows: 4,
        id: Some("login_cookie"),
        ..Default::default()
    });
    widgets::small(ui, "Cookie 等同于登录凭证，不要发给别人", p.text_3);

    ui.add_space(4.0);
    if let Some(error) = &st.error {
        ui.add(Label::new(RichText::new(error).color(p.danger)).wrap(true));
    }
    let cookie = st.cookie.trim();
    let text = if st.busy { "登录中…" } else { "登录" };
    if button_ex(ui, text, Kind::Primary, Size::Xl, st.busy, !cookie.is_empty()).clicked() {
        st.account_job = Some(app.jobs.spawn_add_account(cookie.to_owned(), app.client.clone(), app.default_ua.clone()));
        st.busy = true;
        st.error = None;
    }
}
