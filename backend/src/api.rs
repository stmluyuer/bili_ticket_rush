use serde::Deserialize;
use common::cookie_manager::CookieManager;
use common::http_utils::request_get;
use common::ticket::{*};
use common::gen_cp::CTokenGenerator;
use serde_json;
use common::login::QrCodeLoginStatus;
use reqwest::{Client, Request, RequestBuilder, Response, StatusCode};
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::Mutex;
use serde_json::{json, Value};
use rand::{thread_rng, Rng};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

fn cache_path(project_id: &str) -> std::path::PathBuf {
    let dir = std::path::Path::new("cache");
    dir.join(format!("project_{}.json", project_id))
}

fn save_project_cache(project_id: &str, info: &InfoResponse) {
    let path = cache_path(project_id);
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    match serde_json::to_string_pretty(info) {
        Ok(json) => {
            if let Err(e) = std::fs::write(&path, json) {
                log::warn!("保存项目缓存失败: {}", e);
            } else {
                log::info!("已缓存项目信息到 {:?}", path);
            }
        }
        Err(e) => log::warn!("序列化项目缓存失败: {}", e),
    }
}

fn load_project_cache(project_id: &str) -> Option<InfoResponse> {
    let path = cache_path(project_id);
    match std::fs::read_to_string(&path) {
        Ok(json) => {
            match serde_json::from_str::<InfoResponse>(&json) {
                Ok(info) => {
                    log::info!("已从本地缓存加载项目信息: {:?}", path);
                    Some(info)
                }
                Err(e) => {
                    log::warn!("解析本地缓存失败: {}", e);
                    None
                }
            }
        }
        Err(_) => None,
    }
}


pub async fn get_countdown(
    cookie_manager: Arc<CookieManager>,
    info: Option<TicketInfo>,
) -> Result<f64, String> {
    let sale_begin = info
        .ok_or("获取开始时间失败")?
        .sale_begin
        .ok_or("开售时间(sale_begin)为空")?;
    let request = cookie_manager.get("https://api.bilibili.com/x/click-interface/click/now").await;
    let server_time = match send_purchase(request).await {
        Ok((value, retry_after)) => purchase_data(&value, retry_after)
            .ok()
            .and_then(|data| data["now"].as_i64())
            .filter(|time| *time > 0),
        Err(error) => {
            log::debug!("获取网络时间失败：{error}");
            None
        }
    };
    let now = server_time.unwrap_or_else(|| {
        log::debug!("使用本地时间计算开售倒计时");
        chrono::Utc::now().timestamp()
    });
    Ok((sale_begin - now) as f64)
}

pub async fn get_buyer_info(cookie_manager: Arc<CookieManager>) -> Result<BuyerInfoResponse,String>{
    let req = cookie_manager.get("https://show.bilibili.com/api/ticket/buyer/list").await;
    let response = req.send().await;
    match response {
        Ok(resp)=>{
            if resp.status().is_success(){
                match tokio::task::block_in_place(||{
                    let rt = tokio::runtime::Runtime::new().unwrap();
                    rt.block_on(resp.text())
                }){
                    Ok(text) => {
                        log::debug!("获取购票人信息：{}",text);
                        match serde_json::from_str::<BuyerInfoResponse>(&text){
                            Ok(buyer_info) => {
                                return Ok(buyer_info);
                            }
                            Err(e) => {
                                log::error!("获取购票人信息json解析失败：{}",e);
                                return Err(format!("获取购票人信息json解析失败：{}",e))
                            }

                        }
                    }
                    Err(e) => {
                        log::error!("获取购票人信息失败：{}",e);
                        return Err(format!("获取购票人信息失败：{}",e))
                    }

                }
            }
            else{
                
                log::debug!("请求响应失败: {:?}", resp);
                return Err(format!("请求响应失败: {}", resp.status()));
            }
        }
        Err(e) => {
            Err(format!("请求失败: {}", e))
        }
    }
}

pub async fn get_project(cookie_manager: Arc<CookieManager>, project_id : &str, referer_link: &str) -> Result<InfoResponse,String>{
    let url = "https://show.bilibili.com/api/ticket/project/getV2";
    let request = cookie_manager
        .get_with_headers(url, HashMap::from([("Referer", referer_link)]))
        .await
        .query(&[("id", project_id), ("project_id", project_id)]);
    let result = async {
        let (value, _) = send_purchase(request).await.map_err(|error| error.to_string())?;
        let mut info = InfoResponse::deserialize(value)
            .map_err(|error| format!("解析项目详情失败：{error}"))?;
        info.data.fill_missing_sale_begin();
        info.data.validate()?;
        Ok(info)
    }.await;

    #[cfg(test)]
    if purchase_test::is_active() {
        return result;
    }

    // 成功时缓存到本地，失败时尝试从缓存加载
    match result {
        Ok(info) => {
            save_project_cache(project_id, &info);
            Ok(info)
        }
        Err(e) => {
            log::warn!("在线获取项目信息失败({}), 尝试从本地缓存加载...", e);
            match load_project_cache(project_id) {
                Some(cached) => {
                    log::info!("已降级使用本地缓存的项目信息");
                    Ok(cached)
                }
                None => Err(e),
            }
        }
    }
}


//轮询登录状态
pub async fn poll_qrcode_login(qrcode_key: &str,user_agent: Option<&str>) ->QrCodeLoginStatus {
    
    
    let client_builder = Client::builder();
    let client = if let Some(ua) = user_agent {
        client_builder.user_agent(ua)
    } else {
        client_builder.user_agent("Mozilla/5.0 (Windows NT 10.0; Win64; x64) Chrome/110.0.0.0 Safari/537.36")
    }.build()
    .unwrap_or_default();
    
    let max_attempts = 60;
    
    for attempt in 1..max_attempts{

    
    //轮询
    let response = match request_get(
        &client,
        &format!("https://passport.bilibili.com/x/passport-login/web/qrcode/poll?qrcode_key={}", qrcode_key),
       
        None,
    ).await {
        Ok(resp) => resp,
        Err(e) => return QrCodeLoginStatus::Failed(e.to_string()),
    };

    let mut all_cookies = Vec::new();
    let cookie_headers = response.headers().get_all(reqwest::header::SET_COOKIE);
    for value in cookie_headers {
     if let Ok(cookie_str) = value.to_str() {
        
        if let Some(end_pos) = cookie_str.find(';') {
            all_cookies.push(cookie_str[0..end_pos].to_string());
        } else {
            all_cookies.push(cookie_str.to_string());
        }
    }
    }
    
    let json = match response.json::<serde_json::Value>().await {
        Ok(j) => j,
        Err(e) => return QrCodeLoginStatus::Failed(e.to_string()),
    };
    
    
    let code = json["data"]["code"]
        .as_i64()
        .or_else(|| json["code"].as_i64().filter(|&c| c != 0))
        .unwrap_or(-1);
    match code {
        0 => {
            //json获取cookie
            
            if let Some(cookie_info) = json["data"]["cookie_info"].as_object() {
                for (key, value) in cookie_info {
                    if let Some(val_str) = value["value"].as_str() {
                        all_cookies.push(format!("{}={}", key, val_str));
                    }
                }
            }
            
            
            if !all_cookies.is_empty() {
                return QrCodeLoginStatus::Success(all_cookies.join("; "));
            } else {
                return QrCodeLoginStatus::Failed("无法获取Cookie信息".to_string());
            }
        },
        86038 => return QrCodeLoginStatus::Expired,
        86090 => {
            log::info!("二维码已扫描，等待确认 (尝试 {} / {} 次)", attempt, max_attempts);
            //return QrCodeLoginStatus::Scanning;
        },
        86101 => {
            log::info!("二维码已生成，等待扫描 (尝试 {} / {} 次)", attempt, max_attempts);
            //return QrCodeLoginStatus::Pending
        },
        _ => {
            let message = json["data"]["message"]
                .as_str()
                .or_else(|| json["message"].as_str())
                .unwrap_or("未知错误");

            return QrCodeLoginStatus::Failed(message.to_string())
        }
    }
    tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
}
QrCodeLoginStatus::Expired
}


const PURCHASE_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Debug, PartialEq, Eq)]
pub struct PurchaseTarget {
    pub project_id: i64,
    pub screen_id: i64,
    pub sku_id: i64,
    pub count: i32,
}

impl PurchaseTarget {
    pub fn new(
        project_id: &str,
        screen_id: &str,
        sku_id: &str,
        count: i16,
    ) -> Result<Self, PurchaseApiError> {
        if count <= 0 {
            return Err(PurchaseApiError::InvalidParameter("购票数量必须大于 0".into()));
        }

        Ok(Self {
            project_id: parse_target_id(project_id, "项目")?,
            screen_id: parse_target_id(screen_id, "场次")?,
            sku_id: parse_target_id(sku_id, "票档")?,
            count: i32::from(count),
        })
    }
}

#[derive(Debug)]
pub enum PurchaseApiError {
    Business {
        code: i64,
        message: String,
        retry_after: Option<Duration>,
    },
    Http {
        status: StatusCode,
        retry_after: Option<Duration>,
    },
    Transport(reqwest::Error),
    ReadBody(reqwest::Error),
    Parse(String),
    InvalidParameter(String),
    Challenge(TokenRiskParam),
    Cancelled,
}

impl PurchaseApiError {
    pub fn retry_after(&self) -> Option<Duration> {
        match self {
            Self::Business { retry_after, .. } | Self::Http { retry_after, .. } => *retry_after,
            _ => None,
        }
    }
}

impl std::fmt::Display for PurchaseApiError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Business { code, message, .. } => write!(f, "业务码 {code}，{message}"),
            Self::Http { status, .. } => write!(f, "HTTP {status}"),
            Self::Transport(error) => write!(f, "请求失败：{}", error),
            Self::ReadBody(error) => write!(f, "读取响应失败：{}", error),
            Self::Parse(message) => write!(f, "响应解析失败：{message}"),
            Self::InvalidParameter(message) => write!(f, "购票参数有误：{message}"),
            Self::Challenge(risk) => write!(f, "需要人机验证（{}）：{}", risk.code, risk.message),
            Self::Cancelled => f.write_str("发送前已停止"),
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub struct CreatedOrder {
    pub order_id: Option<String>,
    pub query_token: Option<String>,
}

#[derive(Debug)]
pub struct CheckOrderResult {
    pub order_id: Option<String>,
    pub pay_result: Option<CheckFakeResultData>,
    pub retry_after: Option<Duration>,
}

fn parse_target_id(value: &str, name: &str) -> Result<i64, PurchaseApiError> {
    value
        .parse::<i64>()
        .ok()
        .filter(|id| *id > 0)
        .ok_or_else(|| PurchaseApiError::InvalidParameter(format!("{name} ID 无效")))
}

fn nonempty_string(value: &Value) -> Option<&str> {
    value.as_str().filter(|text| !text.trim().is_empty())
}

fn order_id(value: &Value) -> Option<String> {
    match value {
        Value::Number(number) => number.as_u64().filter(|id| *id > 0).map(|id| id.to_string()),
        Value::String(text) => text.parse::<u128>().ok().filter(|id| *id > 0).map(|id| id.to_string()),
        _ => None,
    }
}

fn retry_after_delay(value: &str, now: chrono::DateTime<chrono::Utc>) -> Option<Duration> {
    value
        .trim()
        .parse::<u64>()
        .ok()
        .map(Duration::from_secs)
        .or_else(|| {
            chrono::DateTime::parse_from_rfc2822(value)
                .ok()
                .map(|deadline| deadline.signed_duration_since(now).to_std().unwrap_or_default())
        })
}

fn response_status(
    status: StatusCode,
    retry_after: Option<Duration>,
    exact_ok: bool,
) -> Result<(), PurchaseApiError> {
    if (exact_ok && status != StatusCode::OK) || (!exact_ok && !status.is_success()) {
        return Err(PurchaseApiError::Http { status, retry_after });
    }
    Ok(())
}

fn decode_purchase_body(body: &[u8]) -> Result<Value, PurchaseApiError> {
    serde_json::from_slice(body).map_err(|error| PurchaseApiError::Parse(error.to_string()))
}

async fn purchase_response(
    response: Response,
    exact_ok: bool,
) -> Result<(Value, Option<Duration>), PurchaseApiError> {
    let retry_after = response
        .headers()
        .get(reqwest::header::RETRY_AFTER)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| retry_after_delay(value, chrono::Utc::now()));
    response_status(response.status(), retry_after, exact_ok)?;
    let body = response.bytes().await.map_err(|error| PurchaseApiError::ReadBody(error.without_url()))?;
    Ok((decode_purchase_body(&body)?, retry_after))
}

async fn execute_purchase(
    client: Client,
    request: Request,
    exact_ok: bool,
) -> Result<(Value, Option<Duration>), PurchaseApiError> {
    #[cfg(test)]
    if let Some(reply) = purchase_test::take_response(&request) {
        return purchase_test::resolve_reply(reply, exact_ok).await;
    }

    let response = client
        .execute(request)
        .await
        .map_err(|error| PurchaseApiError::Transport(error.without_url()))?;
    purchase_response(response, exact_ok).await
}

async fn send_purchase(request: RequestBuilder) -> Result<(Value, Option<Duration>), PurchaseApiError> {
    let (client, request) = request.timeout(PURCHASE_TIMEOUT).build_split();
    let request = request.map_err(|error| {
        PurchaseApiError::InvalidParameter(format!("无法构造购票请求：{}", error.without_url()))
    })?;
    execute_purchase(client, request, false).await
}

fn purchase_data(value: &Value, retry_after: Option<Duration>) -> Result<&Value, PurchaseApiError> {
    let errno = value["errno"].as_i64();
    let code = value["code"].as_i64();
    match (errno, code) {
        (Some(errno), Some(code)) if errno != code => {
            Err(PurchaseApiError::Parse(format!("业务码不一致（errno {errno}，code {code}）")))
        }
        (Some(0), _) | (_, Some(0)) => Ok(&value["data"]),
        (Some(code), _) | (_, Some(code)) => Err(PurchaseApiError::Business {
            code,
            message: nonempty_string(&value["msg"])
                .or_else(|| nonempty_string(&value["message"]))
                .unwrap_or("服务端未返回说明")
                .to_owned(),
            retry_after,
        }),
        (None, None) => Err(PurchaseApiError::Parse("缺少有效业务码".into())),
    }
}

fn decode_prepare(
    value: &Value,
    retry_after: Option<Duration>,
    is_hot: bool,
) -> Result<(String, String), PurchaseApiError> {
    let data = match purchase_data(value, retry_after) {
        Ok(data) => data,
        Err(PurchaseApiError::Business { code, message, .. }) if matches!(code, -401 | 401) => {
            let risk = &value["data"]["ga_data"]["riskParams"];
            let field = |key: &str| nonempty_string(&risk[key]).map(str::to_owned);
            return Err(PurchaseApiError::Challenge(TokenRiskParam {
                code: code as i32,
                message,
                mid: field("mid"),
                decision_type: field("decision_type"),
                buvid: field("buvid"),
                ip: field("ip"),
                scene: field("scene"),
                ua: field("ua"),
                v_voucher: field("v_voucher"),
                risk_param: risk.as_object().map(|_| risk.clone()),
            }));
        }
        Err(error) => return Err(error),
    };
    let token = nonempty_string(&data["token"])
        .ok_or_else(|| PurchaseApiError::Parse("准备响应缺少购票 token".into()))?;
    // 热门项目也可能返回 null ptoken，沿用接口的空值语义。
    let ptoken = if is_hot {
        data["ptoken"].as_str().unwrap_or_default().to_owned()
    } else {
        String::new()
    };
    Ok((token.to_owned(), ptoken))
}

pub async fn get_ticket_token(
    cookie_manager: Arc<CookieManager>,
    cpdd: Arc<Mutex<CTokenGenerator>>,
    target: &PurchaseTarget,
    is_hot: bool,
) -> Result<(String, String), PurchaseApiError> {
    let token = if is_hot {
        cpdd.lock().unwrap().generate_ctoken(false)
    } else {
        String::new()
    };
    let params = json!({
        "count": target.count,
        "screen_id": target.screen_id,
        "order_type": 1,
        "project_id": target.project_id,
        "sku_id": target.sku_id,
        "token": token,
        "newRisk": true,
        "requestSource": "neul-next",
    });
    let url = "https://show.bilibili.com/api/ticket/order/prepare";
    let request = if is_hot {
        // 热门项目与 createV2 复用同一个 HTTP/2 客户端。
        cookie_manager.post_with_headers_h2(url, HashMap::new()).await
    } else {
        cookie_manager.post(url).await
    };
    let (value, retry_after) = send_purchase(request.query(&[("project_id", target.project_id)]).json(&params)).await?;
    decode_prepare(&value, retry_after, is_hot)
}

fn confirm_target_id(
    data: &Value,
    keys: [&str; 2],
    expected: i64,
    name: &str,
) -> Result<(), PurchaseApiError> {
    for field in keys {
        if let Some(value) = data.get(field).filter(|value| !value.is_null()) {
            let id = value.as_i64().or_else(|| value.as_str()?.parse().ok())
                .ok_or_else(|| PurchaseApiError::Parse(format!("确认响应的 {field} 无效")))?;
            if id != expected {
                return Err(PurchaseApiError::InvalidParameter(format!(
                    "确认响应的{name} ID {id} 与所选 ID {expected} 不符"
                )));
            }
        }
    }
    Ok(())
}

fn decode_confirmation(
    value: &Value,
    retry_after: Option<Duration>,
    target: &PurchaseTarget,
) -> Result<ConfirmTicketResult, PurchaseApiError> {
    let data = purchase_data(value, retry_after)?;
    let confirm = ConfirmTicketResult::deserialize(data)
        .map_err(|error| PurchaseApiError::Parse(format!("确认参数：{error}")))?;
    for fields in [data, &data["ticket_info"]] {
        confirm_target_id(fields, ["project_id", "projectId"], target.project_id, "项目")?;
        confirm_target_id(fields, ["screen_id", "screenId"], target.screen_id, "场次")?;
        confirm_target_id(fields, ["sku_id", "skuId"], target.sku_id, "票档")?;
    }
    if confirm.count != target.count || confirm.ticket_info.count != target.count {
        return Err(PurchaseApiError::InvalidParameter(format!(
            "确认数量为 {}（票档数量 {}），与所选数量 {} 不符",
            confirm.count, confirm.ticket_info.count, target.count
        )));
    }
    if confirm.pay_money < 0 {
        return Err(PurchaseApiError::InvalidParameter("确认金额不能为负数".into()));
    }
    Ok(confirm)
}

pub async fn confirm_ticket_order(
    cookie_manager: Arc<CookieManager>,
    target: &PurchaseTarget,
    token: &str,
) -> Result<ConfirmTicketResult, PurchaseApiError> {
    let project_id = target.project_id.to_string();
    let request = cookie_manager.get("https://show.bilibili.com/api/ticket/order/confirmInfo").await.query(&[
        ("token", token),
        ("voucher", ""),
        ("project_id", project_id.as_str()),
        ("requestSource", "neul-next"),
    ]);
    let (value, retry_after) = send_purchase(request).await?;
    decode_confirmation(&value, retry_after, target)
}

fn decode_created_order(
    value: &Value,
    retry_after: Option<Duration>,
) -> Result<CreatedOrder, PurchaseApiError> {
    let data = purchase_data(value, retry_after)?;
    Ok(CreatedOrder {
        order_id: order_id(&data["orderId"]),
        query_token: nonempty_string(&data["token"]).map(str::to_owned),
    })
}

pub async fn create_order(
    cookie_manager: Arc<CookieManager>,
    cpdd: Arc<Mutex<CTokenGenerator>>,
    target: &PurchaseTarget,
    token: &str,
    ptoken: &str,
    confirm_result: &ConfirmTicketResult,
    is_hot: bool,
    biliticket: &BilibiliTicket,
    buyer_info: &[BuyerInfo],
    is_mobile: bool,
    need_retry: bool,
    fast_mode: bool,
    screen_size: Option<(u32, u32)>,
    before_send: impl FnOnce() -> bool,
) -> Result<CreatedOrder, PurchaseApiError> {
    let position_type = if need_retry && is_mobile {
        ClickPositionType::RetryButton
    } else if is_mobile {
        ClickPositionType::MobileConfirm
    } else {
        ClickPositionType::PcConfirm
    };
    let (width, height) = screen_size.unwrap_or((1080, 2400));
    let click_position = random_click_position(position_type, fast_mode, Some(width), Some(height)).await;
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64;
    let mut data = json!({
        "project_id": target.project_id,
        "screen_id": target.screen_id,
        "sku_id": target.sku_id,
        "token": token,
        "clickPosition": click_position,
        "newRisk": true,
        "requestSource": if is_mobile { "neul-next" } else { "pc-new" },
        "deviceId": cookie_manager.get_cookie("deviceFingerprint"),
        "pay_money": confirm_result.pay_money,
        "count": target.count,
        "timestamp": timestamp,
        "order_type": 1,
    });

    let ptoken = ptoken.replace('=', "");
    match biliticket.id_bind {
        0 => {
            let buyer = biliticket.no_bind_buyer_info.as_ref().ok_or_else(|| {
                PurchaseApiError::InvalidParameter("缺少非实名购票联系人".into())
            })?;
            if buyer.name.trim().is_empty() || buyer.tel.trim().is_empty() {
                return Err(PurchaseApiError::InvalidParameter("购票联系人姓名或电话为空".into()));
            }
            data["buyer"] = json!(buyer.name);
            data["tel"] = json!(buyer.tel);
        }
        1 | 2 => {
            if buyer_info.is_empty() {
                return Err(PurchaseApiError::InvalidParameter("缺少实名购票人".into()));
            }
            data["buyer_info"] = json!(serde_json::to_string(buyer_info).map_err(|error| {
                PurchaseApiError::InvalidParameter(format!("无法编码购票人信息：{error}"))
            })?);
            if is_hot {
                data["ctoken"] = json!(cpdd.lock().unwrap().generate_ctoken(true));
                data["ptoken"] = json!(ptoken);
            }
        }
        _ => return Err(PurchaseApiError::InvalidParameter("未知购票人绑定类型".into())),
    }

    let risk_header = format!(
        "platform/h5 uid/{} deviceId/{}",
        cookie_manager.get_cookie("DedeUserID").unwrap_or_default(),
        cookie_manager.get_cookie("buvid3").unwrap_or_default()
    );
    let url = "https://show.bilibili.com/api/ticket/order/createV2";
    let mut request = cookie_manager
        .post_with_headers_h2(url, HashMap::from([("X-Risk-Header", risk_header.as_str())]))
        .await
        .query(&[("project_id", target.project_id)])
        .timeout(PURCHASE_TIMEOUT)
        .json(&data);
    if is_hot {
        request = request.query(&[("ptoken", ptoken.as_str())]);
    }
    let (client, request) = request.build_split();
    let request = request.map_err(|error| {
        PurchaseApiError::InvalidParameter(format!("无法构造创建请求：{}", error.without_url()))
    })?;
    // 调用方在同一处检查取消、计数，并记录结果未知的订单上下文。
    if !before_send() {
        return Err(PurchaseApiError::Cancelled);
    }
    let (value, retry_after) = execute_purchase(client, request, true).await?;
    decode_created_order(&value, retry_after)
}

fn decode_payment(
    value: &Value,
    retry_after: Option<Duration>,
    expected_order_id: Option<&str>,
) -> Result<CheckOrderResult, PurchaseApiError> {
    let data = purchase_data(value, retry_after)?;
    let order_id = order_id(&data["orderId"]);
    if let (Some(actual), Some(expected)) = (&order_id, expected_order_id) {
        if actual != expected {
            return Err(PurchaseApiError::Parse("查单响应的订单 ID 与当前订单不符".into()));
        }
    }
    let pay_param = &data["payParam"];
    let pay_result = if pay_param.is_null()
        || (pay_param.is_object() && (pay_param["code_url"].is_null() || pay_param["sign"].is_null()))
    {
        None
    } else {
        let pay_result = CheckFakeResultData::deserialize(pay_param)
            .map_err(|error| PurchaseApiError::Parse(format!("支付信息：{error}")))?;
        (!pay_result.code_url.trim().is_empty()).then_some(pay_result)
    };
    Ok(CheckOrderResult { order_id, pay_result, retry_after })
}

pub async fn check_fake_ticket(
    cookie_manager: Arc<CookieManager>,
    project_id: &str,
    pay_token: &str,
    order_id: Option<&str>,
) -> Result<CheckOrderResult, PurchaseApiError> {
    let timestamp = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs();
    let mut request = cookie_manager.get("https://show.bilibili.com/api/ticket/order/createstatus").await.query(&[
        ("project_id", project_id),
        ("token", pay_token),
    ]).query(&[("timestamp", timestamp)]);
    if let Some(order_id) = order_id {
        request = request.query(&[("orderId", order_id)]);
    }
    let (value, retry_after) = send_purchase(request).await?;
    decode_payment(&value, retry_after, order_id)
}

/// 点击位置类型枚举
#[derive(Debug, Clone, Copy)]
pub enum ClickPositionType {
    /// PC端确认下单按钮位置
    PcConfirm,
    /// 手机端确认下单按钮位置
    MobileConfirm,
    /// "再试一次"按钮位置（屏幕中间）
    RetryButton,
}


/// 生成随机点击位置
/// 
/// # 参数
/// * `position_type` - 位置类型：PC/手机/重试按钮
/// * `fast_mode` - 时间间隔模式：true为快模式(0.8-4.6秒)，false为慢模式(4-12秒)
/// * `screen_width` - (可选)手机屏幕宽度，用于计算比例坐标，默认1080
/// * `screen_height` - (可选)手机屏幕高度，用于计算比例坐标，默认2400
/// 
pub async fn random_click_position(
    position_type: ClickPositionType, 
    fast_mode: bool,
    screen_width: Option<u32>,
    screen_height: Option<u32>
) -> Value {
    let mut rng = thread_rng();
    
    // 获取手机屏幕尺寸（默认使用常见尺寸1080x2400）
    let mobile_width = screen_width.unwrap_or(1080);
    let mobile_height = screen_height.unwrap_or(2400);
    
    // 根据不同设备/按钮类型确定基准坐标和偏移范围
    let (base_x, base_y, offset_range) = match position_type {
        ClickPositionType::PcConfirm => {
            // PC端确认下单按钮位置(右侧中下部)
            (1131, 636, 10)
        },
        ClickPositionType::MobileConfirm => {
            // 手机端确认下单按钮位置(右下角)
            // 使用比例计算：x在屏幕宽度的0.55-0.9之间，y在屏幕底部附近
            let x_ratio = rng.gen_range(0.55..0.9);
            let y_ratio = rng.gen_range(0.9..0.95);
            
            let x = (mobile_width as f32 * x_ratio) as i32;
            let y = (mobile_height as f32 * y_ratio) as i32;
            
            (x, y, mobile_width.min(20) as i32 / 4) // 偏移范围根据屏幕宽度按比例缩放
        },
        ClickPositionType::RetryButton => {
            // 手机版"再试一次"按钮位置(屏幕中间靠下)
            // x坐标在屏幕宽度的1/3到2/3之间，y坐标在屏幕高度的2/3左右
            let x_ratio = rng.gen_range(0.33..0.67); // 屏幕宽度的2/6到4/6之间
            let y_ratio = rng.gen_range(0.6..0.7);   // 屏幕高度的2/3左右
            
            let x = (mobile_width as f32 * x_ratio) as i32;
            let y = (mobile_height as f32 * y_ratio) as i32;
            
            (x, y, mobile_width.min(30) as i32 / 4) // 偏移较大，因为这是个大按钮
        }
    };
    
    // 生成随机偏移
    let offset_x = rng.gen_range(-offset_range..=offset_range);
    let offset_y = rng.gen_range(-offset_range..=offset_range);
    
    // 计算最终坐标
    let final_x = base_x + offset_x;
    let final_y = base_y + offset_y;
    
    // 获取当前时间戳
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64;
    
    // 根据模式生成不同的延迟时间
    let random_delay = if fast_mode {
        // 快模式：0.8-4.6秒
        rng.gen_range(800..4600)
    } else {
        // 慢模式：4-12秒
        rng.gen_range(4000..12000)
    };
    
    // 计算起始时间
    let origin = now - random_delay;
    
    // 构建JSON对象
    json!({
        "x": final_x,
        "y": final_y,
        "origin": origin,
        "now": now
    })
}


fn extract_qrcode_poll_code(json: &Value) -> i64 {
    json["data"]["code"]
        .as_i64()
        .or_else(|| json["code"].as_i64().filter(|&c| c != 0))
        .unwrap_or(-1)
}

#[cfg(test)]
pub(crate) mod purchase_test {
    use super::*;
    use std::cell::RefCell;
    use std::collections::VecDeque;
    use std::future::Future;
    use tokio::sync::Notify;

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub enum Endpoint {
        Prepare,
        Confirm,
        Create,
        Check,
        Project,
        Clock,
    }

    pub enum Reply {
        Json(Value),
        JsonWithRetryAfter(Value, Duration),
        Http(StatusCode, Option<Duration>),
        Raw(&'static [u8]),
        TransportFailure,
        ReadBodyFailure,
        Wait {
            entered: Arc<Notify>,
            release: Arc<Notify>,
            reply: Box<Reply>,
        },
    }

    #[derive(Debug)]
    pub struct RecordedRequest {
        pub endpoint: Endpoint,
        pub body: Option<Value>,
        pub query: Vec<(String, String)>,
    }

    struct State {
        responses: VecDeque<(Endpoint, Reply)>,
        requests: Vec<RecordedRequest>,
    }

    tokio::task_local! {
        static STATE: RefCell<State>;
    }

    pub async fn with_responses<T>(
        responses: impl IntoIterator<Item = (Endpoint, Reply)>,
        run: impl Future<Output = T>,
    ) -> (T, Vec<RecordedRequest>) {
        let state = RefCell::new(State {
            responses: responses.into_iter().collect(),
            requests: Vec::new(),
        });
        STATE.scope(state, async {
            let result = run.await;
            (result, STATE.with(|state| std::mem::take(&mut state.borrow_mut().requests)))
        }).await
    }

    pub(super) fn is_active() -> bool {
        STATE.try_with(|_| ()).is_ok()
    }

    pub(super) fn take_response(request: &Request) -> Option<Reply> {
        STATE.try_with(|state| {
            let endpoint = match request.url().path() {
                "/api/ticket/order/prepare" => Endpoint::Prepare,
                "/api/ticket/order/confirmInfo" => Endpoint::Confirm,
                "/api/ticket/order/createV2" => Endpoint::Create,
                "/api/ticket/order/createstatus" => Endpoint::Check,
                "/api/ticket/project/getV2" => Endpoint::Project,
                "/x/click-interface/click/now" => Endpoint::Clock,
                path => panic!("未设置购票测试接口：{path}"),
            };
            let mut state = state.borrow_mut();
            let (expected, reply) = state.responses.pop_front()
                .unwrap_or_else(|| panic!("未设置 {endpoint:?} 的测试响应"));
            assert_eq!(endpoint, expected);
            let body = request.body()
                .and_then(reqwest::Body::as_bytes)
                .map(|bytes| serde_json::from_slice(bytes).expect("请求体应为 JSON"));
            let query = request.url().query_pairs().into_owned().collect();
            state.requests.push(RecordedRequest { endpoint, body, query });
            reply
        }).ok()
    }

    fn synthetic_request_error() -> reqwest::Error {
        // 只构造离线错误值，供调用方分别覆盖发送和读体失败后的处理。
        Client::new().get("invalid-url").build().unwrap_err()
    }

    pub(super) async fn resolve_reply(
        mut reply: Reply,
        exact_ok: bool,
    ) -> Result<(Value, Option<Duration>), PurchaseApiError> {
        loop {
            match reply {
                Reply::Json(value) => return Ok((value, None)),
                Reply::JsonWithRetryAfter(value, delay) => return Ok((value, Some(delay))),
                Reply::Raw(body) => return Ok((decode_purchase_body(body)?, None)),
                Reply::TransportFailure => {
                    return Err(PurchaseApiError::Transport(synthetic_request_error()));
                }
                Reply::ReadBodyFailure => {
                    return Err(PurchaseApiError::ReadBody(synthetic_request_error()));
                }
                Reply::Http(status, retry_after) => {
                    response_status(status, retry_after, exact_ok)?;
                    return Ok((decode_purchase_body(&[])?, retry_after));
                }
                Reply::Wait { entered, release, reply: next } => {
                    entered.notify_one();
                    release.notified().await;
                    reply = *next;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn qrcode_poll_uses_nested_data_code() {
        let unscanned = json!({
            "code": 0,
            "message": "0",
            "ttl": 1,
            "data": {
                "url": "",
                "refresh_token": "",
                "timestamp": 0,
                "code": 86101,
                "message": "未扫码"
            }
        });
        assert_eq!(extract_qrcode_poll_code(&unscanned), 86101);

        let scanned = json!({
            "code": 0,
            "data": { "code": 86090, "message": "二维码已扫码未确认" }
        });
        assert_eq!(extract_qrcode_poll_code(&scanned), 86090);

        let success = json!({
            "code": 0,
            "data": { "code": 0, "message": "" }
        });
        assert_eq!(extract_qrcode_poll_code(&success), 0);

        let api_err = json!({
            "code": -400,
            "message": "请求错误"
        });
        assert_eq!(extract_qrcode_poll_code(&api_err), -400);
    }

    #[test]
    fn confirmation_matches_selected_ids_count_and_accepts_zero_price() {
        let target = PurchaseTarget::new("7", "11", "13", 2).unwrap();
        let mut response = json!({
            "errno": 0,
            "data": {
                "project_name": "活动",
                "screen_name": "场次",
                "project_id": 7,
                "screenId": "11",
                "count": 2,
                "pay_money": 0,
                "ticket_info": { "name": "票种", "count": 2, "price": 325, "sku_id": 13 }
            }
        });
        assert_eq!(decode_confirmation(&response, None, &target).unwrap().pay_money, 0);
        response["data"]["pay_money"] = json!(325);
        assert_eq!(decode_confirmation(&response, None, &target).unwrap().pay_money, 325);
        response["data"]["count"] = json!(1);
        assert!(matches!(
            decode_confirmation(&response, None, &target),
            Err(PurchaseApiError::InvalidParameter(_))
        ));
        response["data"]["count"] = json!(2);
        response["data"]["ticket_info"]["sku_id"] = json!(14);
        assert!(matches!(
            decode_confirmation(&response, None, &target),
            Err(PurchaseApiError::InvalidParameter(_))
        ));
        assert!(matches!(
            decode_confirmation(&json!({"errno": 100041, "msg": "token 失效"}), None, &target),
            Err(PurchaseApiError::Business { code: 100041, .. })
        ));
    }

    #[test]
    fn creation_distinguishes_rejection_and_unconfirmed_responses() {
        let response = json!({"errno": 100009, "msg": "库存不足"});
        let delay = Some(Duration::from_secs(4));
        let error = decode_created_order(&response, delay).unwrap_err();
        assert_eq!(error.retry_after(), delay);
        assert!(matches!(error, PurchaseApiError::Business { code: 100009, ref message, .. } if message == "库存不足"));
        for status in [StatusCode::TOO_MANY_REQUESTS, StatusCode::BAD_GATEWAY, StatusCode::CREATED] {
            assert!(matches!(response_status(status, delay, true), Err(PurchaseApiError::Http { .. })));
        }
        assert!(matches!(decode_purchase_body(b"{"), Err(PurchaseApiError::Parse(_))));
        for response in [json!({"data": {"orderId": 31}}), json!({"errno": 0, "code": 100009})] {
            assert!(matches!(decode_created_order(&response, None), Err(PurchaseApiError::Parse(_))));
        }
    }

    #[test]
    fn creation_preserves_each_available_identifier() {
        let by_id = decode_created_order(
            &json!({"code": 0, "data": {"orderId": "18446744073709551616", "token": ""}}),
            None,
        ).unwrap();
        assert_eq!(by_id.order_id.as_deref(), Some("18446744073709551616"));
        assert_eq!(by_id.query_token, None);

        let by_token = decode_created_order(
            &json!({"errno": 0, "data": {"orderId": 0, "token": "query-token"}}),
            None,
        ).unwrap();
        assert_eq!(by_token.order_id, None);
        assert_eq!(by_token.query_token.as_deref(), Some("query-token"));
    }

    #[tokio::test]
    async fn query_requires_success_and_only_sends_available_order_id() {
        use purchase_test::{with_responses, Endpoint::Check, Reply};

        let cookies = Arc::new(CookieManager::from_client(Arc::new(Client::new()), ""));
        let pending = json!({"code": 0, "data": {"orderId": 31, "payParam": {"sign": "", "code_url": " "}}});
        let mut payable = pending.clone();
        payable["data"]["payParam"]["code_url"] = json!("weixin://pay/fixture");
        let mut no_code = payable.clone();
        no_code.as_object_mut().unwrap().remove("code");

        let ((), requests) = with_responses(
            [
                (Check, Reply::Json(pending)),
                (Check, Reply::Json(no_code)),
                (Check, Reply::Json(payable.clone())),
                (Check, Reply::Json(payable)),
            ],
            async {
                let pending = check_fake_ticket(cookies.clone(), "7", "query+token", None).await.unwrap();
                assert_eq!(pending.order_id.as_deref(), Some("31"));
                assert!(pending.pay_result.is_none());
                assert!(matches!(
                    check_fake_ticket(cookies.clone(), "7", "query+token", Some("31")).await,
                    Err(PurchaseApiError::Parse(_))
                ));
                assert!(matches!(
                    check_fake_ticket(cookies.clone(), "7", "query+token", Some("32")).await,
                    Err(PurchaseApiError::Parse(_))
                ));
                let payable = check_fake_ticket(cookies, "7", "query+token", Some("31")).await.unwrap();
                assert_eq!(payable.pay_result.unwrap().code_url, "weixin://pay/fixture");
            },
        ).await;
        assert!(requests[0].query.iter().all(|(key, _)| key != "orderId"));
        assert!(requests[1].query.contains(&("orderId".into(), "31".into())));
        assert!(requests[1].query.contains(&("token".into(), "query+token".into())));
    }
}
