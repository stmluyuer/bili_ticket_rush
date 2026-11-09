use common::{cookie_manager::CookieManager, show_orderlist::OrderResponse};
use serde::Deserialize;
use std::sync::Arc;

use crate::api::{purchase_data, send_purchase};

pub async fn get_orderlist(cookie_manager: Arc<CookieManager>) -> Result<OrderResponse, String> {
    let request = cookie_manager.get(
        "https://show.bilibili.com/api/ticket/ordercenter/ticketList?page=0&page_size=10",
    ).await;
    let (value, retry_after) = send_purchase(request).await.map_err(|error| error.to_string())?;
    // 原账号核对必须区分查询失败与确实没有订单。
    purchase_data(&value, retry_after).map_err(|error| error.to_string())?;
    OrderResponse::deserialize(value).map_err(|error| format!("解析订单列表失败：{error}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::purchase_test::{with_responses, Endpoint, Reply};
    use serde_json::json;

    #[tokio::test]
    async fn original_account_orders_distinguish_failure_from_empty_list() {
        let manager = Arc::new(CookieManager::from_client(Arc::new(reqwest::Client::new()), "DedeUserID=42"));
        let empty = json!({"errno": 0, "errtag": "", "msg": "", "data": {"total": 0, "list": []}});
        let mut failed = empty.clone();
        failed["errno"] = 100001.into();
        failed["msg"] = "请稍后重试".into();
        let mut missing = empty.clone();
        missing.as_object_mut().unwrap().remove("errno");
        let mut contradictory = empty.clone();
        contradictory["code"] = 100001.into();
        for reply in [failed, missing, contradictory] {
            let (result, requests) = with_responses(
                [(Endpoint::Orders, Reply::Json(reply))],
                get_orderlist(manager.clone()),
            ).await;
            assert!(result.is_err());
            assert_eq!(requests.len(), 1);
        }
        let (result, _) = with_responses(
            [(Endpoint::Orders, Reply::Json(empty))], get_orderlist(manager.clone()),
        ).await;
        assert!(result.unwrap().data.list.is_empty());
        let (result, _) = with_responses(
            [(Endpoint::Orders, Reply::Http(reqwest::StatusCode::SERVICE_UNAVAILABLE, None))],
            get_orderlist(manager),
        ).await;
        assert!(result.is_err());
    }
}
