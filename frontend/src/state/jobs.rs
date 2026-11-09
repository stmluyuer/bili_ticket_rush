//! 界面线程之外的一次性网络任务。每个任务开一个线程，需要异步时在线程内自建 tokio 运行时，
//! 结果经通道回到界面线程，由 `poll` 每帧收取。
use std::fs::File;
use std::io::{Read, Write};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use eframe::egui;
use jsonwebtoken::{decode, Algorithm, DecodingKey, Validation};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use common::account::{add_account, signout_account, Account};
use common::cookie_manager::CookieManager;
use common::taskmanager::{QrCodeLoginRequest, TaskRequest};
use common::utils::{load_texture_from_url, save_config};

use crate::app::{Dialog, Myapp};
use crate::ui::dialogs::QrState;
use crate::ui::theme::Tone;

/// 侧栏底部的公告服务状态
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ServiceState { Connecting, Connected, Offline }

pub struct BuyerForm {
    pub name: String,
    pub tel: String,
    /// 0 身份证 / 1 护照 / 2 港澳通行证 / 3 台湾通行证
    pub id_type: usize,
    pub personal_id: String,
    pub is_default: bool,
}

pub enum JobResult {
    Policy { policy: Value, connected: bool },
    /// Ok((qrcode_key, 登录地址))
    LoginQr { job_id: u64, result: Result<(String, String), String> },
    AddAccount { job_id: u64, result: Result<Account, String> },
    SignOut { uid: i64, name: String, result: Result<bool, String> },
    AddBuyer { job_id: u64, uid: i64, name: String, result: Result<(), String> },
    Avatar { uid: i64, texture: Option<egui::TextureHandle> },
}

pub struct Jobs {
    tx: Sender<JobResult>,
    rx: Receiver<JobResult>,
    next_job_id: u64,
}

impl Jobs {
    pub fn new() -> Self {
        let (tx, rx) = channel();
        Self { tx, rx, next_job_id: 0 }
    }

    fn spawn(&self, work: impl FnOnce() -> JobResult + Send + 'static) {
        let tx = self.tx.clone();
        std::thread::spawn(move || {
            let _ = tx.send(work());
        });
    }

    pub fn spawn_policy(&self, client: Client, app: String, version: String, machine_id: String, public_key: String) {
        self.spawn(move || {
            let rt = tokio::runtime::Runtime::new().unwrap();
            let (policy, connected) = rt.block_on(get_policy(&client, &app, &version, &machine_id, &public_key));
            JobResult::Policy { policy, connected }
        });
    }

    pub fn spawn_login_qr(&mut self, client: Client) -> u64 {
        self.next_job_id += 1;
        let job_id = self.next_job_id;
        self.spawn(move || {
            JobResult::LoginQr { job_id, result: common::login::qrcode_login(&client).map(|code| {
                let url = format!(
                    "https://account.bilibili.com/h5/account-h5/auth/scan-web?navhide=1&callback=close&qrcode_key={}&from=main-fe-header",
                    code
                );
                (code, url)
            }) }
        });
        job_id
    }

    pub fn spawn_add_account(&mut self, cookie: String, client: Client, ua: String) -> u64 {
        self.next_job_id += 1;
        let job_id = self.next_job_id;
        self.spawn(move || JobResult::AddAccount { job_id, result: add_account(&cookie, &client, &ua) });
        job_id
    }

    pub fn spawn_sign_out(&self, account: Account) {
        self.spawn(move || JobResult::SignOut {
            uid: account.uid,
            name: account.name.clone(),
            result: signout_account(&account),
        });
    }

    pub fn spawn_add_buyer(&mut self, uid: i64, cookie_manager: Arc<CookieManager>, form: BuyerForm) -> u64 {
        self.next_job_id += 1;
        let job_id = self.next_job_id;
        self.spawn(move || JobResult::AddBuyer {
            job_id,
            uid,
            name: form.name.clone(),
            result: post_buyer(&cookie_manager, &form),
        });
        job_id
    }

    pub fn spawn_avatar(&self, ctx: egui::Context, uid: i64, cookie_manager: Arc<CookieManager>, url: String) {
        self.spawn(move || JobResult::Avatar {
            uid,
            texture: load_texture_from_url(&ctx, cookie_manager, &url, &format!("avatar_{uid}")),
        });
    }
}

/// 收取后台结果并写回界面状态
pub fn poll(app: &mut Myapp) {
    let results: Vec<JobResult> = app.jobs.rx.try_iter().collect();
    for result in results {
        match result {
            JobResult::Policy { policy, connected } => app.apply_policy(policy, connected),
            JobResult::LoginQr { job_id, result } => {
                let Dialog::Login(st) = &mut app.dialog else { continue };
                if st.qr_job != Some(job_id) {
                    continue;
                }
                st.qr_job = None;
                match result {
                    Ok((key, url)) => {
                        let request = TaskRequest::QrCodeLoginRequest(QrCodeLoginRequest {
                            qrcode_key: key,
                            qrcode_url: url.clone(),
                            user_agent: Some(app.custom_config.custom_ua.clone()),
                        });
                        match app.task_manager.submit_task(request) {
                            Ok(task_id) => {
                                st.qr_task = Some(task_id);
                                st.qr = QrState::Ready { url };
                                log::info!("开始轮询二维码登录状态...");
                            }
                            Err(e) => {
                                log::error!("提交二维码轮询任务失败: {}", e);
                                st.qr = QrState::Failed(e);
                            }
                        }
                    }
                    Err(e) => {
                        log::error!("获取二维码失败，原因: {}", e);
                        st.qr = QrState::Failed(e);
                    }
                }
            }
            JobResult::AddAccount { job_id, result } => {
                let Dialog::Login(st) = &mut app.dialog else { continue };
                if st.account_job != Some(job_id) {
                    continue;
                }
                st.account_job = None;
                let account = match result {
                    Ok(account) => account,
                    Err(e) => {
                        log::error!("登录成功，但添加账号失败: {}", e);
                        st.busy = false;
                        st.error = Some(format!("登录失败：{e}"));
                        continue;
                    }
                };
                let uid = account.uid;
                let name = account.name.clone();
                let mut account = account;
                account.ensure_client();
                app.account_manager.accounts.push(account.clone());
                match save_config(&mut app.config, None, None, Some(account)) {
                    Ok(_) => log::info!("登录成功，账号已添加"),
                    Err(e) => log::error!("登录成功，但保存账号失败: {}", e),
                }
                if matches!(app.dialog, Dialog::Login(_)) {
                    app.close_dialog();
                }
                app.toasts.success(format!("已登录 {name}"));
                app.accounts_selected = Some(uid);
                app.draft.account_uid = Some(uid);
            }
            JobResult::SignOut { uid, name, result } => {
                match result {
                    Ok(_) => {
                        app.account_manager.accounts.retain(|a| a.uid != uid);
                        app.config.delete_account(uid);
                        log::info!("账号 {} 已删除", uid);
                        app.buyers.remove(&uid);
                        app.orders.remove(&uid);
                        if app.accounts_selected == Some(uid) {
                            app.accounts_selected = app.account_manager.accounts.first().map(|a| a.uid);
                        }
                        if app.draft.account_uid == Some(uid) {
                            app.draft.account_uid = app.account_manager.accounts.first().map(|a| a.uid);
                            app.draft.selected_buyers.clear();
                        }
                        app.toasts.success(format!("已退出 {name}"));
                    }
                    Err(e) => app.toasts.push(Tone::Danger, format!("退出失败：{e}"), None, None),
                }
                if matches!(app.dialog, Dialog::ConfirmSignOut { uid: d, .. } if d == uid) {
                    app.dialog = Dialog::None;
                }
            }
            JobResult::AddBuyer { job_id, uid, name, result } => match result {
                Ok(()) => {
                    if matches!(&app.dialog, Dialog::AddBuyer(st) if st.job_id == Some(job_id)) {
                        app.dialog = Dialog::None;
                    }
                    app.toasts.success(format!("已添加购票人 {name}"));
                    if app.draft.account_uid == Some(uid) {
                        app.draft.select_after_refresh = Some(name);
                    }
                    app.request_buyers(uid);
                }
                Err(e) => {
                    if let Dialog::AddBuyer(st) = &mut app.dialog {
                        if st.job_id == Some(job_id) {
                            st.job_id = None;
                            st.busy = false;
                            st.server_error = Some(e);
                        }
                    }
                }
            },
            JobResult::Avatar { uid, texture } => {
                if let Some(texture) = texture {
                    if let Some(account) = app.account_manager.accounts.iter_mut().find(|a| a.uid == uid) {
                        account.avatar_texture = Some(texture);
                    }
                }
            }
        }
    }
}

/// 与旧版添加购票人窗口相同的请求；网络和接口错误返回 Err 文案
fn post_buyer(cookie_manager: &CookieManager, form: &BuyerForm) -> Result<(), String> {
    let mut json_form = serde_json::Map::new();
    json_form.insert("name".to_string(), Value::String(form.name.clone()));
    json_form.insert("tel".to_string(), Value::String(form.tel.clone()));
    json_form.insert("id_type".to_string(), Value::Number(serde_json::Number::from(form.id_type)));
    json_form.insert("personal_id".to_string(), Value::String(form.personal_id.clone()));
    json_form.insert("is_default".to_string(), Value::String(if form.is_default { "1" } else { "0" }.to_string()));
    json_form.insert("src".to_string(), Value::String("ticket".to_string()));
    log::debug!("添加购票人数据: {:?}", json_form);

    let rt = tokio::runtime::Runtime::new().map_err(|e| format!("创建运行时失败：{e}"))?;
    rt.block_on(async {
        let response = cookie_manager
            .post("https://show.bilibili.com/api/ticket/buyer/create")
            .await
            .json(&json_form)
            .send()
            .await
            .map_err(|e| format!("网络请求失败：{e}"))?;
        if !response.status().is_success() {
            log::error!("添加购票人失败: {:?}", response.status());
            return Err(format!("服务器返回 {}", response.status()));
        }
        let text = response.text().await.map_err(|e| {
            log::error!("获取响应文本失败: {}", e);
            format!("读取响应失败：{e}")
        })?;
        let value: Value = serde_json::from_str(&text).map_err(|e| {
            log::error!("解析JSON失败! 原因: {}, 响应原文: {}", e, text);
            "响应格式无法解析".to_string()
        })?;
        let errno = value.get("errno").and_then(|v| v.as_i64()).unwrap_or(-1);
        let code = if errno != -1 { errno } else { value.get("code").and_then(|v| v.as_i64()).unwrap_or(-1) };
        if code == 0 {
            log::info!("添加购票人成功: {:?}", text);
            Ok(())
        } else {
            log::error!("添加购票人失败: {:?}", text);
            let msg = ["msg", "message"]
                .iter()
                .filter_map(|k| value.get(*k).and_then(|v| v.as_str()))
                .find(|s| !s.is_empty())
                .unwrap_or("未知错误");
            Err(format!("{msg}（{code}）"))
        }
    })
}

#[derive(Debug, Serialize, Deserialize)]
struct PolicyPayload {
    policy: Value,
}

#[derive(Debug, Serialize, Deserialize)]
struct PermissionsPayload {
    permissions: Value,
}

/// 原 Myapp::get_policy。返回 (策略, 是否成功连上公告服务)；兜底的 allow_run: true 视为未连接
async fn get_policy(client: &Client, app: &str, version: &str, machine_id: &str, public_key: &str) -> (Value, bool) {
    let timestamp = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs();
    let data = json!({
        "ts": timestamp,
        "machine_id": machine_id,
    });
    let url = format!("https://policy.nexaorion.cn/api/client/{}/{}/dispatch.json", app, version);

    match client.post(&url).json(&data).send().await {
        Ok(response) => match response.json::<Value>().await {
            Ok(resp) => {
                if let Some(code) = resp["code"].as_i64() {
                    if code != 0 {
                        log::error!("获取策略失败: {}", resp["message"]);
                        return (json!({"allow_run": true}), false);
                    }
                    match decode_policy(resp["data"]["data"].as_str().unwrap_or(""), public_key) {
                        Ok(policy) => {
                            // 旧逻辑中权限值随后会被策略覆盖，这里只保留写入或校验权限文件的行为
                            if let Some(permission_token) = resp["data"]["permission"].as_str() {
                                match decode_permissions(permission_token, public_key) {
                                    Ok(_) => {
                                        if let Ok(mut file) = File::create("permissions") {
                                            let _ = file.write_all(permission_token.as_bytes());
                                        }
                                    }
                                    Err(e) => {
                                        log::error!("权限签名无效: {}", e);
                                        let _ = load_local_permissions(public_key);
                                    }
                                }
                            }
                            return (policy, true);
                        }
                        Err(e) => {
                            log::error!("策略签名无效: {}", e);
                            return (json!({"allow_run": false}), false);
                        }
                    }
                }
            }
            Err(e) => log::error!("解析响应失败: {}", e),
        },
        Err(e) => log::error!("请求策略失败: {}", e),
    }
    (json!({"allow_run": true}), false)
}

fn decode_policy(token: &str, public_key: &str) -> Result<Value, String> {
    let decoding_key = DecodingKey::from_rsa_pem(public_key.as_bytes()).map_err(|e| format!("无效的公钥: {}", e))?;
    let mut validation = Validation::new(Algorithm::RS256);
    validation.validate_exp = false;
    validation.required_spec_claims.clear();
    decode::<PolicyPayload>(token, &decoding_key, &validation)
        .map(|data| data.claims.policy)
        .map_err(|e| format!("解码JWT失败: {}", e))
}

fn decode_permissions(token: &str, public_key: &str) -> Result<Value, String> {
    let decoding_key = DecodingKey::from_rsa_pem(public_key.as_bytes()).map_err(|e| format!("无效的公钥: {}", e))?;
    let mut validation = Validation::new(Algorithm::RS256);
    validation.validate_exp = false;
    validation.required_spec_claims.clear();
    decode::<PermissionsPayload>(token, &decoding_key, &validation)
        .map(|data| data.claims.permissions)
        .map_err(|e| format!("解码JWT失败: {}", e))
}

fn load_local_permissions(public_key: &str) -> Value {
    if let Ok(mut file) = File::open("permissions") {
        let mut contents = String::new();
        if file.read_to_string(&mut contents).is_ok() {
            if let Ok(decoded) = decode_permissions(&contents, public_key) {
                return decoded;
            }
        }
    }
    json!({})
}
