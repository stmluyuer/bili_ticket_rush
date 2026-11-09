use std::collections::HashMap;
use std::future::Future;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use common::record_log::TASK_LABEL;

use common::cookie_manager::CookieManager;
use rand::{Rng, SeedableRng, rngs::StdRng};

use tokio::runtime::Runtime;
use tokio::sync::{mpsc, watch};
use common::taskmanager::{*};
use common::captcha::handle_risk_verification;
use common::login::{send_loginsms,sms_login};
use common::push::PushConfig;
use common::gen_cp::CTokenGenerator;
use common::ticket::{*};
use crate::show_orderlist::get_orderlist;
use crate::api::{*};


struct GrabTaskEntry {
    info_seed: RunningTaskInfo,
    start_time: std::time::Instant,
    cancel: watch::Sender<bool>,
    outcome: Option<TaskOutcome>,
}

impl GrabTaskEntry {
    fn new(request: &GrabTicketRequest, seq: u64) -> Self {
        let project = request.biliticket.project_info.as_ref();
        Self {
            info_seed: RunningTaskInfo {
                task_id: request.task_id.clone(),
                seq,
                uid: request.uid,
                account_name: request.biliticket.account.name.clone(),
                project_name: project.and_then(|p| p.name.clone()).unwrap_or_else(|| request.project_id.clone()),
                grab_mode: request.grab_mode,
                status: TaskStatus::Running,
                elapsed_secs: 0,
                phase: GrabPhase::Preparing,
                sale_at: None,
                attempts: 0,
                rounds: 0,
                last_event: None,
                target: grab_target(request.grab_mode, project, &request.screen_id, &request.ticket_id, request.count),
                project_id: request.project_id.clone(),
                ticket_id: request.ticket_id.clone(),
                order: None,
            },
            start_time: std::time::Instant::now(),
            cancel: watch::channel(false).0,
            outcome: None,
        }
    }
}

/// RAII 守卫：任务结束时（正常返回 / return / panic unwind）都会从注册表移除该任务，
/// 移除时把最后快照转入已结束列表，供界面取走。
struct GrabRegistryGuard {
    registry: Arc<Mutex<HashMap<String, GrabTaskEntry>>>,
    finished: Arc<Mutex<Vec<FinishedTaskInfo>>>,
    task_id: String,
}

impl Drop for GrabRegistryGuard {
    fn drop(&mut self) {
        let entry = self.registry.lock().unwrap_or_else(|e| e.into_inner()).remove(&self.task_id);
        let Some(entry) = entry else { return };
        let mut info = entry.info_seed;
        info.elapsed_secs = entry.start_time.elapsed().as_secs();
        let outcome = match entry.outcome {
            Some(outcome) => outcome,
            None if *entry.cancel.borrow() => TaskOutcome::Stopped,
            None if matches!(info.order.as_ref().map(|order| &order.state), Some(OrderState::PendingConfirmation)) => {
                TaskOutcome::AwaitingConfirmation("购票任务异常结束，请核对订单".to_string())
            }
            None => TaskOutcome::Ended("购票任务异常结束".to_string()),
        };
        self.finished.lock().unwrap_or_else(|e| e.into_inner()).push(FinishedTaskInfo {
            info,
            outcome,
            finished_at: SystemTime::now(),
        });
    }
}

// 购票流程在同一任务内运行，进度、发送边界与最终订单共用这一条注册记录。
tokio::task_local! {
    static GRAB_CTX: (Arc<Mutex<HashMap<String, GrabTaskEntry>>>, String);
}

fn report<T>(f: impl FnOnce(&mut GrabTaskEntry) -> T) -> T {
    GRAB_CTX.with(|(registry, task_id)| {
        let mut registry = registry.lock().unwrap_or_else(|e| e.into_inner());
        f(registry.get_mut(task_id).expect("购票任务应已注册"))
    })
}

fn event(text: impl Into<String>, is_error: bool) -> TaskEvent {
    TaskEvent {
        text: text.into(),
        is_error,
        at: SystemTime::now(),
    }
}

fn report_phase(phase: GrabPhase) {
    report(|t| t.info_seed.phase = phase);
}

fn report_event(text: impl Into<String>, is_error: bool) {
    report(|t| t.info_seed.last_event = Some(event(text, is_error)));
}

/// 任务卡片上的“场次 · 票种 × 数量”
fn grab_target(
    mode: u8,
    project_info: Option<&TicketInfo>,
    screen_id: &str,
    ticket_id: &str,
    count: i16,
) -> String {
    if mode == 2 {
        return format!("全部场次 · 捡漏 × {}", count);
    }
    project_info
        .and_then(|p| {
            p.screen_list
                .iter()
                .find(|s| s.id.map(|id| id.to_string()).as_deref() == Some(screen_id))
        })
        .and_then(|screen| {
            let ticket = screen
                .ticket_list
                .iter()
                .find(|t| t.id.map(|id| id.to_string()).as_deref() == Some(ticket_id))?;
            Some(format!(
                "{} · {} × {}",
                screen.name.as_deref()?,
                ticket.desc.as_deref()?,
                count
            ))
        })
        .unwrap_or_else(|| format!("场次 {} · 票种 {} × {}", screen_id, ticket_id, count))
}

pub struct TaskManagerImpl {
    task_sender: mpsc::Sender<TaskMessage>,
    result_receiver: mpsc::Receiver<TaskResult>,
    running_tasks: HashMap<String, Task>, // 使用 Task 枚举
    grab_registry: Arc<Mutex<HashMap<String, GrabTaskEntry>>>,
    grab_finished: Arc<Mutex<Vec<FinishedTaskInfo>>>,
    seq_counter: Arc<AtomicU64>,
    runtime: Arc<Runtime>,
    _worker_thread: Option<thread::JoinHandle<()>>,
}

enum TaskMessage {
    SubmitTask((String, TaskRequest)),
    CancelTask(String),
    Shutdown,
}

impl TaskManager for TaskManagerImpl {
    fn new() -> Self {
        // 创建通道
        let (task_tx, mut task_rx) = mpsc::channel(100);
        let (result_tx, result_rx) = mpsc::channel(100);
        
        // 创建tokio运行时
        let runtime = Arc::new(Runtime::new().unwrap());
        let rt = runtime.clone();

        let grab_registry: Arc<Mutex<HashMap<String, GrabTaskEntry>>> =
            Arc::new(Mutex::new(HashMap::new()));
        let seq_counter: Arc<AtomicU64> = Arc::new(AtomicU64::new(0));
        let grab_finished: Arc<Mutex<Vec<FinishedTaskInfo>>> = Arc::new(Mutex::new(Vec::new()));
        let grab_finished_worker = grab_finished.clone();
        let grab_registry_worker = grab_registry.clone();

        // 启动工作线程
        let worker = thread::spawn(move || {
            rt.block_on(async {
                let mut qr_tasks: HashMap<String, tokio::task::JoinHandle<()>> = HashMap::new();
                while let Some(msg) = task_rx.recv().await {
                    match msg {
        TaskMessage::SubmitTask((task_id, request)) => {
                            let result_tx = result_tx.clone();
                            
                            // 根据任务类型处理
                            match request {
                                
                                TaskRequest::QrCodeLoginRequest(qrcode_req) => {
                                    qr_tasks.retain(|_, handle| !handle.is_finished());
                                    let qr_task_id = task_id.clone();
                                    let qr_task = tokio::spawn(async move {
                                        // 二维码登录逻辑
                                        let status = poll_qrcode_login(&qrcode_req.qrcode_key,qrcode_req.user_agent.as_deref()).await;
                                        
                                        let (cookie, error) = match &status {
                                            common::login::QrCodeLoginStatus::Success(cookie) => 
                                                (Some(cookie.clone()), None),
                                            common::login::QrCodeLoginStatus::Failed(err) => 
                                                (None, Some(err.clone())),
                                            _ => (None, None)
                                        };
                                        
                                        // 创建正确的结果类型
                                        let task_result = TaskResult::QrCodeLoginResult(TaskQrCodeLoginResult {
                                            task_id,
                                            status,
                                            cookie,
                                            error,
                                        });
                                        
                                        let _ = result_tx.send(task_result).await;
                                    });
                                    qr_tasks.insert(qr_task_id, qr_task);
                                }
                                TaskRequest::LoginSmsRequest(login_sms_req) => {
                                    let phone = login_sms_req.phone.clone();
                                    let client = login_sms_req.client.clone();
                                    let custom_config = login_sms_req.custom_config.clone();
                                    let result_tx = result_tx.clone();
                                    let local_captcha = login_sms_req.local_captcha.clone();
                                    

                                    /* let client = match reqwest::Client::builder()
                                        .user_agent(user_agent.clone())
                                        .cookie_store(true)
                                        .build() {
                                            Ok(client) => client,
                                            Err(err) => {
                                               // 记录错误并发送错误结果
                                               log::error!("创建请求客户端失败 ID: {}, 错误: {}", task_id, err);
                
                                               let task_result = TaskResult::LoginSmsResult(LoginSmsRequestResult {
                                                    task_id,
                                                    phone,
                                                    success: false,
                                                    message: format!("创建客户端失败: {}", err),
                                                    });
                
                                               let _ = result_tx.send(task_result).await;
                                               return; 
                                               }
                                               }; */
                                    

                                    tokio::spawn(async move{
                                        log::info!("开始发送短信验证码 ID: {}", task_id);

                                       
                                            log::info!("开始发送短信验证码 ID: {}", task_id);
                                            let response = send_loginsms(
                                                &phone, 
                                                &client, 
                                                custom_config,
                                                local_captcha,
                                            ).await;
                                            log::info!("开始发送短信验证码 ID: {}", task_id);
                                            let success = response.is_ok();
                                            let message = match &response {
                                                    Ok(msg) => msg.clone(),
                                                    Err(err) => {
                                                        log::error!("发送短信验证码失败: {}", err);
                                                        err.to_string()
                                                    },
                                                };
                                            log::info!("发送短信任务完成 ID: {}, 结果: {}", 
                                                task_id, 
                                                if success { "成功" } else { "失败" }
                                            );

                                            let task_result = TaskResult::LoginSmsResult(LoginSmsRequestResult {
                                                task_id,
                                                phone,
                                                success,
                                                message,
                                            });

                                            let _ = result_tx.send(task_result).await;

                                        
                                        


                                    });
                                
                                }
                                TaskRequest::PushRequest(push_req) => {
                                    let push_config = push_req.push_config.clone();
                                    let title = push_req.title.clone();
                                    let message = push_req.message.clone();
                                    let jump_url = push_req.jump_url.clone();
                                    let push_type = push_req.push_type.clone();
                                    let result_tx = result_tx.clone();
                                    
                                    // 启动异步任务处理推送
                                    tokio::spawn(async move {
                                        log::info!("开始处理推送任务 ID: {}, 类型: {:?}", task_id, push_type);
                                        
                                        let (success, result_message) = match push_type {
                                            PushType::All => {
                                                push_config.push_all_async( &title, &message,&jump_url).await
                                            },
                                            
                                            // 其他推送类型的处理...
                                            _ => (false, "未实现的推送类型".to_string())
                                        };
                                        
                                        // 创建任务结果
                                        let task_result = TaskResult::PushResult(PushRequestResult {
                                            task_id: task_id.clone(),
                                            success,
                                            message: result_message,
                                            push_type: push_type.clone(),
                                        });
                                        
                                        // 发送结果
                                        if let Err(e) = result_tx.send(task_result).await {
                                            log::error!("发送推送任务结果失败: {}", e);
                                        }
                                        
                                        log::info!("推送任务 ID: {} 完成, 结果: {}", task_id, 
                                                  if success { "成功" } else { "失败" });
                                    });
                                    
                                 
                                }
                                TaskRequest::SubmitLoginSmsRequest(login_sms_req) => {
                                    let phone = login_sms_req.phone.clone();
                                    let client = login_sms_req.client.clone();
                                    let captcha_key = login_sms_req.captcha_key.clone();
                                    let code = login_sms_req.code.clone();
                                    let result_tx = result_tx.clone();

                                    tokio::spawn(async move{
                                        log::info!("短信验证码登录进行中 ID: {}", task_id);
                                        
                                        let result = async{
                                            let response = sms_login(&phone,  &code,&captcha_key, &client).await;
                                            let success = response.is_ok();
                                            let message: String = match &response {
                                                    Ok(msg) => msg.clone(),
                                                    Err(err) => {
                                                        log::error!("提交短信验证码失败: {}", err);
                                                        err.to_string()
                                                    },
                                                };
                                            let cookie = match &response {
                                                Ok(msg) => Some(msg.clone()),
                                                Err(_) => None,
                                            };
                                            log::info!("提交短信任务完成 ID: {}, 结果: {}", 
                                                task_id, 
                                                if success { "成功" } else { "失败" }
                                            );

                                            let task_result = TaskResult::SubmitSmsLoginResult(SubmitSmsLoginResult {
                                                task_id,
                                                phone,
                                                success,
                                                message,
                                                cookie,
                                            });

                                            let _ = result_tx.send(task_result).await;

                                        }.await;
                                    });
                                }
                                TaskRequest::GetAllorderRequest(get_order_req) => {
                                    let cookie_manager = get_order_req.cookie_manager.clone();
                                    let result_tx = result_tx.clone();
                                    let account_id = get_order_req.account_id.clone();
                                    let cookies = get_order_req.cookies.clone();
                                    tokio::spawn(async move{
                                        log::info!("正在获取全部订单 ID: {}", task_id);
                                        let (success, message, order_info) = match get_orderlist(cookie_manager).await {
                                            Ok(order_resp) => {
                                                let msg = format!("获取全部订单成功: {}", order_resp.data.total);
                                                (true, msg, Some(order_resp))
                                            }
                                            Err(err) => {
                                                log::error!("获取全部订单失败: {}", err);
                                                (false, err, None)
                                            }
                                        };
                                        let task_result = TaskResult::GetAllorderRequestResult(GetAllorderRequestResult {
                                            task_id: task_id.clone(),
                                            success,
                                            message,
                                            order_info,
                                            account_id: account_id.clone(),
                                            timestamp: std::time::Instant::now(),
                                        });
                                        let _ = result_tx.send(task_result).await;
                                    });
                                }
                                TaskRequest::GetTicketInfoRequest(get_ticketinfo_req) => {
                                    let cookie_manager = get_ticketinfo_req.cookie_manager.clone();
                                    let result_tx = result_tx.clone();
                                    let project_id = get_ticketinfo_req.project_id.clone();
                                    let uid = get_ticketinfo_req.uid.clone();
                                    let referer_link = get_ticketinfo_req.referer_link.clone();
                                    tokio::spawn(async move{
                                        log::debug!("正在获取project{}",task_id);
                                        let response  = get_project(cookie_manager, &project_id, &referer_link).await;
                                        let success = response.is_ok();
                                        let ticket_info = match &response{
                                            Ok(info) => {
                                                // 检查数据是否有效
                                                if info.data.screen_list.is_empty() {
                                                    log::warn!("项目信息获取成功但场次列表为空，可能是API格式变化");
                                                }
                                                Some(info.clone())
                                            },
                                            Err(e) => {
                                                log::error!("获取项目时失败，原因：{}",e);
                                                None
                                            }
                                        };
                                        let message = match &response{
                                            Ok(info) => {
                                                //log::debug!("项目{}请求成功",info.errno);
                                                format!("项目{}请求成功",info.errno)
                                            }
                                            Err(e) => {
                                                e.to_string()
                                            }
                                        };
                                        let task_result = TaskResult::GetTicketInfoResult(GetTicketInfoResult{
                                            task_id : task_id.clone(),
                                            uid: uid.clone(),
                                            ticket_info : ticket_info.clone(),
                                            success : success,
                                            message : message.clone(),

                                        });
                                        let _ = result_tx.send(task_result).await;
                                    });
                                }
                                TaskRequest::GetBuyerInfoRequest(get_buyerinfo_req)=>{
                                    let cookie_manager = get_buyerinfo_req.cookie_manager.clone();
                                    let result_tx = result_tx.clone();
                                    let uid = get_buyerinfo_req.uid.clone();
                                    tokio::spawn(async move{
                                        log::debug!("正在获取购票人信息{}",task_id);
                                        let response  = get_buyer_info(cookie_manager).await;
                                        let (success, buyer_info, message) = match response {
                                            Ok(info) => {
                                                // 成功：按账号 uid 写入本地加密缓存（设备码加密），供下次失败时回退。
                                                // 仅在列表非空时缓存，避免用空/异常响应覆盖掉之前的有效缓存。
                                                if !info.data.list.is_empty() {
                                                    if let Err(e) = common::utils::save_buyer_cache(uid, &info.data.list) {
                                                        log::warn!("保存购票人缓存失败：{}", e);
                                                    }
                                                }
                                                (true, Some(info), "购票人信息请求成功".to_string())
                                            }
                                            Err(e) => {
                                                log::error!("获取购票人信息失败，原因：{}", e);
                                                // 失败：尝试从本地加密缓存读取该账号的购票人列表
                                                match common::utils::load_buyer_cache(uid) {
                                                    Ok(list) if !list.is_empty() => {
                                                        log::warn!("已从本地缓存读取账号 {} 的购票人信息（{} 人）", uid, list.len());
                                                        let cached = BuyerInfoResponse {
                                                            errno: 0,
                                                            errtag: 0,
                                                            msg: String::new(),
                                                            code: 0,
                                                            message: String::new(),
                                                            data: BuyerInfoData { list },
                                                        };
                                                        (true, Some(cached), "网络获取购票人失败，已从本地缓存读取".to_string())
                                                    }
                                                    _ => (false, None, e),
                                                }
                                            }
                                        };
                                        let task_result = TaskResult::GetBuyerInfoResult(GetBuyerInfoResult{
                                            task_id : task_id.clone(),
                                            uid: uid.clone(),
                                            buyer_info : buyer_info.clone(),
                                            success : success,
                                            message : message.clone(),

                                        });
                                        let _ = result_tx.send(task_result).await;
                                        
                                    });
                                }
                                TaskRequest::GrabTicketRequest(grab_ticket_req) => {
                                    let guard = GrabRegistryGuard {
                                        registry: grab_registry_worker.clone(),
                                        finished: grab_finished_worker.clone(),
                                        task_id,
                                    };
                                    tokio::spawn(run_registered_grab_task(guard, grab_ticket_req));
                                }
                            }
                        },
                        TaskMessage::CancelTask(task_id) => {
                            if let Some(handle) = qr_tasks.remove(&task_id) {
                                handle.abort();
                            }
                        },
                        TaskMessage::Shutdown => break,
                    }
                }
            });
        });
        
        Self {
            task_sender: task_tx,
            result_receiver: result_rx,
            running_tasks: HashMap::new(),
            grab_registry,
            grab_finished,
            seq_counter,
            runtime: runtime,
            _worker_thread: Some(worker),
        }
    }
    
    fn submit_task(&mut self, mut request: TaskRequest) -> Result<String, String> {
        // 生成任务ID
        let task_id = uuid::Uuid::new_v4().to_string();
        
        // 根据请求类型创建相应的任务
        match &mut request {
            
            TaskRequest::QrCodeLoginRequest(qrcode_req) => {
                log::info!("提交二维码登录任务 ID: {}", task_id);
                // 创建二维码登录任务
                let task = QrCodeLoginTask {
                    task_id: task_id.clone(),
                    qrcode_key: qrcode_req.qrcode_key.clone(),
                    qrcode_url: qrcode_req.qrcode_url.clone(),
                    status: TaskStatus::Pending,
                    start_time: Some(std::time::Instant::now()),
                };
                
                // 保存任务
                self.running_tasks.insert(task_id.clone(), Task::QrCodeLoginTask(task));
            }
            TaskRequest::LoginSmsRequest(login_sms_req) => {
                log::info!("提交短信验证码任务 ID: {}, 手机号: {}", task_id, login_sms_req.phone);
                
                // 创建短信任务
                let task = LoginSmsRequestTask {
                    task_id: task_id.clone(),
                    phone: login_sms_req.phone.clone(),
                    status: TaskStatus::Pending,
                    start_time: Some(std::time::Instant::now()),
                };
                
                // 保存任务
                self.running_tasks.insert(task_id.clone(), Task::LoginSmsRequestTask(task));
            }
            TaskRequest::PushRequest(push_req) => {
                log::info!("提交推送任务 ID: {}", task_id);
                // 创建推送任务
                let task = PushTask {
                    task_id: task_id.clone(),
                    push_type: push_req.push_type.clone(),  // 使用push_type
                    title: push_req.title.clone(),
                    message: push_req.message.clone(),
                    status: TaskStatus::Pending,
                    start_time: Some(std::time::Instant::now()),
                };
                
                // 保存任务
                self.running_tasks.insert(task_id.clone(), Task::PushTask(task));
            }

            TaskRequest::SubmitLoginSmsRequest(login_sms_req) => {
                log::info!("提交短信验证码登录任务 ID: {}, 手机号: {}", task_id, login_sms_req.phone);
                
                // 创建短信验证码登录任务
                let task = SubmitLoginSmsRequestTask {
                    task_id: task_id.clone(),
                    phone: login_sms_req.phone.clone(),
                    code: login_sms_req.code.clone(),
                    captcha_key: login_sms_req.captcha_key.clone(),
                    status: TaskStatus::Pending,
                    start_time: Some(std::time::Instant::now()),
                };
                
                // 保存任务
                self.running_tasks.insert(task_id.clone(), Task::SubmitLoginSmsRequestTask(task));
            }
            TaskRequest::GetAllorderRequest(get_order_req) => {
                log::info!("提交获取全部订单任务 ID: {}", task_id);
                
                // 创建获取全部订单任务
                let task = GetAllorderRequest {
                    task_id: task_id.clone(),
                    cookie_manager: get_order_req.cookie_manager.clone(),
                    status: TaskStatus::Pending,
                    cookies: get_order_req.cookies.clone(),
                    account_id: get_order_req.account_id.clone(),
                    start_time: Some(std::time::Instant::now()),
                };
                
                // 保存任务
                self.running_tasks.insert(task_id.clone(), Task::GetAllorderRequestTask(task));
            }
            TaskRequest::GetTicketInfoRequest(get_ticketinfo_req) => {
                log::info!("{}",task_id);
                let task = GetTicketInfoTask{
                    task_id : task_id.clone(),
                    project_id: get_ticketinfo_req.project_id.clone(),
                    status: TaskStatus::Running,
                    start_time: Some(std::time::Instant::now()),
                    referer_link : get_ticketinfo_req.referer_link.clone(),
                    cookie_manager: get_ticketinfo_req.cookie_manager.clone(), 
                };
                self.running_tasks.insert(task_id.clone(),Task::GetTicketInfoTask(task));
            }
            TaskRequest::GetBuyerInfoRequest(get_buyerinfo_req) => {
                log::info!("提交获取购票人信息任务 ID: {}", task_id);
                
                //创建任务
                let task = GetBuyerInfoTask {
                    uid: get_buyerinfo_req.uid.clone(),
                    task_id: task_id.clone(),
                    cookie_manager: get_buyerinfo_req.cookie_manager.clone(),
                    status: TaskStatus::Pending,
                    start_time: Some(std::time::Instant::now()),
                    
                };
                
                // 保存任务
                self.running_tasks.insert(task_id.clone(), Task::GetBuyerInfoTask(task));
            }
            TaskRequest::GrabTicketRequest(ref mut grab_ticket_req) => {
                log::info!("提交抢票任务 ID: {}", task_id);
                grab_ticket_req.task_id = task_id.clone();
                // 提交返回前即可取消，worker 订阅时也能看到已提交的取消请求。
                let seq = self.seq_counter.fetch_add(1, Ordering::Relaxed) + 1;
                self.grab_registry.lock().unwrap_or_else(|e| e.into_inner()).insert(
                    task_id.clone(),
                    GrabTaskEntry::new(grab_ticket_req, seq),
                );
            }

        }
        
        // 发送任务
        if let Err(e) = self.task_sender.blocking_send(TaskMessage::SubmitTask((task_id.clone(), request))) {
            self.grab_registry.lock().unwrap_or_else(|e| e.into_inner()).remove(&task_id);
            return Err(format!("无法提交任务: {}", e));
        }
        
        Ok(task_id)
    }
    
    fn get_results(&mut self) -> Vec<TaskResult> {
        let mut results = Vec::new();
        
        // 非阻塞方式获取所有可用结果
        while let Ok(result) = self.result_receiver.try_recv() {
            results.push(result);
        }
        
        results
    }
    
    fn cancel_task(&mut self, task_id: &str) -> Result<(), String> {
        if matches!(self.running_tasks.get(task_id), Some(Task::QrCodeLoginTask(_))) {
            // 提交和取消共用队列，立即关闭时也会先注册轮询句柄再终止。
            self.task_sender
                .blocking_send(TaskMessage::CancelTask(task_id.to_string()))
                .map_err(|e| format!("无法取消任务: {}", e))?;
            self.running_tasks.remove(task_id);
            return Ok(());
        }
        let registry = self
            .grab_registry
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        match registry.get(task_id) {
            Some(entry) => {
                entry.cancel.send_replace(true);
                Ok(())
            }
            None => Err("任务不存在".to_string()),
        }
    }

    /// 注意：当前仅反映抢票任务（grab_registry）。其它任务类型（登录/订单等）始终返回 None。
    fn get_task_status(&self, task_id: &str) -> Option<TaskStatus> {
        let registry = self
            .grab_registry
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        registry.get(task_id).map(|entry| {
            if *entry.cancel.borrow() {
                TaskStatus::Cancelling
            } else {
                TaskStatus::Running
            }
        })
    }

    fn list_running_tasks(&self) -> Vec<RunningTaskInfo> {
        let registry = self
            .grab_registry
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let mut tasks: Vec<RunningTaskInfo> = registry
            .values()
            .map(|entry| {
                let mut info = entry.info_seed.clone();
                info.elapsed_secs = entry.start_time.elapsed().as_secs();
                if *entry.cancel.borrow() {
                    info.status = TaskStatus::Cancelling;
                }
                info
            })
            .collect();
        tasks.sort_by_key(|t| t.seq);
        tasks
    }

    fn take_finished_tasks(&mut self) -> Vec<FinishedTaskInfo> {
        let mut finished = self
            .grab_finished
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        std::mem::take(&mut *finished)
    }

    fn shutdown(&mut self) {
        let _ = self.task_sender.blocking_send(TaskMessage::Shutdown);
        if let Some(handle) = self._worker_thread.take() {
            let _ = handle.join();
        }
    }
}

const CREATE_INTERVAL: Duration = Duration::from_millis(400);
const PREPARE_INTERVAL: Duration = Duration::from_secs(1);
const CONFIRM_INTERVAL: Duration = Duration::from_millis(300);
const ORDER_CHECK_INTERVAL: Duration = Duration::from_secs(1);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);
const CAPTCHA_TIMEOUT: Duration = Duration::from_secs(30);
const MAX_TOKEN_REFRESHES: u8 = 5;
const MAX_CONFIRM_ATTEMPTS: u8 = 4;
const MAX_ORDER_CHECKS: u32 = 30;

enum TargetExit {
    NextTarget(String),
    Finished(TaskOutcome),
}

enum PreparedExit {
    RefreshToken { reason: String, delay: Duration },
    Target(TargetExit),
}

enum BusinessRejection {
    Retry(Duration),
    RefreshToken,
    Target,
    Task,
}

fn business_rejection(code: i64) -> Option<BusinessRejection> {
    Some(match code {
        100001 | 429 | 900001 => BusinessRejection::Retry(Duration::from_millis(300)),
        100009 => BusinessRejection::Retry(Duration::from_millis(600)),
        211 => BusinessRejection::Retry(Duration::ZERO),
        3 => BusinessRejection::Retry(Duration::from_millis(4800)),
        100041 | 100050 | 900002 => BusinessRejection::RefreshToken,
        100016 | 100017 | 100039 | 100080 | 100082 => BusinessRejection::Target,
        1 | 83000004 | 100079 | 100003 | 209001 | 919 | 999 => BusinessRejection::Task,
        _ => return None,
    })
}

async fn while_running<T>(
    cancel: &mut watch::Receiver<bool>,
    action: impl Future<Output = T>,
) -> Option<T> {
    tokio::select! {
        biased;
        _ = cancel.wait_for(|stopped| *stopped) => None,
        result = action => Some(result),
    }
}

async fn wait_running(cancel: &mut watch::Receiver<bool>, delay: Duration) -> bool {
    while_running(cancel, tokio::time::sleep(delay)).await.is_some()
}

async fn run_registered_grab_task(guard: GrabRegistryGuard, mut request: GrabTicketRequest) {
    let (task_label, mut cancel) = {
        let registry = guard.registry.lock().unwrap_or_else(|e| e.into_inner());
        let entry = &registry[&guard.task_id];
        (
            format!("#{} {}", entry.info_seed.seq, entry.info_seed.account_name),
            entry.cancel.subscribe(),
        )
    };
    let grab_ctx = (guard.registry.clone(), guard.task_id.clone());
    let push_config = request.biliticket.push_self.clone();
    let notification = GRAB_CTX
        .scope(grab_ctx, TASK_LABEL.scope(task_label, async {
            let outcome = run_grab(&mut request, &mut cancel).await;
            report(|entry| {
                entry.outcome = Some(outcome);
                if !push_config.enabled {
                    return None;
                }
                let order = entry.info_seed.order.as_ref()?;
                let OrderState::AwaitingPayment(payment) = &order.state else { return None };
                Some(PaymentNotice::new(order, payment))
            })
        }))
        .await;

    drop(guard);
    if let Some(notification) = notification {
        #[cfg(test)]
        tests::observe_notice(&notification);
        tokio::spawn(notification.push(push_config));
    }
}

struct PaymentNotice {
    title: String,
    message: String,
    jump_url: Option<String>,
}

impl PaymentNotice {
    fn new(order: &OrderContext, payment: &CheckFakeResultData) -> Self {
        let confirm = &order.confirm;
        let amount = format!("{}.{:02}", confirm.pay_money / 100, confirm.pay_money % 100);
        Self {
            title: format!("恭喜{}抢票成功！", confirm.project_name),
            message: format!(
                "抢票成功！\n项目：{}\n场次：{}\n票种：{}\n支付链接：{}\n请尽快支付{}元",
                confirm.project_name, confirm.screen_name, confirm.ticket_info.name, payment.code_url, amount,
            ),
            jump_url: order.order_id.as_ref().map(|id| {
                format!("bilibili://mall/web?url=https://mall.bilibili.com/neul-next/ticket/orderDetail.html?order_id={id}")
            }),
        }
    }

    async fn push(self, config: PushConfig) {
        let (success, _) = config.push_all_async(&self.title, &self.message, &self.jump_url).await;
        if !success {
            log::warn!("购票通知发送失败");
        }
    }
}

async fn run_grab(request: &mut GrabTicketRequest, cancel: &mut watch::Receiver<bool>) -> TaskOutcome {
    if *cancel.borrow() {
        return TaskOutcome::Stopped;
    }
    if request.uid != request.biliticket.uid || request.uid != request.biliticket.account.uid {
        return TaskOutcome::Ended("购票请求的账号与提交快照不符".to_string());
    }
    match request.grab_mode {
        0 | 1 => {
            let target = match select_target(request) {
                Ok(target) => target,
                Err(error) => return TaskOutcome::Ended(error.to_string()),
            };
            let sale_begin = request.biliticket.project_info.as_ref().and_then(|project| project.sale_begin);
            let cpdd = token_generator(&request.cookie_manager, sale_begin);
            if request.grab_mode == 0 {
                if let Err(outcome) = wait_for_sale(request, cancel).await {
                    return outcome;
                }
            }
            let prepare_limit = if request.grab_mode == 0 { 5 } else { 10 };
            match purchase_target(request, &target, cpdd, prepare_limit, 60, cancel).await {
                TargetExit::NextTarget(reason) => TaskOutcome::Ended(reason),
                TargetExit::Finished(outcome) => outcome,
            }
        }
        2 => monitor_targets(request, cancel).await,
        _ => TaskOutcome::Ended("未知购票模式".to_string()),
    }
}

fn select_target(request: &mut GrabTicketRequest) -> Result<PurchaseTarget, PurchaseApiError> {
    let target = PurchaseTarget::new(&request.project_id, &request.screen_id, &request.ticket_id, request.count)?;
    let ticket = &mut request.biliticket;
    ticket.project_id.clone_from(&request.project_id);
    ticket.screen_id.clone_from(&request.screen_id);
    ticket.select_ticket_id = Some(request.ticket_id.clone());
    ticket.count = Some(target.count);
    Ok(target)
}

fn token_generator(cookie_manager: &CookieManager, sale_begin: Option<i64>) -> Arc<Mutex<CTokenGenerator>> {
    Arc::new(Mutex::new(CTokenGenerator::new(
        sale_begin.unwrap_or_else(|| chrono::Utc::now().timestamp()),
        0,
        StdRng::from_entropy().gen_range(2000..10000),
        cookie_manager.get_ua(),
    )))
}

async fn wait_for_sale(request: &GrabTicketRequest, cancel: &mut watch::Receiver<bool>) -> Result<(), TaskOutcome> {
    let info = &request.biliticket.project_info;
    let countdown = get_countdown(request.cookie_manager.clone(), info.clone());
    let mut countdown = match while_running(cancel, countdown).await {
        Some(Ok(seconds)) => seconds,
        Some(Err(error)) => return Err(TaskOutcome::Ended(format!("获取倒计时失败：{error}"))),
        None => return Err(TaskOutcome::Stopped),
    };
    if countdown <= 0.0 {
        return Ok(());
    }
    report(|entry| {
        entry.info_seed.phase = GrabPhase::WaitingSale;
        entry.info_seed.sale_at = info.as_ref().and_then(|project| project.sale_begin);
    });
    while countdown > 20.0 {
        if !wait_running(cancel, Duration::from_secs(15)).await {
            return Err(TaskOutcome::Stopped);
        }
        countdown -= 15.0;
    }
    while countdown > 1.3 {
        if !wait_running(cancel, Duration::from_secs(1)).await {
            return Err(TaskOutcome::Stopped);
        }
        countdown -= 1.0;
    }
    if !wait_running(cancel, Duration::from_millis(800)).await {
        return Err(TaskOutcome::Stopped);
    }
    Ok(())
}

async fn monitor_targets(request: &mut GrabTicketRequest, cancel: &mut watch::Receiver<bool>) -> TaskOutcome {
    // 捡漏提交时尚无场次或票档，先核对整个任务共用的参数。
    if request.count <= 0 {
        return TaskOutcome::Ended("购票数量必须大于 0".to_string());
    }
    if !request.project_id.parse::<i64>().is_ok_and(|id| id > 0) {
        return TaskOutcome::Ended("项目 ID 无效".to_string());
    }
    loop {
        report_phase(GrabPhase::Monitoring);
        let project = get_project(
            request.cookie_manager.clone(), &request.project_id, &request.biliticket.referer,
        );
        let project = match while_running(cancel, project).await {
            Some(Ok(project)) => project.data,
            Some(Err(error)) => {
                report_event(format!("获取项目数据失败：{error}"), true);
                if !wait_running(cancel, Duration::from_secs(2)).await {
                    return TaskOutcome::Stopped;
                }
                continue;
            }
            None => return TaskOutcome::Stopped,
        };

        let Some(id_bind @ (1 | 2)) = project.id_bind else {
            return TaskOutcome::Ended("暂不支持非实名票捡漏".to_string());
        };
        request.biliticket.id_bind = id_bind;
        request.is_hot = project.hot_project.unwrap_or(false);
        for screen in project.screen_list.into_iter().filter(|screen| screen.clickable) {
            let Some(screen_id) = screen.id else { continue };
            for ticket in screen.ticket_list.into_iter().filter(|ticket| ticket.clickable == Some(true)) {
                if *cancel.borrow() {
                    return TaskOutcome::Stopped;
                }
                let Some(ticket_id) = ticket.id else { continue };
                let screen_name = ticket.screen_name.as_deref().or(screen.name.as_deref()).unwrap_or("未知场次");
                let ticket_name = ticket.desc.as_deref().unwrap_or("未知票种");
                if request.skip_words.as_ref().is_some_and(|words| {
                    let screen = screen_name.to_lowercase();
                    let ticket = ticket_name.to_lowercase();
                    words.iter().any(|word| {
                        let word = word.to_lowercase();
                        screen.contains(&word) || ticket.contains(&word)
                    })
                }) {
                    continue;
                }
                request.screen_id = screen_id.to_string();
                request.ticket_id = ticket_id.to_string();
                let target = match select_target(request) {
                    Ok(target) => target,
                    Err(error) => {
                        report_event(error.to_string(), true);
                        continue;
                    }
                };

                report_event(format!("发现候选票档：{screen_name} {ticket_name}"), false);
                let cpdd = token_generator(&request.cookie_manager, project.sale_begin);
                match purchase_target(request, &target, cpdd, 5, 30, cancel).await {
                    TargetExit::NextTarget(reason) => report_event(reason, true),
                    TargetExit::Finished(outcome) => return outcome,
                }
            }
        }
        report(|entry| {
            entry.info_seed.phase = GrabPhase::Monitoring;
            entry.info_seed.rounds += 1;
            if entry.info_seed.last_event.as_ref().is_none_or(|last| !last.is_error) {
                entry.info_seed.last_event = Some(event(format!("第 {} 轮检查完毕", entry.info_seed.rounds), false));
            }
        });
        if !wait_running(cancel, Duration::from_secs(2)).await {
            return TaskOutcome::Stopped;
        }
    }
}

async fn purchase_target(
    request: &GrabTicketRequest,
    target: &PurchaseTarget,
    cpdd: Arc<Mutex<CTokenGenerator>>,
    prepare_limit: u8,
    create_limit: u32,
    cancel: &mut watch::Receiver<bool>,
) -> TargetExit {
    let mut create_attempts = 0;
    let mut refreshes = 0;
    loop {
        let (token, ptoken) = match prepare_token(request, target, &cpdd, prepare_limit, cancel).await {
            Ok(tokens) => tokens,
            Err(exit) => return exit,
        };
        let exit = match confirm_parameters(request, target, &token, cancel).await {
            Ok(confirmation) => {
                create_on_target(
                    request, target, &cpdd, &token, &ptoken, confirmation,
                    &mut create_attempts, create_limit, cancel,
                ).await
            }
            Err(exit) => exit,
        };
        match exit {
            PreparedExit::Target(exit) => return exit,
            PreparedExit::RefreshToken { reason, delay } => {
                if create_attempts == create_limit {
                    return create_limit_reached(create_limit);
                }
                if refreshes == MAX_TOKEN_REFRESHES {
                    return TargetExit::NextTarget(format!("购票 token 刷新已达上限：{reason}"));
                }
                refreshes += 1;
                report_event(format!("购票 token 失效，重新准备（{refreshes}/{MAX_TOKEN_REFRESHES}）：{reason}"), false);
                if !wait_running(cancel, delay).await {
                    return TargetExit::Finished(TaskOutcome::Stopped);
                }
            }
        }
    }
}

async fn prepare_token(
    request: &GrabTicketRequest,
    target: &PurchaseTarget,
    cpdd: &Arc<Mutex<CTokenGenerator>>,
    limit: u8,
    cancel: &mut watch::Receiver<bool>,
) -> Result<(String, String), TargetExit> {
    let mut failures = 0;
    loop {
        report_phase(GrabPhase::Preparing);
        let prepare = get_ticket_token(request.cookie_manager.clone(), cpdd.clone(), target, request.is_hot);
        let result = while_running(cancel, prepare).await;
        let (reason, delay) = match result {
            Some(Ok(tokens)) => return Ok(tokens),
            Some(Err(PurchaseApiError::Challenge(risk))) => {
                report_phase(GrabPhase::Captcha);
                let captcha = handle_risk_verification(
                    request.cookie_manager.clone(),
                    risk,
                    &request.biliticket.config,
                    &request.biliticket.account.csrf,
                    request.local_captcha.clone(),
                );
                let reason = match while_running(cancel, tokio::time::timeout(CAPTCHA_TIMEOUT, captcha)).await {
                    Some(Ok(Ok(()))) => "验证码已完成，继续准备购票".to_string(),
                    Some(Ok(Err(error))) => format!("验证码处理失败：{error}"),
                    Some(Err(_)) => "验证码处理超时".to_string(),
                    None => return Err(TargetExit::Finished(TaskOutcome::Stopped)),
                };
                (reason, PREPARE_INTERVAL)
            }
            Some(Err(error)) => {
                let reason = format!("准备购票失败：{error}");
                let mut delay = PREPARE_INTERVAL.max(error.retry_after().unwrap_or_default());
                match error {
                    PurchaseApiError::Business { code, .. } => match business_rejection(code) {
                        Some(BusinessRejection::Target) => return Err(TargetExit::NextTarget(reason)),
                        Some(BusinessRejection::Task) => return Err(TargetExit::Finished(TaskOutcome::Ended(reason))),
                        Some(BusinessRejection::Retry(wait)) => delay = delay.max(wait),
                        _ => {}
                    },
                    PurchaseApiError::InvalidParameter(_) => {
                        return Err(TargetExit::Finished(TaskOutcome::Ended(reason)));
                    }
                    _ => {}
                }
                (reason, delay)
            }
            None => return Err(TargetExit::Finished(TaskOutcome::Stopped)),
        };
        failures += 1;
        if failures == limit {
            return Err(TargetExit::NextTarget(format!("准备购票已达上限（{limit} 次）：{reason}")));
        }
        report_event(reason, true);
        if !wait_running(cancel, delay).await {
            return Err(TargetExit::Finished(TaskOutcome::Stopped));
        }
    }
}

async fn confirm_parameters(
    request: &GrabTicketRequest,
    target: &PurchaseTarget,
    token: &str,
    cancel: &mut watch::Receiver<bool>,
) -> Result<ConfirmTicketResult, PreparedExit> {
    report_phase(GrabPhase::Confirming);
    let mut failures = 0;
    loop {
        let confirm = confirm_ticket_order(request.cookie_manager.clone(), target, token);
        let result = while_running(cancel, confirm).await;
        let (reason, delay) = match result {
            Some(Ok(confirmation)) => return Ok(confirmation),
            Some(Err(error)) => {
                let reason = format!("确认订单参数失败：{error}");
                let mut delay = CONFIRM_INTERVAL.max(error.retry_after().unwrap_or_default());
                match error {
                    PurchaseApiError::Business { code, .. } => match business_rejection(code) {
                        Some(BusinessRejection::RefreshToken) => return Err(PreparedExit::RefreshToken { reason, delay }),
                        Some(BusinessRejection::Task) => return Err(PreparedExit::Target(TargetExit::Finished(TaskOutcome::Ended(reason)))),
                        Some(BusinessRejection::Target) | None => {
                            return Err(PreparedExit::Target(TargetExit::NextTarget(reason)));
                        }
                        Some(BusinessRejection::Retry(wait)) => delay = delay.max(wait),
                    },
                    PurchaseApiError::InvalidParameter(_) => {
                        return Err(PreparedExit::Target(TargetExit::NextTarget(reason)));
                    }
                    _ => {}
                }
                (reason, delay)
            }
            None => return Err(PreparedExit::Target(TargetExit::Finished(TaskOutcome::Stopped))),
        };
        failures += 1;
        if failures == MAX_CONFIRM_ATTEMPTS {
            return Err(PreparedExit::Target(TargetExit::NextTarget(format!(
                "确认订单参数已达上限（{MAX_CONFIRM_ATTEMPTS} 次）：{reason}"
            ))));
        }
        report_event(reason, true);
        if !wait_running(cancel, delay).await {
            return Err(PreparedExit::Target(TargetExit::Finished(TaskOutcome::Stopped)));
        }
    }
}

fn create_limit_reached(limit: u32) -> TargetExit {
    TargetExit::NextTarget(format!("创建订单已达上限（{limit} 次）"))
}

async fn create_on_target(
    request: &GrabTicketRequest,
    target: &PurchaseTarget,
    cpdd: &Arc<Mutex<CTokenGenerator>>,
    token: &str,
    ptoken: &str,
    confirm: ConfirmTicketResult,
    attempts: &mut u32,
    limit: u32,
    cancel: &mut watch::Receiver<bool>,
) -> PreparedExit {
    let mut order = OrderContext {
        order_id: None,
        query_token: None,
        project_id: request.project_id.clone(),
        screen_id: request.screen_id.clone(),
        ticket_id: request.ticket_id.clone(),
        confirm,
        state: OrderState::PendingConfirmation,
    };
    loop {
        if *cancel.borrow() {
            return PreparedExit::Target(TargetExit::Finished(TaskOutcome::Stopped));
        }
        if *attempts == limit {
            return PreparedExit::Target(create_limit_reached(limit));
        }
        report_phase(GrabPhase::Grabbing);
        let attempts_before = *attempts;
        let create = create_order(
            request.cookie_manager.clone(), cpdd.clone(), target, token, ptoken, &order.confirm,
            request.is_hot, &request.biliticket, &request.buyer_info, true, attempts_before >= 3, false, None,
            || report(|entry| {
                // 取消和发送边界使用同一把锁。跨过边界后必须等待本次请求收束。
                if *entry.cancel.borrow() {
                    return false;
                }
                *attempts += 1;
                entry.info_seed.attempts += 1;
                entry.info_seed.order = Some(order.clone());
                true
            }),
        );
        let error = match tokio::time::timeout(REQUEST_TIMEOUT, create).await {
            Ok(Ok(created)) => {
                order.order_id = created.order_id;
                order.query_token = created.query_token;
                report(|entry| entry.info_seed.order = Some(order.clone()));
                let outcome = check_existing_order(&request.cookie_manager, order, cancel).await;
                return PreparedExit::Target(TargetExit::Finished(outcome));
            }
            Ok(Err(PurchaseApiError::Cancelled)) => return PreparedExit::Target(TargetExit::Finished(TaskOutcome::Stopped)),
            Ok(Err(error)) => error,
            Err(_) => {
                let reason = "创建订单请求超时".to_string();
                let outcome = if *attempts == attempts_before {
                    if *cancel.borrow() { TaskOutcome::Stopped } else { TaskOutcome::Ended(reason) }
                } else {
                    unresolved_order(cancel, reason)
                };
                return PreparedExit::Target(TargetExit::Finished(outcome));
            }
        };
        let reason = format!("创建订单失败：{error}");
        let delay = CREATE_INTERVAL.max(error.retry_after().unwrap_or_default());
        match error {
            PurchaseApiError::Business { code, .. } => {
                let Some(rejection) = business_rejection(code) else {
                    return PreparedExit::Target(TargetExit::Finished(unresolved_order(cancel, reason)));
                };
                // 只有明确的业务拒绝才清掉本次发送记录，允许按类别继续。
                report(|entry| entry.info_seed.order = None);
                if *cancel.borrow() {
                    return PreparedExit::Target(TargetExit::Finished(TaskOutcome::Stopped));
                }
                match rejection {
                    BusinessRejection::Retry(wait) => {
                        report_event(reason, false);
                        if (*attempts < limit || request.grab_mode == 2)
                            && !wait_running(cancel, delay.max(wait)).await
                        {
                            return PreparedExit::Target(TargetExit::Finished(TaskOutcome::Stopped));
                        }
                    }
                    BusinessRejection::RefreshToken => return PreparedExit::RefreshToken { reason, delay },
                    BusinessRejection::Target => return PreparedExit::Target(TargetExit::NextTarget(reason)),
                    BusinessRejection::Task => {
                        return PreparedExit::Target(TargetExit::Finished(TaskOutcome::Ended(reason)));
                    }
                }
            }
            _ => {
                let outcome = if *attempts == attempts_before {
                    if *cancel.borrow() { TaskOutcome::Stopped } else { TaskOutcome::Ended(reason) }
                } else {
                    unresolved_order(cancel, reason)
                };
                return PreparedExit::Target(TargetExit::Finished(outcome));
            }
        }
    }
}

fn unresolved_order(cancel: &watch::Receiver<bool>, reason: String) -> TaskOutcome {
    report_event(reason.clone(), true);
    if *cancel.borrow() {
        TaskOutcome::Stopped
    } else {
        TaskOutcome::AwaitingConfirmation(reason)
    }
}

async fn check_existing_order(
    cookie_manager: &Arc<CookieManager>,
    mut order: OrderContext,
    cancel: &mut watch::Receiver<bool>,
) -> TaskOutcome {
    if *cancel.borrow() {
        return TaskOutcome::Stopped;
    }
    let Some(query_token) = order.query_token.clone() else {
        return unresolved_order(cancel, "订单已受理，缺少查询标识，请核对订单".to_string());
    };
    report_phase(GrabPhase::CheckingOrder);
    let mut checks = 0;
    loop {
        let check = async {
            // 在开始查单时扣预算。取消先于这一边界时，不发起新查询。
            if report(|entry| *entry.cancel.borrow()) {
                return Err(PurchaseApiError::Cancelled);
            }
            checks += 1;
            check_fake_ticket(
                cookie_manager.clone(), &order.project_id, &query_token, order.order_id.as_deref(),
            ).await
        };
        let result = while_running(cancel, check).await;
        let (reason, delay) = match result {
            Some(Ok(checked)) => {
                if order.order_id.is_none() {
                    order.order_id = checked.order_id;
                }
                if let Some(payment) = checked.pay_result {
                    order.state = OrderState::AwaitingPayment(payment);
                    report(|entry| {
                        entry.info_seed.order = Some(order);
                        entry.info_seed.last_event = Some(event("订单已确认，等待付款", false));
                    });
                    return TaskOutcome::Succeeded;
                }
                report(|entry| entry.info_seed.order = Some(order.clone()));
                (
                    "等待订单支付信息".to_string(),
                    ORDER_CHECK_INTERVAL.max(checked.retry_after.unwrap_or_default()),
                )
            }
            Some(Err(PurchaseApiError::Cancelled)) | None => return TaskOutcome::Stopped,
            Some(Err(error)) => (
                format!("查询订单暂未确认：{error}"),
                ORDER_CHECK_INTERVAL.max(error.retry_after().unwrap_or_default()),
            ),
        };
        if checks == MAX_ORDER_CHECKS {
            return unresolved_order(cancel, format!("订单查询已达上限（{MAX_ORDER_CHECKS} 次）：{reason}，请核对订单"));
        }
        report_event(reason, false);
        if !wait_running(cancel, delay).await {
            return TaskOutcome::Stopped;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::purchase_test::{with_responses, Endpoint, RecordedRequest, Reply};
    use common::account::Account;
    use common::captcha::LocalCaptcha;
    use common::utility::CustomConfig;
    use reqwest::StatusCode;
    use serde_json::{json, Value};
    use tokio::sync::Notify;

    const SALE_BEGIN: i64 = 1_800_000_000;
    type Response = (Endpoint, Reply);
    type NoticeProbe = Box<dyn Fn(&PaymentNotice) + Send>;

    tokio::task_local! {
        static NOTICE_PROBE: NoticeProbe;
    }

    pub(super) fn observe_notice(notice: &PaymentNotice) {
        let _ = NOTICE_PROBE.try_with(|probe| probe(notice));
    }

    struct Fixture {
        guard: GrabRegistryGuard,
        request: GrabTicketRequest,
    }

    struct Trace {
        finished: FinishedTaskInfo,
        requests: Vec<RecordedRequest>,
        notices: Vec<(String, Option<String>)>,
    }

    impl Fixture {
        fn new(mode: u8) -> Self {
            let cookie_manager = Arc::new(CookieManager::from_client(Arc::new(reqwest::Client::new()), ""));
            let account = Account {
                uid: 123,
                name: "测试账号".into(),
                level: String::new(),
                cookie: String::new(),
                csrf: String::new(),
                is_login: true,
                account_status: String::new(),
                vip_label: String::new(),
                is_active: true,
                avatar_url: None,
                avatar_texture: None,
                cookie_manager: Some(cookie_manager.clone()),
            };
            let mut biliticket = BilibiliTicket::new(
                &mode,
                &"购票测试".to_string(),
                &CustomConfig::new(),
                &account,
                &PushConfig::new(),
                &0,
                "11",
                "https://show.bilibili.com/",
            );
            biliticket.project_info = Some(serde_json::from_value(project_data(&[31])).unwrap());
            biliticket.id_bind = 2;
            let buyer_info = [100, 101].into_iter().map(|id| {
                serde_json::from_value(json!({
                    "id": id, "uid": 123, "personal_id": "", "name": "测试购票人",
                    "tel": "", "id_type": 0, "is_default": 0,
                })).unwrap()
            }).collect();
            let request = GrabTicketRequest {
                task_id: "test-grab".into(),
                uid: account.uid,
                project_id: "11".into(),
                screen_id: if mode == 2 { String::new() } else { "21".into() },
                ticket_id: if mode == 2 { String::new() } else { "31".into() },
                count: 2,
                buyer_info,
                cookie_manager,
                biliticket,
                grab_mode: mode,
                status: TaskStatus::Pending,
                start_time: None,
                is_hot: true,
                local_captcha: LocalCaptcha::default(),
                skip_words: None,
            };
            let guard = GrabRegistryGuard {
                registry: Arc::new(Mutex::new(HashMap::from([(
                    request.task_id.clone(),
                    GrabTaskEntry::new(&request, 1),
                )]))),
                finished: Arc::new(Mutex::new(Vec::new())),
                task_id: request.task_id.clone(),
            };
            Self { guard, request }
        }

        async fn run(self, responses: Vec<Response>) -> Trace {
            let finished = self.guard.finished.clone();
            let finished_for_probe = finished.clone();
            let registry = self.guard.registry.clone();
            let registry_for_probe = registry.clone();
            let notices = Arc::new(Mutex::new(Vec::new()));
            let notices_for_probe = notices.clone();
            let (_, requests) = NOTICE_PROBE.scope(
                Box::new(move |notice| {
                    assert!(registry_for_probe.lock().unwrap().is_empty());
                    assert_eq!(finished_for_probe.lock().unwrap().len(), 1);
                    notices_for_probe.lock().unwrap().push((notice.message.clone(), notice.jump_url.clone()));
                }),
                with_responses(responses, run_registered_grab_task(self.guard, self.request)),
            ).await;
            assert!(registry.lock().unwrap().is_empty());
            let mut finished = finished.lock().unwrap();
            assert_eq!(finished.len(), 1);
            let notices = std::mem::take(&mut *notices.lock().unwrap());
            Trace { finished: finished.pop().unwrap(), requests, notices }
        }
    }

    fn project_data(skus: &[i64]) -> Value {
        let tickets: Vec<Value> = skus.iter().map(|sku| json!({
            "id": sku, "clickable": true, "desc": format!("票档 {sku}"), "sale_flag": {}
        })).collect();
        json!({
            "id": 11, "name": "测试项目", "sale_begin": SALE_BEGIN,
            "id_bind": 2, "hotProject": true,
            "screen_list": [{
                "id": 21, "name": "测试场次", "ticket_list": tickets, "clickable": true,
                "sale_start": SALE_BEGIN, "sale_end": SALE_BEGIN + 60, "sale_flag_number": 2,
                "show_date": "测试日期"
            }]
        })
    }

    fn clock(now: i64) -> Response {
        (Endpoint::Clock, Reply::Json(json!({ "code": 0, "data": { "now": now } })))
    }

    fn prepare(token: &str) -> Response {
        (Endpoint::Prepare, Reply::Json(json!({
            "errno": 0, "data": { "token": token, "ptoken": format!("{token}-p") }
        })))
    }

    fn confirm(amount: i64) -> Response {
        (Endpoint::Confirm, Reply::Json(json!({
            "errno": 0,
            "data": {
                "count": 2, "pay_money": amount, "project_name": "测试项目",
                "screen_name": "测试场次", "ticket_info": { "name": "测试票档", "count": 2, "price": 999 }
            }
        })))
    }

    fn rejection(endpoint: Endpoint, code: i64) -> Response {
        (endpoint, Reply::Json(json!({"errno": code, "msg": "测试接口拒绝"})))
    }

    fn created(order_id: Option<u64>, query_token: Option<&str>) -> Response {
        (Endpoint::Create, Reply::Json(json!({
            "errno": 0, "data": { "orderId": order_id, "token": query_token }
        })))
    }

    fn payable(order_id: Option<u64>) -> Response {
        (Endpoint::Check, Reply::Json(json!({
            "errno": 0, "data": {
                "orderId": order_id, "payParam": { "sign": "test", "code_url": "https://payment.invalid/" }
            }
        })))
    }

    fn query<'a>(request: &'a RecordedRequest, key: &str) -> Option<&'a str> {
        request.query.iter().find_map(|(name, value)| (name == key).then_some(value.as_str()))
    }

    fn requests(trace: &Trace, endpoint: Endpoint) -> impl Iterator<Item = &RecordedRequest> {
        trace.requests.iter().filter(move |request| request.endpoint == endpoint)
    }

    fn assert_amount_and_target(request: &RecordedRequest, sku: i64, amount: Option<i64>) {
        let body = request.body.as_ref().unwrap();
        assert_eq!(body["project_id"], 11);
        assert_eq!(body["screen_id"], 21);
        assert_eq!(body["sku_id"], sku);
        assert_eq!(body["count"], 2);
        if let Some(amount) = amount {
            assert_eq!(body["pay_money"], amount);
            let buyers: Vec<BuyerInfo> = serde_json::from_str(body["buyer_info"].as_str().unwrap()).unwrap();
            assert_eq!(buyers.into_iter().map(|buyer| buyer.id).collect::<Vec<_>>(), [100, 101]);
        }
    }

    #[tokio::test]
    async fn timed_mode_refreshes_and_checks_last_allowed_create() {
        let mut fixture = Fixture::new(0);
        fixture.request.biliticket.push_self.enabled = true;
        let mut responses = vec![
            clock(SALE_BEGIN),
            prepare("before-confirm"),
            rejection(Endpoint::Confirm, 900002),
            prepare("before-create"),
            confirm(0),
            rejection(Endpoint::Create, 100041),
            prepare("refreshed"),
            confirm(0),
        ];
        responses.extend((0..58).map(|_| rejection(Endpoint::Create, 211)));
        responses.extend([
            created(None, Some("same+query")),
            (Endpoint::Check, Reply::Http(StatusCode::SERVICE_UNAVAILABLE, None)),
            (Endpoint::Check, Reply::Json(json!({
                "errno": 0, "data": { "orderId": 75, "payParam": { "sign": "test", "code_url": "" } }
            }))),
            payable(Some(75)),
        ]);

        let trace = fixture.run(responses).await;
        assert!(matches!(trace.finished.outcome, TaskOutcome::Succeeded));
        assert_eq!(trace.finished.info.attempts, 60);
        assert_eq!(trace.finished.info.uid, 123);
        let order = trace.finished.info.order.as_ref().unwrap();
        assert_eq!(order.order_id.as_deref(), Some("75"));
        assert_eq!(order.query_token.as_deref(), Some("same+query"));
        assert_eq!(order.confirm.pay_money, 0);
        assert!(matches!(&order.state, OrderState::AwaitingPayment(_)));
        assert_eq!(trace.notices.len(), 1);
        assert!(trace.notices[0].0.contains("支付0.00元"));
        assert!(trace.notices[0].1.as_deref().unwrap().ends_with("order_id=75"));

        for prepare in requests(&trace, Endpoint::Prepare) {
            assert_amount_and_target(prepare, 31, None);
        }
        let confirm_tokens: Vec<_> = requests(&trace, Endpoint::Confirm).map(|request| query(request, "token")).collect();
        assert_eq!(confirm_tokens, [Some("before-confirm"), Some("before-create"), Some("refreshed")]);
        for (index, create) in requests(&trace, Endpoint::Create).enumerate() {
            assert_amount_and_target(create, 31, Some(0));
            let token = if index == 0 { "before-create" } else { "refreshed" };
            assert_eq!(create.body.as_ref().unwrap()["token"], token);
            assert_eq!(query(create, "ptoken").unwrap(), format!("{token}-p"));
        }
        let checked: Vec<_> = requests(&trace, Endpoint::Check).collect();
        assert_eq!(checked.len(), 3);
        assert!(checked.iter().all(|request| query(request, "token") == Some("same+query")));
        assert_eq!(checked.iter().map(|request| query(request, "orderId")).collect::<Vec<_>>(), [None, None, Some("75")]);
    }

    #[tokio::test]
    async fn direct_mode_keeps_create_budget_across_refresh() {
        let mut responses = vec![
            prepare("before-create"), confirm(123),
            rejection(Endpoint::Create, 100050),
            prepare("refreshed"), confirm(123),
        ];
        responses.extend((0..59).map(|_| rejection(Endpoint::Create, 211)));
        let trace = Fixture::new(1).run(responses).await;
        assert!(matches!(&trace.finished.outcome, TaskOutcome::Ended(reason) if reason.contains("60 次")));
        assert_eq!(trace.finished.info.attempts, 60);
        assert!(trace.finished.info.order.is_none());
        assert!(trace.notices.is_empty());
        assert_eq!(requests(&trace, Endpoint::Prepare).count(), 2);
        assert_eq!(requests(&trace, Endpoint::Confirm).count(), 2);
        let creates: Vec<_> = requests(&trace, Endpoint::Create).collect();
        assert_eq!(creates.len(), 60);
        assert_eq!(creates[1].body.as_ref().unwrap()["token"], "refreshed");
        for create in creates {
            assert_amount_and_target(create, 31, Some(123));
        }
    }

    #[tokio::test]
    async fn monitor_mode_visits_next_ticket_in_the_same_screen_after_limit() {
        let mut responses = vec![
            (Endpoint::Project, Reply::Json(json!({"errno": 0, "data": project_data(&[31, 32])}))),
            prepare("initial"), confirm(123), rejection(Endpoint::Create, 900002),
            prepare("refreshed"), confirm(123),
        ];
        responses.extend((0..28).map(|_| rejection(Endpoint::Create, 211)));
        let retry_after = Duration::from_secs(1);
        let (throttled, last_create, release_create) = hold(Reply::JsonWithRetryAfter(
            json!({"errno": 100001, "msg": "测试明确限速拒绝"}),
            retry_after,
        ));
        let (held_prepare, next_prepare, release_prepare) = hold(prepare("next-sku").1);
        responses.extend([
            (Endpoint::Create, throttled),
            (Endpoint::Prepare, held_prepare),
            confirm(123),
            created(Some(76), Some("query")),
            payable(Some(76)),
        ]);

        let (trace, ()) = tokio::join!(Fixture::new(2).run(responses), async {
            tokio::time::timeout(Duration::from_secs(20), last_create.notified()).await.unwrap();
            let rejected_at = tokio::time::Instant::now();
            release_create.notify_one();
            tokio::time::timeout(Duration::from_secs(5), next_prepare.notified()).await.unwrap();
            assert!(rejected_at.elapsed() >= retry_after);
            release_prepare.notify_one();
        });
        assert!(matches!(trace.finished.outcome, TaskOutcome::Succeeded));
        assert_eq!(trace.finished.info.attempts, 31);
        assert!(trace.notices.is_empty());
        let order = trace.finished.info.order.as_ref().unwrap();
        assert_eq!((order.project_id.as_str(), order.screen_id.as_str(), order.ticket_id.as_str()), ("11", "21", "32"));
        assert_eq!(order.confirm.pay_money, 123);
        let OrderState::AwaitingPayment(payment) = &order.state else { panic!("应进入待支付") };
        assert!(PaymentNotice::new(order, payment).message.contains("支付1.23元"));
        for (index, prepare) in requests(&trace, Endpoint::Prepare).enumerate() {
            assert_amount_and_target(prepare, if index < 2 { 31 } else { 32 }, None);
        }
        for (index, create) in requests(&trace, Endpoint::Create).enumerate() {
            assert_amount_and_target(create, if index < 30 { 31 } else { 32 }, Some(123));
        }
    }

    #[tokio::test]
    async fn unknown_create_response_keeps_order_fact_and_never_sends_again() {
        let (_, code_unknown) = rejection(Endpoint::Create, 737);
        for reply in [
            Reply::Http(StatusCode::TOO_MANY_REQUESTS, Some(Duration::from_secs(3))),
            Reply::Http(StatusCode::BAD_GATEWAY, None),
            Reply::TransportFailure,
            Reply::ReadBodyFailure,
            Reply::Raw(b"{bad json"),
            Reply::Json(json!({"data": {}})),
            code_unknown,
        ] {
            let trace = Fixture::new(1).run(vec![prepare("test"), confirm(0), (Endpoint::Create, reply)]).await;
            assert!(matches!(trace.finished.outcome, TaskOutcome::AwaitingConfirmation(_)));
            assert_eq!(trace.finished.info.attempts, 1);
            let order = trace.finished.info.order.as_ref().unwrap();
            assert!(order.order_id.is_none());
            assert!(order.query_token.is_none());
            assert!(matches!(&order.state, OrderState::PendingConfirmation));
            assert!(trace.notices.is_empty());
        }
    }

    #[tokio::test]
    async fn accepted_create_preserves_partial_identifiers_and_queries_at_most_thirty_times() {
        for order_id in [Some(75), None] {
            let trace = Fixture::new(1).run(vec![prepare("test"), confirm(0), created(order_id, None)]).await;
            assert!(matches!(trace.finished.outcome, TaskOutcome::AwaitingConfirmation(_)));
            assert_eq!(requests(&trace, Endpoint::Check).count(), 0);
            assert_eq!(trace.finished.info.order.unwrap().order_id, order_id.map(|id| id.to_string()));
        }

        let mut responses = vec![prepare("test"), confirm(0), created(Some(75), Some("query"))];
        responses.extend((0..30).map(|index| (Endpoint::Check, match index % 3 {
            0 => Reply::Http(StatusCode::SERVICE_UNAVAILABLE, None),
            1 => Reply::Raw(b"{"),
            _ => Reply::Json(json!({"errno": 87654, "msg": "测试订单尚未完成"})),
        })));
        let trace = Fixture::new(1).run(responses).await;
        assert!(matches!(&trace.finished.outcome, TaskOutcome::AwaitingConfirmation(reason) if reason.contains("30 次")));
        assert_eq!(trace.finished.info.attempts, 1);
        let checks: Vec<_> = requests(&trace, Endpoint::Check).collect();
        assert_eq!(checks.len(), 30);
        assert!(checks.iter().all(|request| query(request, "token") == Some("query") && query(request, "orderId") == Some("75")));
        let order = trace.finished.info.order.as_ref().unwrap();
        assert!(matches!(&order.state, OrderState::PendingConfirmation));
        assert_eq!(order.order_id.as_deref(), Some("75"));
        assert!(trace.notices.is_empty());
    }

    #[tokio::test]
    async fn payable_query_without_order_id_does_not_make_a_detail_link() {
        let mut fixture = Fixture::new(1);
        fixture.request.biliticket.push_self.enabled = true;
        let trace = fixture.run(vec![prepare("test"), confirm(123), created(None, Some("query")), payable(None)]).await;
        assert!(matches!(trace.finished.outcome, TaskOutcome::Succeeded));
        let order = trace.finished.info.order.unwrap();
        assert!(matches!(order.state, OrderState::AwaitingPayment(_)));
        assert!(order.order_id.is_none());
        assert_eq!(trace.notices.len(), 1);
        assert!(trace.notices[0].1.is_none());
    }

    #[tokio::test]
    async fn failed_challenges_confirm_and_refresh_have_separate_finite_budgets() {
        let mut responses = vec![clock(SALE_BEGIN)];
        // 无风控参数的挑战会在现有验证码入口立即失败，不触发模型加载或网络。
        responses.extend((0..5).map(|_| rejection(Endpoint::Prepare, -401)));
        let trace = Fixture::new(0).run(responses).await;
        assert!(matches!(&trace.finished.outcome, TaskOutcome::Ended(reason) if reason.contains("5 次")));
        assert_eq!(requests(&trace, Endpoint::Prepare).count(), 5);
        assert_eq!(trace.finished.info.attempts, 0);

        let mut responses = vec![prepare("test")];
        responses.extend((0..4).map(|_| (Endpoint::Confirm, Reply::Http(StatusCode::SERVICE_UNAVAILABLE, None))));
        let trace = Fixture::new(1).run(responses).await;
        assert!(matches!(&trace.finished.outcome, TaskOutcome::Ended(reason) if reason.contains("4 次")));
        assert_eq!(requests(&trace, Endpoint::Confirm).count(), 4);
        assert_eq!(trace.finished.info.attempts, 0);

        let responses = (0..6).flat_map(|_| [
            prepare("test"), confirm(0), rejection(Endpoint::Create, 100041)
        ]).collect();
        let trace = Fixture::new(1).run(responses).await;
        assert!(matches!(&trace.finished.outcome, TaskOutcome::Ended(reason) if reason.contains("刷新已达上限")));
        assert_eq!(trace.finished.info.attempts, 6);
        assert_eq!(requests(&trace, Endpoint::Prepare).count(), 6);
    }

    #[tokio::test]
    async fn confirmed_target_mismatch_and_local_buyer_failure_never_create() {
        let (_, Reply::Json(mut wrong_confirmation)) = confirm(0) else { unreachable!() };
        wrong_confirmation["data"]["count"] = json!(1);
        let trace = Fixture::new(1).run(vec![prepare("test"), (Endpoint::Confirm, Reply::Json(wrong_confirmation))]).await;
        assert!(matches!(&trace.finished.outcome, TaskOutcome::Ended(reason) if reason.contains("数量")));
        assert_eq!(trace.finished.info.attempts, 0);

        let mut fixture = Fixture::new(1);
        fixture.request.buyer_info.clear();
        let trace = fixture.run(vec![prepare("test"), confirm(0)]).await;
        assert!(matches!(&trace.finished.outcome, TaskOutcome::Ended(reason) if reason.contains("购票人")));
        assert_eq!(trace.finished.info.attempts, 0);
        assert!(trace.finished.info.order.is_none());
    }

    #[tokio::test]
    async fn monitor_rejects_invalid_shared_parameters_before_scanning() {
        let mut fixture = Fixture::new(2);
        fixture.request.count = 0;
        let trace = fixture.run(Vec::new()).await;
        assert!(matches!(&trace.finished.outcome, TaskOutcome::Ended(reason) if reason.contains("数量")));
        assert!(trace.requests.is_empty());

        let mut fixture = Fixture::new(2);
        fixture.request.project_id = "0".into();
        let trace = fixture.run(Vec::new()).await;
        assert!(matches!(&trace.finished.outcome, TaskOutcome::Ended(reason) if reason.contains("项目 ID")));
        assert!(trace.requests.is_empty());
    }

    fn hold(reply: Reply) -> (Reply, Arc<Notify>, Arc<Notify>) {
        let entered = Arc::new(Notify::new());
        let release = Arc::new(Notify::new());
        let reply = Reply::Wait { entered: entered.clone(), release: release.clone(), reply: Box::new(reply) };
        (reply, entered, release)
    }

    #[tokio::test]
    async fn cancellation_preserves_create_and_interrupts_read_only_stages() {
        for stage in [Endpoint::Confirm, Endpoint::Create, Endpoint::Check] {
            let fixture = Fixture::new(1);
            let registry = fixture.guard.registry.clone();
            let task_id = fixture.request.task_id.clone();
            let mut responses = vec![prepare("test")];
            let reply = match stage {
                Endpoint::Confirm => confirm(0).1,
                Endpoint::Create => {
                    responses.push(confirm(0));
                    created(Some(75), Some("query")).1
                }
                Endpoint::Check => {
                    responses.extend([confirm(0), created(Some(75), Some("query"))]);
                    payable(Some(75)).1
                }
                _ => unreachable!(),
            };
            let (held, entered, release) = hold(reply);
            responses.push((stage, held));

            let (trace, ()) = tokio::join!(fixture.run(responses), async {
                tokio::time::timeout(Duration::from_secs(2), entered.notified()).await.unwrap();
                {
                    let registry = registry.lock().unwrap();
                    let entry = &registry[&task_id];
                    assert_eq!(entry.info_seed.attempts, u32::from(stage != Endpoint::Confirm));
                    if stage == Endpoint::Create {
                        let order = entry.info_seed.order.as_ref().unwrap();
                        assert!(order.order_id.is_none());
                        assert!(order.query_token.is_none());
                    }
                    entry.cancel.send_replace(true);
                }
                if stage == Endpoint::Create {
                    release.notify_one();
                }
            });
            assert!(matches!(trace.finished.outcome, TaskOutcome::Stopped));
            assert!(trace.notices.is_empty());
            if stage == Endpoint::Confirm {
                assert!(trace.finished.info.order.is_none());
            } else {
                let order = trace.finished.info.order.as_ref().unwrap();
                assert!(matches!(&order.state, OrderState::PendingConfirmation));
                assert_eq!(order.order_id.as_deref(), Some("75"));
                assert_eq!(order.query_token.as_deref(), Some("query"));
            }
        }
    }

    #[tokio::test]
    async fn timed_wait_cancellation_never_starts_prepare() {
        let fixture = Fixture::new(0);
        let registry = fixture.guard.registry.clone();
        let task_id = fixture.request.task_id.clone();
        let (clock, entered, release) = hold(clock(SALE_BEGIN - 60).1);
        let (trace, ()) = tokio::join!(fixture.run(vec![(Endpoint::Clock, clock)]), async {
            tokio::time::timeout(Duration::from_secs(2), entered.notified()).await.unwrap();
            release.notify_one();
            tokio::time::timeout(Duration::from_secs(2), async {
                loop {
                    if registry.lock().unwrap()[&task_id].info_seed.phase == GrabPhase::WaitingSale {
                        break;
                    }
                    tokio::task::yield_now().await;
                }
            }).await.unwrap();
            registry.lock().unwrap()[&task_id].cancel.send_replace(true);
        });
        assert!(matches!(trace.finished.outcome, TaskOutcome::Stopped));
        assert_eq!(trace.requests.len(), 1);
        assert_eq!(trace.finished.info.attempts, 0);
    }

    #[tokio::test]
    async fn cancelled_inflight_create_timeout_keeps_unknown_order() {
        let fixture = Fixture::new(1);
        let registry = fixture.guard.registry.clone();
        let task_id = fixture.request.task_id.clone();
        let (held, entered, _release) = hold(created(Some(75), Some("query")).1);
        let (trace, ()) = tokio::join!(
            fixture.run(vec![prepare("test"), confirm(0), (Endpoint::Create, held)]),
            async {
                tokio::time::timeout(Duration::from_secs(2), entered.notified()).await.unwrap();
                registry.lock().unwrap()[&task_id].cancel.send_replace(true);
            },
        );
        assert!(matches!(trace.finished.outcome, TaskOutcome::Stopped));
        assert_eq!(trace.finished.info.attempts, 1);
        let order = trace.finished.info.order.unwrap();
        assert!(matches!(order.state, OrderState::PendingConfirmation));
        assert!(order.order_id.is_none());
    }

    #[tokio::test]
    async fn abnormal_exit_at_send_boundary_retains_unresolved_order() {
        let fixture = Fixture::new(1);
        let finished = fixture.guard.finished.clone();
        let registry = fixture.guard.registry.clone();
        // 脚本省略 create 响应，在实际构造和记录发送边界后触发夹具 panic。
        let task = tokio::spawn(with_responses(
            vec![prepare("test"), confirm(0)],
            run_registered_grab_task(fixture.guard, fixture.request),
        ));
        assert!(task.await.unwrap_err().is_panic());
        assert!(registry.lock().unwrap().is_empty());
        let finished = finished.lock().unwrap();
        assert_eq!(finished.len(), 1);
        assert!(matches!(finished[0].outcome, TaskOutcome::AwaitingConfirmation(_)));
        assert_eq!(finished[0].info.attempts, 1);
        let order = finished[0].info.order.as_ref().unwrap();
        assert!(matches!(&order.state, OrderState::PendingConfirmation));
        assert!(order.order_id.is_none());
    }
}
