use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::{Duration, Instant};

use eframe::egui;
use reqwest::{header, Client};
use serde_json::Value;

use backend::taskmanager::TaskManagerImpl;
use common::account::Account;
use common::captcha::LocalCaptcha;
use common::cookie_manager::CookieManager;
use common::login::QrCodeLoginStatus;
use common::push::PushConfig;
use common::show_orderlist::OrderResponse;
use common::taskmanager::*;
use common::utility::CustomConfig;
use common::utils::Config;

use crate::state::draft::{parse_project_input, ParseResult, TaskDraft};
use crate::state::jobs::{self, Jobs, ServiceState};
use crate::state::logs::{LogFilter, LogStore};
use crate::state::persist::{self, Contact, SaveState, ThemeMode, UiConfig};
use crate::state::remote::Remote;
use crate::state::tasks::{TaskBoard, TasksTab};
use crate::state::BuyerList;
use crate::ui;
use crate::ui::dialogs::{AddBuyerState, LoginState, QrState};
use crate::ui::theme::Tone;
use crate::ui::toast::{ToastAction, Toasts};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Page { NewTask, Tasks, Accounts, Settings, About }

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AccountsTab { Buyers, Orders }

pub enum Dialog {
    None,
    Login(LoginState),
    AddBuyer(AddBuyerState),
    /// 付款信息在 board.payments[board.payment_index]
    Payment,
    ConfirmSignOut { uid: i64, busy: bool },
    PolicyBlocked { message: String },
}

impl Dialog {
    pub(crate) fn show_pending_payment(&mut self, board: &mut TaskBoard) {
        // 付款提醒等当前对话框关闭，避免中断登录等尚未完成的操作。
        match self {
            Self::None if board.take_pending_payment_dialog() => {
                board.payment_index = board.payments.len() - 1;
                *self = Self::Payment;
            }
            Self::Payment => {
                // 新提醒加入队列，当前展示的付款项保持选中。
                board.take_pending_payment_dialog();
            }
            _ => {}
        }
    }
}

/// 设置页“发送测试推送”的状态，按推送任务 ID 匹配结果
#[derive(Clone, Debug, PartialEq)]
pub enum PushTest { Idle, Busy(String), Sent, Failed(String) }

pub struct AccountManager {
    pub accounts: Vec<Account>,
}

pub struct Myapp {
    pub app: String,
    pub version: String,
    pub policy: Option<Value>,
    pub public_key: String,
    pub machine_id: String,
    pub task_manager: Box<dyn TaskManager>,
    pub account_manager: AccountManager,
    pub push_config: PushConfig,
    pub config: Config,
    pub custom_config: CustomConfig,
    /// 登录用的 client，登录后账号有自己的 cookie_manager
    pub client: Client,
    pub default_ua: String,
    pub local_captcha: LocalCaptcha,
    /// 构造 BilibiliTicket 时传入的状态刷新间隔
    pub status_delay: usize,
    /// 主公告（抢票页）
    pub announce1: Option<String>,
    pub announce2: Option<String>,
    /// 任务页公告
    pub announce3: Option<String>,
    /// 不允许运行时的说明
    pub announce4: Option<String>,
    pub default_avatar_texture: Option<egui::TextureHandle>,

    pub page: Page,
    pub dialog: Dialog,
    pub toasts: Toasts,
    pub draft: TaskDraft,
    pub project_history: Vec<String>,
    pub buyers: HashMap<i64, Remote<BuyerList>>,
    pub orders: HashMap<i64, Remote<OrderResponse>>,
    pub board: TaskBoard,
    pub logs: LogStore,
    pub log_filter: LogFilter,
    pub log_panel_open: bool,
    pub jobs: Jobs,
    pub service: ServiceState,
    pub ui_config: UiConfig,
    /// key = uid 字符串
    pub contacts: HashMap<String, Contact>,
    pub accounts_selected: Option<i64>,
    pub accounts_tab: AccountsTab,
    /// 已提交过头像请求的账号，同一 uid 只请求一次
    pub avatar_requested: HashSet<i64>,
    pub settings_dirty_at: Option<Instant>,
    pub settings_save: SaveState,
    /// 推送渠道展开状态：Bark、PushPlus、Server 酱、钉钉、企业微信、邮件、Gotify
    pub settings_expanded: [bool; 7],
    pub push_test: PushTest,
    pub tasks_tab: TasksTab,
    /// 抢票页公告 1、任务页公告 3 本次运行内关闭
    pub notice_dismissed: [bool; 2],
    /// 已应用的 (dark, scale)，变化时才重设样式
    pub applied_theme: Option<(bool, f32)>,
}

impl Myapp {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        ui::theme::configure_fonts(&cc.egui_ctx);
        let config = match Config::load_config() {
            Ok(load_config) => {
                log::info!("配置文件加载成功");
                load_config
            }
            Err(e) => {
                log::error!("配置文件加载失败: {}", e);
                log::info!("尝试迁移json配置");
                match Config::load_json_config() {
                    Ok(load_config) => {
                        log::info!("配置文件加载成功");
                        match load_config.save_config() {
                            Ok(_) => {
                                log::info!("配置文件保存成功");
                                match Config::delete_json_config() {
                                    Ok(_) => log::info!("旧配置文件删除成功"),
                                    Err(e) => log::error!("旧配置文件删除失败: {}", e),
                                }
                                log::info!("迁移成功");
                            }
                            Err(e) => log::error!("配置文件保存失败: {}", e),
                        }
                        load_config
                    }
                    Err(e) => {
                        log::error!("迁移失败: {}", e);
                        let cfg = Config::new();
                        match cfg.save_config() {
                            Ok(_) => log::info!("配置文件保存成功"),
                            Err(e) => log::error!("配置文件保存失败: {}", e),
                        }
                        cfg
                    }
                }
            }
        };

        let accounts = Config::load_all_accounts();
        let first_uid = accounts.first().map(|a| a.uid);
        let mut draft = TaskDraft::default();
        draft.account_uid = first_uid;
        draft.project_input = "1007551".to_string();
        draft.parsed = parse_project_input(&draft.project_input);
        draft.skip_words = persist::load_skip_words(&config);

        let mut app = Self {
            app: String::from("BRT"),
            version: String::from("6.6.1"),
            policy: None,
            public_key: String::from("-----BEGIN PUBLIC KEY-----\nMIIBIjANBgkqhkiG9w0BAQEFAAOCAQ8AMIIBCgKCAQEApTAS0RElXIs4Kr0bO4n8\nJB+eBFF/TwXUlvtOM9FNgHjK8m13EdwXaLy9zjGTSQr8tshSRr0dQ6iaCG19Zo2Y\nXfvJrwQLqdezMN+ayMKFy58/S9EGG3Np2eGgKHUPnCOAlRicqWvBdQ/cxzTDNCxa\nORMZdJRoBvya7JijLLIC3CoqmMc6Fxe5i8eIP0zwlyZ0L0C1PQ82BcWn58y7tlPY\nTCz12cWnuKwiQ9LSOfJ4odJJQK0k7rXxwBBsYxULRno0CJ3rKfApssW4cfITYVax\nFtdbu0IUsgEeXs3EzNw8yIYnsaoZlFwLS8SMVsiAFOy2y14lR9043PYAQHm1Cjaf\noQIDAQAB\n-----END PUBLIC KEY-----"),
            machine_id: common::machine_id::get_machine_id_ob(),
            task_manager: Box::new(TaskManagerImpl::new()),
            account_manager: AccountManager { accounts },
            push_config: match serde_json::from_value::<PushConfig>(config["push_config"].clone()) {
                Ok(config) => config,
                Err(e) => {
                    log::warn!("无法解析推送配置: {}, 使用默认值", e);
                    PushConfig::new()
                }
            },
            custom_config: match serde_json::from_value::<CustomConfig>(config["custom_config"].clone()) {
                Ok(config) => config,
                Err(e) => {
                    log::warn!("无法解析自定义配置: {}, 使用默认值", e);
                    CustomConfig::new()
                }
            },
            ui_config: persist::load_ui(&config),
            contacts: persist::load_contacts(&config),
            config: config.clone(),
            client: Client::new(),
            default_ua: String::from("Mozilla/5.0 (Linux; Android 6.0; Nexus 5 Build/MRA58N) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/134.0.0.0 Mobile Safari/537.36"),
            local_captcha: LocalCaptcha::new(),
            status_delay: 2,
            announce1: None,
            announce2: None,
            announce3: None,
            announce4: None,
            default_avatar_texture: load_default_avatar(&cc.egui_ctx),
            page: Page::NewTask,
            dialog: Dialog::None,
            toasts: Toasts::default(),
            draft,
            project_history: persist::load_project_history(&config),
            buyers: HashMap::new(),
            orders: HashMap::new(),
            board: TaskBoard::default(),
            logs: LogStore::default(),
            log_filter: LogFilter::default(),
            log_panel_open: false,
            jobs: Jobs::new(),
            service: ServiceState::Connecting,
            accounts_selected: first_uid,
            accounts_tab: AccountsTab::Buyers,
            avatar_requested: HashSet::new(),
            settings_dirty_at: None,
            settings_save: SaveState::Idle,
            settings_expanded: [false; 7],
            push_test: PushTest::Idle,
            tasks_tab: TasksTab::Running,
            notice_dismissed: [false; 2],
            applied_theme: None,
        };
        // 初始化每个账号的 client
        for account in &mut app.account_manager.accounts {
            account.ensure_client();
            log::debug!("为账号 {} 初始化了专属客户端", account.name);
            log::debug!("machine_id: {}", app.machine_id);
        }

        //初始化client和ua
        let random_value = generate_random_string(8);
        app.default_ua = format!(
            "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/134.0.0.0 Safari/537.36 Edg/134.0.0.0 {}",
            random_value
        );
        if config["custom_config"]["enable_custom_ua"].as_bool().unwrap_or(false) && !config["custom_config"]["custom_ua"].is_null() {
            app.default_ua = config["custom_config"]["custom_ua"].as_str().unwrap_or(&app.default_ua).to_string();
        }
        app.client = create_client(app.default_ua.clone());

        app.jobs.spawn_policy(
            app.client.clone(),
            app.app.clone(),
            app.version.clone(),
            app.machine_id.clone(),
            app.public_key.clone(),
        );
        app
    }

    pub fn account(&self, uid: i64) -> Option<&Account> {
        self.account_manager.accounts.iter().find(|a| a.uid == uid)
    }

    /// 账号头像，未加载时用默认头像
    pub fn avatar_of(&self, uid: i64) -> Option<&egui::TextureHandle> {
        self.account(uid)
            .and_then(|a| a.avatar_texture.as_ref())
            .or(self.default_avatar_texture.as_ref())
    }

    /// 该账号进行中的任务数
    pub fn running_for(&self, uid: i64) -> usize {
        self.board.running.iter().filter(|t| t.uid == uid).count()
    }

    pub fn open_login(&mut self) {
        self.close_dialog();
        self.dialog = Dialog::Login(LoginState::default());
    }

    pub fn open_add_buyer(&mut self, uid: i64) {
        self.close_dialog();
        self.dialog = Dialog::AddBuyer(AddBuyerState::new(uid));
    }

    /// 关闭当前对话框；登录框关闭时取消扫码轮询。付款框关闭只隐藏，不移出队列
    pub fn close_dialog(&mut self) {
        if let Dialog::Login(mut st) = std::mem::replace(&mut self.dialog, Dialog::None) {
            self.cancel_login_requests(&mut st);
        }
    }

    /// 切换方式或关闭后停止扫码轮询，并丢弃尚未返回的一次性登录请求结果。
    pub fn cancel_login_requests(&mut self, st: &mut LoginState) {
        if let Some(task_id) = st.qr_task.take() {
            let _ = self.task_manager.cancel_task(&task_id);
        }
        st.sms_task = None;
        st.sms_login_task = None;
        st.qr_job = None;
        st.account_job = None;
        st.qr = QrState::Idle;
        st.busy = false;
        st.sms_busy = false;
    }

    /// 请求新二维码；先取消旧的轮询任务
    pub fn start_login_qr(&mut self, st: &mut LoginState) {
        if let Some(task_id) = st.qr_task.take() {
            let _ = self.task_manager.cancel_task(&task_id);
        }
        st.qr = QrState::Loading;
        st.qr_job = Some(self.jobs.spawn_login_qr(self.client.clone()));
    }

    /// 扫码、短信登录拿到 cookie 后交给后台线程添加账号
    pub fn login_with_cookie(&mut self, cookie: &str) {
        let Dialog::Login(st) = &mut self.dialog else { return };
        st.account_job = Some(self.jobs.spawn_add_account(cookie.to_owned(), self.client.clone(), self.default_ua.clone()));
    }

    /// 按 draft 中的账号和项目 ID 获取项目信息
    pub fn fetch_project(&mut self) {
        let Some(uid) = self.draft.account_uid else { return };
        let ParseResult::Id { id, referer } = self.draft.parsed.clone() else { return };
        self.remember_project(&id);
        let Some(cookie_manager) = self.account(uid).and_then(|a| a.cookie_manager.clone()) else {
            self.draft.project = Remote::Failed { message: "账号登录信息失效，请重新登录".into(), at: Instant::now() };
            return;
        };
        log::info!("提交获取{}project请求 ", id);
        let request = TaskRequest::GetTicketInfoRequest(GetTicketInfoRequest {
            task_id: "".to_string(),
            uid,
            project_id: id.clone(),
            cookie_manager,
            referer_link: referer.clone(),
        });
        log::debug!("referer_link: {}", referer);
        self.draft.project_for = Some((uid, id));
        self.draft.project = match self.task_manager.submit_task(request) {
            Ok(task_id) => {
                log::info!("提交获取project请求，任务ID: {}", task_id);
                Remote::Loading { task_id, since: Instant::now() }
            }
            Err(e) => {
                log::error!("提交获取project请求失败: {}", e);
                Remote::Failed { message: e, at: Instant::now() }
            }
        };
    }

    pub fn request_buyers(&mut self, uid: i64) {
        let Some(cookie_manager) = self.account(uid).and_then(|a| a.cookie_manager.clone()) else {
            self.buyers.insert(uid, Remote::Failed { message: "账号登录信息失效，请重新登录".into(), at: Instant::now() });
            return;
        };
        log::info!("提交获取购票人信息请求");
        let request = TaskRequest::GetBuyerInfoRequest(GetBuyerInfoRequest {
            task_id: "".to_string(),
            uid,
            cookie_manager,
        });
        let state = match self.task_manager.submit_task(request) {
            Ok(task_id) => {
                log::info!("提交获取购票人信息请求，任务ID: {}", task_id);
                Remote::Loading { task_id, since: Instant::now() }
            }
            Err(e) => {
                log::error!("提交获取购票人信息请求失败: {}", e);
                Remote::Failed { message: e, at: Instant::now() }
            }
        };
        self.buyers.insert(uid, state);
    }

    pub fn open_account_orders(&mut self, uid: i64) {
        if !self.account(uid).is_some_and(|account| account.is_login && account.cookie_manager.is_some()) {
            self.toasts.warn(format!("请先登录原购票账号 UID {uid}"));
            return;
        }
        self.close_dialog();
        self.accounts_selected = Some(uid);
        self.accounts_tab = AccountsTab::Orders;
        self.page = Page::Accounts;
        if !matches!(self.orders.get(&uid), Some(Remote::Loading { .. })) {
            self.request_orders(uid);
        }
    }

    pub fn request_orders(&mut self, uid: i64) {
        let Some(account) = self.account_manager.accounts.iter().find(|a| a.uid == uid) else { return };
        let Some(cookie_manager) = account.cookie_manager.clone() else {
            log::error!("账号 {} 的客户端未初始化", account.name);
            self.orders.insert(uid, Remote::Failed { message: "账号登录信息失效，请重新登录".into(), at: Instant::now() });
            return;
        };
        let state = match submit_get_total_order(&mut self.task_manager, cookie_manager, account) {
            Ok(task_id) => Remote::Loading { task_id, since: Instant::now() },
            Err(e) => Remote::Failed { message: e, at: Instant::now() },
        };
        self.orders.insert(uid, state);
    }

    /// 账号头像未加载且有地址时，各提交一次后台加载
    fn request_missing_avatars(&mut self, ctx: &egui::Context) {
        for account in &self.account_manager.accounts {
            if account.avatar_texture.is_some() || self.avatar_requested.contains(&account.uid) {
                continue;
            }
            let (Some(url), Some(cookie_manager)) = (account.avatar_url.clone(), account.cookie_manager.clone()) else { continue };
            if url.is_empty() {
                continue;
            }
            self.avatar_requested.insert(account.uid);
            self.jobs.spawn_avatar(ctx.clone(), account.uid, cookie_manager, url);
        }
    }

    pub fn mark_settings_dirty(&mut self) {
        self.settings_dirty_at = Some(Instant::now());
    }

    pub fn save_settings_now(&mut self) {
        self.settings_dirty_at = None;
        self.settings_save = SaveState::Saving;
        self.settings_save = match persist::save_settings(&mut self.config, &self.push_config, &self.custom_config, &self.ui_config) {
            Ok(()) => SaveState::Saved,
            Err(e) => SaveState::Failed(e),
        };
    }

    pub fn save_skip_words(&mut self) {
        if let Err(e) = persist::save_skip_words(&mut self.config, &self.draft.skip_words) {
            log::error!("保存跳过关键词失败: {}", e);
        }
    }

    pub fn remember_project(&mut self, id: &str) {
        self.project_history.retain(|entry| entry != id);
        self.project_history.insert(0, id.to_owned());
        if let Err(e) = persist::save_project_history(&mut self.config, &self.project_history) {
            log::error!("保存项目历史失败: {}", e);
            self.toasts.push(Tone::Danger, "保存项目历史失败", Some(e), None);
        }
    }

    pub fn save_contacts(&mut self) {
        if let Err(e) = persist::save_contacts(&mut self.config, &self.contacts) {
            log::error!("保存联系人失败: {}", e);
        }
    }

    /// 原 check_policy：写入公告；不允许运行时打开不可关闭的对话框
    pub fn apply_policy(&mut self, policy: Value, connected: bool) {
        self.service = if connected { ServiceState::Connected } else { ServiceState::Offline };
        // 在公告入口移除群宣传，让所有页面共用过滤后的内容。
        let visible_announcement = |key: &str| {
            policy.get(key).and_then(|v| v.as_str()).and_then(|text| {
                let text = text.lines()
                    .filter(|line| !line.contains('群') && !line.contains("1012873573")
                        && !line.contains("qm.qq.com") && !line.contains("qun.qq.com"))
                    .collect::<Vec<_>>()
                    .join("\n");
                let text = text.trim();
                if text.is_empty() { None } else { Some(text.to_string()) }
            })
        };
        self.announce1 = visible_announcement("announcement1");
        self.announce2 = visible_announcement("announcement2");
        self.announce3 = visible_announcement("announcement3");
        if let Some(announcement) = policy.get("announcement4").and_then(|v| v.as_str()) {
            self.announce4 = Some(announcement.to_string());
        }
        let allow_run = policy.get("allow_run").and_then(|v| v.as_bool()).unwrap_or(false);
        if !allow_run {
            if let Some(announcement) = self.announce4.clone() {
                log::error!("公告: {}", announcement);
            }
            log::error!("根据策略配置，当前版本不允许运行");
            self.close_dialog();
            self.dialog = Dialog::PolicyBlocked {
                message: self.announce4.clone().unwrap_or_else(|| "根据策略配置，当前版本不允许运行".to_string()),
            };
        }
        let ticket_id = policy["ticket_id"].as_str().unwrap_or("").to_string();
        if self.draft.project_input.is_empty() && !ticket_id.is_empty() {
            self.draft.parsed = parse_project_input(&ticket_id);
            self.draft.project_input = ticket_id;
        }
        self.policy = Some(policy);
    }

    fn process_task_results(&mut self) {
        for result in self.task_manager.get_results() {
            match result {
                TaskResult::QrCodeLoginResult(qrcode_result) => {
                    if !matches!(&self.dialog, Dialog::Login(st) if st.qr_task.as_ref() == Some(&qrcode_result.task_id)) {
                        continue;
                    }
                    match qrcode_result.status {
                    QrCodeLoginStatus::Success(_) => {
                        log::info!("二维码登录成功!");
                        if let Dialog::Login(st) = &mut self.dialog {
                            st.qr_task = None;
                            st.busy = true;
                        }
                        if let Some(cookie_str) = qrcode_result.cookie {
                            self.login_with_cookie(&cookie_str);
                        }
                    }
                    QrCodeLoginStatus::Failed(err) => {
                        log::error!("二维码登录失败: {}", err);
                        if let Dialog::Login(st) = &mut self.dialog {
                            st.qr_task = None;
                            st.qr = QrState::Failed(err);
                        }
                    }
                    QrCodeLoginStatus::Expired => {
                        log::warn!("二维码已过期，请刷新");
                        if let Dialog::Login(st) = &mut self.dialog {
                            st.qr_task = None;
                            st.qr = QrState::Expired;
                        }
                    }
                    _ => {}
                    }
                }
                TaskResult::LoginSmsResult(sms_result) => {
                    let Dialog::Login(st) = &mut self.dialog else { continue };
                    if st.sms_task.as_ref() != Some(&sms_result.task_id) {
                        continue;
                    }
                    st.sms_task = None;
                    st.sms_busy = false;
                    if sms_result.success {
                        st.captcha_key = sms_result.message.clone();
                        st.sms_cooldown_until = Some(Instant::now() + Duration::from_secs(60));
                        st.error = None;
                        log::debug!("发送captchakey：{}", sms_result.message);
                        log::info!("短信发送成功 ");
                    } else {
                        log::error!("短信发送失败: {}", sms_result.message);
                        st.error = Some(format!("验证码发送失败：{}", sms_result.message));
                    }
                }
                TaskResult::SubmitSmsLoginResult(submit_result) => {
                    let Dialog::Login(st) = &mut self.dialog else { continue };
                    if st.sms_login_task.as_ref() != Some(&submit_result.task_id) {
                        continue;
                    }
                    st.sms_login_task = None;
                    match (submit_result.success, submit_result.cookie) {
                        (true, Some(cookie_str)) => self.login_with_cookie(&cookie_str),
                        _ => {
                            log::error!("短信登录失败: {}", submit_result.message);
                            if let Dialog::Login(st) = &mut self.dialog {
                                st.busy = false;
                                st.error = Some(format!("登录失败：{}", submit_result.message));
                            }
                        }
                    }
                }
                TaskResult::PushResult(push_result) => {
                    if push_result.success {
                        log::info!("推送成功: {}", push_result.message);
                    } else {
                        log::error!("推送失败: {}", push_result.message);
                    }
                    if self.push_test == PushTest::Busy(push_result.task_id.clone()) {
                        self.push_test = if push_result.success { PushTest::Sent } else { PushTest::Failed(push_result.message) };
                    }
                }
                TaskResult::GetAllorderRequestResult(order_result) => {
                    let Ok(uid) = order_result.account_id.parse::<i64>() else { continue };
                    if !matches!(self.orders.get(&uid), Some(Remote::Loading { task_id, .. }) if *task_id == order_result.task_id) {
                        continue;
                    }
                    let state = match (order_result.success, order_result.order_info) {
                        (true, Some(orders)) => {
                            log::info!("账号 {} 订单请求成功", order_result.account_id);
                            Remote::Ready { value: orders, at: Instant::now() }
                        }
                        _ => {
                            log::error!("账号 {} 订单请求失败", order_result.account_id);
                            Remote::Failed { message: order_result.message, at: Instant::now() }
                        }
                    };
                    self.orders.insert(uid, state);
                }
                TaskResult::GetTicketInfoResult(info_result) => {
                    let matches = matches!(&self.draft.project, Remote::Loading { task_id, .. } if *task_id == info_result.task_id);
                    if !matches {
                        continue;
                    }
                    self.draft.project = match (info_result.success, info_result.ticket_info) {
                        (true, Some(response)) => {
                            log::debug!("获取project信息成功: {:?}", response.data);
                            let info = response.data;
                            self.draft.screen_idx = if info.screen_list.is_empty() { None } else { Some(0) };
                            self.draft.ticket_id = None;
                            Remote::Ready { value: info, at: Instant::now() }
                        }
                        _ => {
                            log::error!("获取project信息失败: {}", info_result.message);
                            Remote::Failed { message: info_result.message, at: Instant::now() }
                        }
                    };
                }
                TaskResult::GetBuyerInfoResult(buyer_result) => {
                    let uid = buyer_result.uid;
                    let matches = matches!(self.buyers.get(&uid), Some(Remote::Loading { task_id, .. }) if *task_id == buyer_result.task_id);
                    if !matches {
                        continue;
                    }
                    let state = match (buyer_result.success, buyer_result.buyer_info) {
                        (true, Some(response)) if response.errno == 0 => {
                            log::debug!("获取购票人信息成功: {:?}", response.data);
                            Remote::Ready {
                                value: BuyerList { list: response.data.list, from_cache: buyer_result.message.contains("缓存") },
                                at: Instant::now(),
                            }
                        }
                        (true, Some(response)) => {
                            log::error!("获取购票人信息失败: {:?}", response);
                            let message = if response.msg.is_empty() { response.message } else { response.msg };
                            Remote::Failed { message, at: Instant::now() }
                        }
                        _ => {
                            log::error!("获取购票人信息失败: {}", buyer_result.message);
                            Remote::Failed { message: buyer_result.message, at: Instant::now() }
                        }
                    };
                    self.buyers.insert(uid, state);
                    self.select_buyer_after_refresh(uid);
                }
            }
        }
    }

    /// 添加购票人后刷新列表，把新加的人按姓名选上（单选模式替换原选择）
    fn select_buyer_after_refresh(&mut self, uid: i64) {
        if self.draft.account_uid != Some(uid) {
            return;
        }
        let Some(name) = self.draft.select_after_refresh.clone() else { return };
        let Some(list) = self.buyers.get(&uid).and_then(|r| r.value()) else { return };
        let Some(buyer) = list.list.iter().find(|b| b.name == name) else { return };
        let single = self.draft.mode != 2 && self.draft.project.value().and_then(|p| p.id_bind) == Some(1);
        if single {
            self.draft.selected_buyers = vec![buyer.id];
        } else if !self.draft.selected_buyers.contains(&buyer.id) && self.draft.selected_buyers.len() < 10 {
            self.draft.selected_buyers.push(buyer.id);
        }
        self.draft.select_after_refresh = None;
    }

    fn sync_theme(&mut self, ctx: &egui::Context, frame: &eframe::Frame) {
        let info = frame.info();
        let dark = match self.ui_config.theme {
            ThemeMode::System => info.system_theme == Some(eframe::Theme::Dark),
            ThemeMode::Light => false,
            ThemeMode::Dark => true,
        };
        let wanted = (dark, self.ui_config.scale);
        if self.applied_theme != Some(wanted) {
            ui::theme::apply_theme(ctx, dark, self.ui_config.scale, info.native_pixels_per_point.unwrap_or(1.0));
            self.applied_theme = Some(wanted);
        }
    }

    fn ingest_logs(&mut self) {
        for line in self.logs.ingest() {
            let mut title: String = line.msg.chars().take(80).collect();
            if title.len() < line.msg.len() {
                title.push('…');
            }
            self.toasts.push(Tone::Danger, title, None, Some(("查看日志".into(), ToastAction::OpenLog { label: line.label })));
        }
    }

    fn handle_shortcuts(&mut self, ctx: &egui::Context) {
        if !matches!(self.dialog, Dialog::None) {
            let blocked = matches!(self.dialog, Dialog::PolicyBlocked { .. });
            if !blocked && ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
                self.close_dialog();
            }
            return;
        }
        let pages = [
            (egui::Key::Num1, Page::NewTask),
            (egui::Key::Num2, Page::Tasks),
            (egui::Key::Num3, Page::Accounts),
            (egui::Key::Num4, Page::Settings),
        ];
        for (key, page) in pages {
            if ctx.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, key)) {
                self.page = page;
            }
        }
    }
}

impl eframe::App for Myapp {
    fn update(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        // egui 默认是“响应式”重绘：仅在有输入事件（鼠标/键盘）或显式 request_repaint 时才刷新。
        // 抢票任务运行在后台独立线程，不受界面影响；但若不主动请求重绘，鼠标静止时倒计时、
        // 实时日志、任务卡片和后台结果处理会看起来“卡住”。这里请求一个稳定的基线重绘。
        ctx.request_repaint_after(Duration::from_millis(100));
        self.sync_theme(ctx, frame);

        jobs::poll(self);
        self.process_task_results();
        self.ingest_logs();
        if self.board.refresh(&mut *self.task_manager) {
            frame.request_user_attention(egui::UserAttentionType::Critical);
            frame.focus();
        }
        self.request_missing_avatars(ctx);

        if self.settings_dirty_at.map_or(false, |at| at.elapsed() >= Duration::from_millis(800)) {
            self.save_settings_now();
        }

        self.handle_shortcuts(ctx);
        self.dialog.show_pending_payment(&mut self.board);
        ui::shell::render(self, ctx);
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        if self.settings_dirty_at.is_some() {
            self.save_settings_now();
        }
    }
}

/// 沿用旧的订单请求；返回任务 ID 以便匹配加载状态
pub fn submit_get_total_order(task_manager: &mut Box<dyn TaskManager>, cookie_manager: Arc<CookieManager>, account: &Account) -> Result<String, String> {
    let request = TaskRequest::GetAllorderRequest(GetAllorderRequest {
        task_id: "".to_string(),
        account_id: account.uid.to_string(),
        cookie_manager,
        cookies: account.cookie.clone(),
        status: TaskStatus::Pending,
        start_time: None,
    });
    match task_manager.submit_task(request) {
        Ok(task_id) => {
            log::info!("订单请求提交成功，任务ID: {}", task_id);
            Ok(task_id)
        }
        Err(e) => {
            log::error!("查看全部订单请求提交失败：{}", e);
            Err(e)
        }
    }
}

pub fn create_client(user_agent: String) -> Client {
    let mut headers = header::HeaderMap::new();
    log::info!("客户端 User-Agent: {}", user_agent);
    headers.insert(
        header::USER_AGENT,
        header::HeaderValue::from_str(&user_agent)
            .unwrap_or_else(|_| header::HeaderValue::from_static("Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36")),
    );
    Client::builder().default_headers(headers).cookie_store(true).build().unwrap_or_default()
}

fn load_default_avatar(ctx: &egui::Context) -> Option<egui::TextureHandle> {
    let image = image::load_from_memory(include_bytes!("../assets/default_avatar.jpg")).ok()?;
    let size = [image.width() as usize, image.height() as usize];
    let rgba = image.to_rgba8();
    Some(ctx.load_texture(
        "default_avatar",
        egui::ColorImage::from_rgba_unmultiplied(size, rgba.as_flat_samples().as_slice()),
        Default::default(),
    ))
}

fn generate_random_string(length: usize) -> String {
    use rand::distributions::Alphanumeric;
    use rand::{thread_rng, Rng};
    thread_rng().sample_iter(&Alphanumeric).take(length).map(|c| c as char).collect()
}
