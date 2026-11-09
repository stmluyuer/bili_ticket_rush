use std::sync::Arc;

use reqwest::header::HeaderValue;
use reqwest::{header, Client};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::cookie_manager::CookieManager;
use crate::account::Account;
use crate::push::PushConfig;
use crate::utility::CustomConfig;

//成功下单结构体
#[derive(Clone,Debug,Deserialize,Serialize)]
pub struct SubmitOrderResult{
    #[serde(rename = "orderId")]
    pub order_id: i128,
    #[serde(rename = "orderCreateTime")]
    pub order_create_time: i64,
    #[serde(rename = "token")]
    pub order_token: String,
}

#[derive(Clone,Debug,Deserialize,Serialize)]
pub struct CheckFakeResult{
    #[serde(default)]
    pub errno: i32,
    #[serde(default)]
    pub code: i32,
    #[serde(default)]
    pub errtag: i32,
    #[serde(default)]
    pub msg: String,
    #[serde(default)]
    pub message: String,
    pub data: CheckFakeResultParam,
}
#[derive(Clone,Debug,Deserialize,Serialize)]
pub struct CheckFakeResultParam{
    #[serde(rename = "payParam")]
    pub pay_param: CheckFakeResultData,
}
#[derive(Clone,Debug,Deserialize,Serialize)]
pub struct CheckFakeResultData{
    pub sign : String,
    pub code_url : String,
}

#[derive(Clone,Debug,Deserialize,Serialize)]
pub struct ConfirmTicketInfo{
    pub name: String,
    pub count: i32,
    pub price: i64,
}

//确认订单结构体
#[derive(Clone,Debug,Deserialize,Serialize)]
pub struct ConfirmTicketResult{
    pub count : i32,
    pub pay_money: i64,
    pub project_name: String,
    pub screen_name: String,
    pub ticket_info: ConfirmTicketInfo,
    
}

//获取token响应结构体

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct TokenRiskParam {
    #[serde(default)]
    pub code : i32,
    
    #[serde(default)]
    pub message: String,
    
    pub mid : Option<String>,
    pub decision_type : Option<String>,
    pub buvid : Option<String>,
    pub ip : Option<String>,
    pub scene: Option<String>,
    pub ua: Option<String>,
    pub v_voucher: Option<String>,
    pub risk_param: Option<Value>,
}

#[derive(Clone,Debug)]
pub struct BilibiliTicket{
    pub uid : i64, //UID
    pub method : u8,
    pub ua : String,
    pub config: CustomConfig,
    pub account: Account,
    pub push_self : PushConfig,
    pub status_delay : usize,
    pub captcha_use_type: usize,    //选择的验证码方式
    pub cookie_manager: Option<Arc<CookieManager>>,

    //抢票相关
    pub project_id: String,
    pub referer: String,
    pub screen_id: String,
    pub id_bind: usize, //是否绑定

    pub project_info : Option<TicketInfo>, //项目详情
    pub all_buyer_info: Option<BuyerInfoData>, //所有购票人信息
    pub buyer_info: Option<Vec<BuyerInfo>>,  //购买人信息（实名票）

    pub no_bind_buyer_info: Option<NoBindBuyerInfo>, //不实名制购票人信息

    pub select_ticket_id : Option<String>,

    pub pay_money: Option<i64>, //支付金额
    pub count: Option<i32>, //购买数量
    pub device_id: String, //设备id

}

impl BilibiliTicket{
    pub fn new(
        
        method: &u8,
        ua: &String,
        config: &CustomConfig,
        account: &Account,
        push_self: &PushConfig,
        status_delay: &usize,
        project_id : &str,
        referer : &str,


    ) -> Self{
        let mut finally_ua = String::new();
        if config.custom_ua != "" {
            log::info!("使用自定义UA：{}",config.custom_ua);
            finally_ua.push_str(&config.custom_ua);
        }else{
            log::info!("使用默认UA：{}",ua);
            finally_ua.push_str(ua);
        }
        let mut headers = header::HeaderMap::new();
        match HeaderValue::from_str(&account.cookie){
            Ok(ck_value) => {
                headers.insert(header::COOKIE, ck_value);
                match HeaderValue::from_str(&finally_ua){
                    Ok(ua_value) => {
                        headers.insert(header::USER_AGENT,ua_value);
                    }
                    Err(e) => {
                        log::error!("client插入ua失败！原因：{}",e);
                    }
                }
                
            }
            Err(e) => {
                log::error!("cookie设置失败！原因：{:?}",e);
            }

        }
        

        let client = match Client::builder()
                                    .cookie_store(true)
                                    .user_agent(ua)
                                    .default_headers(headers)
                                    
                                    .build(){
                                        Ok(client) => client,
                                        Err(e) => {
                                            log::error!("初始化client失败！，原因：{:?}",e);
                                            Client::new()
                                        }
                                    };
        let captcha_type = config.captcha_mode;      

       
           
        let new = Self{
            uid: account.uid.clone(),
            method: method.clone(),
            ua: ua.clone(),
            config: config.clone(),
            account: account.clone(),
            push_self: push_self.clone(),
            status_delay: *status_delay,
            captcha_use_type: captcha_type,
            cookie_manager: None,
            project_id: project_id.to_string(),
            referer: referer.to_string(),
            screen_id: String::new(),
            project_info: None,
            buyer_info: None,
            all_buyer_info: None,
            no_bind_buyer_info: None,
            select_ticket_id: None,
            pay_money: None,
            count: None,
            device_id: "".to_string(),
            id_bind: 999,

        };
        log::debug!("新建抢票对象：{:?}",new);
        new

    }

}

#[derive(Clone,Debug,Deserialize,Serialize)]
pub struct TicketInfo {
    pub id: Option<i32>,
    pub name: Option<String>,
    pub is_sale: Option<usize>,
    pub start_time: Option<i64>,
    pub end_time: Option<i64>,
    pub pick_seat: Option<usize>, //0:不选座 1:选座
    pub project_type: Option<usize>, //未知作用，bw2024是type1
    #[serde(default)]
    pub express_fee: Option<usize>, //快递费
    pub sale_begin: Option<i64>, //开售时间
    /// 只保留本次在线响应的原始项目开售时间，缓存无法证明其来源。
    #[serde(skip)]
    pub raw_sale_begin: Option<i64>,
    pub sale_end: Option<i64>, //截止时间
    pub count_down: Option<i64>, //倒计时（可能有负数）
    pub screen_list: Vec<ScreenInfo>, //场次列表
    pub sale_flag_number: Option<usize>, //售票标志位
    #[serde(default)]
    pub sale_flag: String, //售票状态
    pub is_free: Option<bool>,
    pub performance_desc: Option<DescribeList>, //基础信息
    pub id_bind: Option<usize>, //是否绑定
    #[serde(rename = "hotProject")]
    pub hot_project: Option<bool>, //是否热门项目

    


}

impl TicketInfo {
    /// 只在指定场次和票档内选时间，项目级回退必须来自在线原始字段。
    pub fn target_sale_begin(&self, screen_id: usize, ticket_id: usize) -> Option<i64> {
        let screen = self.screen_list.iter().find(|screen| screen.id == Some(screen_id))?;
        let ticket = screen.ticket_list.iter().find(|ticket| ticket.id == Some(ticket_id))?;
        ticket.saleStart.and_then(|time| i64::try_from(time).ok()).filter(|time| *time > 0)
            .or_else(|| i64::try_from(screen.sale_start).ok().filter(|time| *time > 0))
            .or(self.raw_sale_begin.filter(|time| *time > 0))
    }

    /// 为项目展示补齐开售时间；该合成值不参与指定目标的定时回退。
    pub fn fill_missing_sale_begin(&mut self) {
        if self.sale_begin.is_some() {
            return; // 已有值，无需处理
        }

        let mut candidate_times: Vec<i64> = Vec::new();

        // 从 screen_list 中提取时间
        for screen in &self.screen_list {
            // 演出开始时间不是开售时间。
            if screen.sale_start > 0 {
                candidate_times.push(screen.sale_start as i64);
            }

            for ticket in &screen.ticket_list {
                if let Some(t) = ticket.saleStart {
                    if t > 0 {
                        candidate_times.push(t as i64);
                    }
                }
            }
        }

        // 选择最早的时间作为 sale_begin
        if let Some(&earliest) = candidate_times.iter().min() {
            self.sale_begin = Some(earliest);
            log::info!("sale_begin 缺失，已从 screen_list 提取最早时间: {}", earliest);
        } else {
            log::warn!("sale_begin 缺失且无法从 screen_list 提取有效时间");
        }
    }

    /// 校验项目信息中后续流程必需的字段是否为空
    /// 如果关键字段缺失则记录错误日志并返回 Err
    pub fn validate(&self) -> Result<(), String> {
        let mut missing = Vec::new();

        if self.id.is_none() {
            missing.push("id(项目ID)");
        }
        if self.name.is_none() {
            missing.push("name(项目名称)");
        }
        if self.id_bind.is_none() {
            missing.push("id_bind(实名绑定标识)");
        }
        if self.hot_project.is_none() {
            missing.push("hotProject(热门项目标识)");
        }
        if self.screen_list.is_empty() {
            missing.push("screen_list(场次列表为空)");
        }

        if missing.is_empty() {
            Ok(())
        } else {
            let msg = format!(
                "项目信息校验失败，以下必需字段缺失或为空: [{}]",
                missing.join(", ")
            );
            log::error!("{}", msg);
            Err(msg)
        }
    }
}

#[derive(Clone,Debug,Deserialize,Serialize)]
pub struct ScreenInfo {
    #[serde(default)]
    pub sale_flag: SaleFlag,
    pub id: Option<usize>,
    pub start_time: Option<usize>,
    pub name: Option<String>,
    pub ticket_type: Option<usize>,
    pub screen_type: Option<usize>,
    pub delivery_type: Option<usize>,
    pub pick_seat: Option<usize>,
    pub ticket_list: Vec<ScreenTicketInfo>, //当日票种类列表
    pub clickable: bool, //是否可点（可售）
    pub sale_end: usize, //截止时间
    pub sale_start: usize, //开售时间
    pub sale_flag_number: usize, //售票标志位
    pub show_date: String, //展示信息
    
}

#[derive(Clone,Debug,Deserialize,Serialize)]
pub struct SaleFlag {
    #[serde(default)]
    pub number: usize,
    #[serde(default)]
    pub display_name: String,
}

impl Default for SaleFlag {
    fn default() -> Self {
        Self {
            number: 0,
            display_name: "未知状态".to_string(),
        }
    }
}

#[derive(Clone,Debug,Deserialize,Serialize)]
pub struct ScreenTicketInfo{
    pub saleStart : Option<usize>, //开售时间(时间戳)   eg：1720260000
    pub saleEnd : Option<usize>, //截止时间(时间戳)
    pub id: Option<usize>, //票种id
    pub project_id: Option<usize>, //项目id
    pub price: Option<usize>, //票价(分)
    pub desc: Option<String>, //票种描述
    pub sale_start: Option<String>, //开售时间（字符串）    eg:2024-07-06 18:00:00
    pub sale_end: Option<String>, //截止时间（字符串）
    pub r#type: Option<usize>, //类型 关键词替换，对应”type“
    pub sale_type: Option<usize>, //销售状态
    pub is_sale: Option<usize>, //是否销售？0是1否
    pub num: Option<usize>, //数量
    pub sale_flag: SaleFlag, //售票状态
    pub clickable: Option<bool>, //是否可点（可售）
    pub sale_flag_number: Option<usize>, //售票标志位
    pub screen_name: Option<String>, //场次名称


}

#[derive(Clone,Debug,Deserialize,Serialize)]
pub struct DescribeList{
    pub r#type: u8,  // 使用 r# 前缀处理 Rust 关键字
    pub list: Vec<ModuleItem>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ModuleItem {
    pub module: String,
    
    // details 可能是字符串或数组，使用 serde_json::Value 处理多态
    #[serde(default)]
    pub details: Value,
    
    // 可选字段
    #[serde(default)]
    pub module_name: Option<String>,
}

// 为 base_info 模块中的详情项创建结构体
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BaseInfoItem {
    pub title: String,
    pub content: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct InfoResponse{
    #[serde(default)]
    pub errno: i32,
    #[serde(default)]
    pub errtag: i32,
    #[serde(default)]
    pub msg: String,
    #[serde(default)]
    pub code: i32,
    #[serde(default)]
    pub message: String,
    pub data: TicketInfo,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BuyerInfo{
    pub id: i64,
    pub uid: i64,
    pub personal_id: String,
    pub name: String,
    pub tel: String,
    pub id_type: i64,
    pub is_default: i64,
    #[serde(default)]
    pub id_card_front: String,
    #[serde(default)]
    pub id_card_back: String,
    #[serde(default)]
    pub verify_status: i64,
    #[serde(default)]
    pub isBuyerInfoVerified: bool,
    #[serde(default)]
    pub isBuyerValid: bool,
    

}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BuyerInfoResponse{
    #[serde(default)]
    pub errno: i32,
    #[serde(default)]
    pub errtag: i32,
    #[serde(default)]
    pub msg: String,
    #[serde(default)]
    pub code: i32,
    #[serde(default)]
    pub message: String,
    pub data: BuyerInfoData,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BuyerInfoData{
    pub list: Vec<BuyerInfo>,
    
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct NoBindBuyerInfo {
    pub name: String,
    pub tel: String,
    pub uid: i64,
}

#[cfg(test)]
mod sale_time_tests {
    use super::*;
    use serde_json::json;

    fn project() -> TicketInfo {
        serde_json::from_value(json!({
            "id": 1, "name": "测试项目", "sale_begin": 100, "id_bind": 1, "hotProject": false,
            "screen_list": [
                {"id": 10, "start_time": 5, "sale_start": 200, "sale_end": 900,
                 "clickable": true, "sale_flag_number": 2, "show_date": "",
                 "ticket_list": [
                    {"id": 11, "saleStart": 300, "sale_flag": {}},
                    {"id": 12, "sale_flag": {}}
                 ]},
                {"id": 20, "start_time": 6, "sale_start": 0, "sale_end": 900,
                 "clickable": true, "sale_flag_number": 2, "show_date": "",
                 "ticket_list": [{"id": 21, "sale_flag": {}}]}
            ]
        })).unwrap()
    }

    #[test]
    fn selected_target_uses_its_own_sale_time() {
        let mut project = project();
        project.raw_sale_begin = project.sale_begin;
        assert_eq!(project.target_sale_begin(10, 11), Some(300));
        assert_eq!(project.target_sale_begin(10, 12), Some(200));
        assert_eq!(project.target_sale_begin(20, 21), Some(100));
        assert_eq!(project.target_sale_begin(20, 11), None);
        assert_eq!(project.target_sale_begin(99, 21), None);
    }

    #[test]
    fn cache_cannot_restore_online_project_time_provenance() {
        let mut online = project();
        online.raw_sale_begin = online.sale_begin;
        let cached: TicketInfo = serde_json::from_str(&serde_json::to_string(&online).unwrap()).unwrap();
        assert_eq!(cached.sale_begin, Some(100));
        assert_eq!(cached.raw_sale_begin, None);
        assert_eq!(cached.target_sale_begin(10, 11), Some(300));
        assert_eq!(cached.target_sale_begin(10, 12), Some(200));
        assert_eq!(cached.target_sale_begin(20, 21), None);
    }

    #[test]
    fn synthesized_time_and_performance_time_do_not_schedule_another_target() {
        let mut project = project();
        project.sale_begin = None;
        project.fill_missing_sale_begin();
        assert_eq!(project.sale_begin, Some(200));
        assert_eq!(project.target_sale_begin(20, 21), None);
        project.screen_list[0].sale_start = 0;
        project.screen_list[0].ticket_list[0].saleStart = None;
        project.sale_begin = None;
        project.fill_missing_sale_begin();
        assert_eq!(project.sale_begin, None);
        assert!(project.validate().is_ok());
    }
}
