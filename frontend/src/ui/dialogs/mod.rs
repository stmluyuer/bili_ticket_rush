pub mod add_buyer;
pub mod confirm;
pub mod login;
pub mod payment;

use std::time::Instant;

use eframe::egui;

use crate::app::{Dialog, Myapp};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LoginMethod { Qr, Sms, Cookie }

#[derive(Clone, Debug, PartialEq)]
pub enum QrState {
    /// 尚未请求二维码
    Idle,
    Loading,
    Ready { url: String },
    Expired,
    Failed(String),
}

pub struct LoginState {
    pub method: LoginMethod,
    pub qr: QrState,
    /// 扫码轮询任务，刷新二维码或关闭对话框时取消
    pub qr_task: Option<String>,
    pub qr_job: Option<u64>,
    pub account_job: Option<u64>,
    pub sms_task: Option<String>,
    pub sms_login_task: Option<String>,
    pub phone: String,
    pub sms_code: String,
    /// 发送短信成功后服务端返回的 captcha_key，提交短信登录时带上
    pub captcha_key: String,
    pub sms_cooldown_until: Option<Instant>,
    pub sms_busy: bool,
    pub cookie: String,
    /// 短信登录或 Cookie 登录提交中
    pub busy: bool,
    pub error: Option<String>,
}

impl Default for LoginState {
    fn default() -> Self {
        Self {
            method: LoginMethod::Qr,
            qr: QrState::Idle,
            qr_task: None,
            qr_job: None,
            account_job: None,
            sms_task: None,
            sms_login_task: None,
            phone: String::new(),
            sms_code: String::new(),
            captcha_key: String::new(),
            sms_cooldown_until: None,
            sms_busy: false,
            cookie: String::new(),
            busy: false,
            error: None,
        }
    }
}

#[derive(Default)]
pub struct FieldErrors {
    pub name: Option<String>,
    pub phone: Option<String>,
    pub id_number: Option<String>,
}

pub struct AddBuyerState {
    pub uid: i64,
    pub job_id: Option<u64>,
    pub name: String,
    pub phone: String,
    /// 0 身份证 / 1 护照 / 2 港澳通行证 / 3 台湾通行证
    pub id_type: usize,
    pub id_number: String,
    pub as_default: bool,
    pub busy: bool,
    pub errors: FieldErrors,
    pub server_error: Option<String>,
}

impl AddBuyerState {
    pub fn new(uid: i64) -> Self {
        Self {
            uid,
            job_id: None,
            name: String::new(),
            phone: String::new(),
            id_type: 0,
            id_number: String::new(),
            as_default: false,
            busy: false,
            errors: FieldErrors::default(),
            server_error: None,
        }
    }
}

/// 对话框层：同一时间只显示一个
pub fn show(app: &mut Myapp, ctx: &egui::Context) {
    match app.dialog {
        Dialog::None => {}
        Dialog::Login(_) => login::show(app, ctx),
        Dialog::AddBuyer(_) => add_buyer::show(app, ctx),
        Dialog::Payment => payment::show(app, ctx),
        Dialog::ConfirmSignOut { .. } | Dialog::PolicyBlocked { .. } => confirm::show(app, ctx),
    }
}
