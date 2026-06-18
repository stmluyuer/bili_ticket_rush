/* use std::collections::HashMap;


pub const fn enabled() -> bool {
    true
}


#[cfg(not(feature = "tls-impersonate"))]
pub async fn post_json(
    url: &str,
    cookie_header: &str,
    headers: &HashMap<&str, &str>,
    body: &serde_json::Value,
) -> Result<(u16, String), String> {
    use once_cell::sync::OnceCell;

    
    static H2_CLIENT: OnceCell<reqwest::Client> = OnceCell::new();
    let client = H2_CLIENT.get_or_try_init(|| {
        reqwest::Client::builder()
            // 对应 httpx 的 http2=True：强制 HTTP/2（B站 createV2 风控要求 h2，否则 100001）。
            .http2_prior_knowledge()
            .build()
            .map_err(|e| format!("构建 HTTP/2 客户端失败: {}", e))
    })?;

    let mut rb = client.post(url);
    if !cookie_header.is_empty() {
        rb = rb.header("Cookie", cookie_header);
    }
    for (k, v) in headers.iter() {
        rb = rb.header(*k, *v);
    }
    let resp = rb
        .json(body)
        .send()
        .await
        .map_err(|e| format!("HTTP/2 请求失败: {}", e))?;
    let status = resp.status().as_u16();
    let text = resp
        .text()
        .await
        .map_err(|e| format!("读取响应失败: {}", e))?;
    Ok((status, text))
}

// ===== 可选实现：wreq（Chrome JA3 + HTTP/2 指纹伪装），需 tls-impersonate feature =====
#[cfg(feature = "tls-impersonate")]
pub async fn post_json(
    url: &str,
    cookie_header: &str,
    headers: &HashMap<&str, &str>,
    body: &serde_json::Value,
) -> Result<(u16, String), String> {
    use once_cell::sync::OnceCell;

    static CLIENT: OnceCell<wreq::Client> = OnceCell::new();
    let client = CLIENT.get_or_try_init(|| {
        wreq::Client::builder()
            .emulation(wreq_util::Emulation::Chrome137)
            .build()
            .map_err(|e| format!("构建指纹客户端失败: {}", e))
    })?;

    let mut rb = client.post(url);
    if !cookie_header.is_empty() {
        rb = rb.header("Cookie", cookie_header);
    }
    for (k, v) in headers.iter() {
        rb = rb.header(*k, *v);
    }
    let resp = rb
        .json(body)
        .send()
        .await
        .map_err(|e| format!("指纹请求失败: {}", e))?;
    let status = resp.status().as_u16();
    let text = resp
        .text()
        .await
        .map_err(|e| format!("读取指纹响应失败: {}", e))?;
    Ok((status, text))
}
  */