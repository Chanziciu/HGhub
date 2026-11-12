//! 网络层：与上游站点对话。
//!
//! 站点把全部 `/api/**` 调用收敛成 `POST /api/edge/request` 的信封，
//! 我们只实现这一个入口，其余接口都是「填 path」。
//!
//! # 代理（用户反馈「浏览器能开、HGhub 打不开」的根因就在这里）
//!
//! 客户端的流量**全部**走本文件的 reqwest，跟 WebView2 无关，所以它既不共享
//! 浏览器的代理设置，也不共享浏览器的证书库：
//!
//! * reqwest 只有开了 `system-proxy` feature 才会读 Windows 系统代理
//!   （`HKCU\...\Internet Settings` 的 `ProxyEnable`/`ProxyServer`/`ProxyOverride`）。
//!   本 crate 用 `default-features = false`，早先漏了这个 feature ⇒ 永远直连，
//!   只有 TUN/虚拟网卡这种网络层接管才「看起来能用」。
//! * 即使开了 `system-proxy`，PAC（`AutoConfigURL`）也不被支持，只认固定代理。
//! * Android 没有注册表可读，系统「Wi-Fi 代理」同样不被 reqwest 采用；
//!   安卓上只有 VPN/TUN 模式（VpnService 抓全流量）才有效。
//!
//! 排查手段：环境变量 `HGHUB_PROXY=http://127.0.0.1:7897` 可强制指定代理，
//! `net_diag` 命令会输出 DNS/HTTP/环境变量快照。

use base64::engine::general_purpose::STANDARD as B64;
use base64::Engine;
use once_cell::sync::Lazy;
use serde_json::{json, Value};
use std::time::{Duration, Instant};
use std::collections::HashMap;
use std::sync::Mutex;

// Playback credentials stay in memory and are scoped to the CDN origin supplied by the API.
static CDN_AUTH: Lazy<Mutex<HashMap<String, Value>>> = Lazy::new(|| Mutex::new(HashMap::new()));

pub fn register_cdn(url: &str, cdn: &Value) {
    if let Ok(url) = url::Url::parse(url) {
        let mut entries = CDN_AUTH.lock().unwrap();
        if entries.len() >= 32 { entries.clear(); }
        entries.insert(url.origin().ascii_serialization(), cdn.clone());
    }
}

pub const BASE: &str = "https://huangguoai.com";

pub const UA: &str =
    "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36";

/// 手动指定代理（排查 / 公司内网用）：`HGHUB_PROXY=http://127.0.0.1:7897`。
/// 不设时交给 reqwest 的 `system-proxy` 探测（注册表 + 环境变量）。
fn proxy_override() -> Option<String> {
    std::env::var("HGHUB_PROXY")
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

pub static CLIENT: Lazy<reqwest::Client> = Lazy::new(|| {
    let mut builder = reqwest::Client::builder()
        .user_agent(UA)
        .timeout(Duration::from_secs(20))
        .connect_timeout(Duration::from_secs(10))
        .gzip(true)
        .brotli(true)
        .pool_idle_timeout(Duration::from_secs(60));
    #[cfg(target_os = "ios")]
    { builder = builder.no_proxy().dns_resolver(std::sync::Arc::new(crate::ios_dns::PublicDns)); }
    if let Some(p) = proxy_override() {
        match reqwest::Proxy::all(&p) {
            Ok(proxy) => builder = builder.proxy(proxy),
            Err(e) => eprintln!("[net] 忽略无效的 HGHUB_PROXY={p}: {e}"),
        }
    }
    builder.build().expect("build http client")
});

/// reqwest 的 `Display` 只有「error sending request for url (...)」，
/// 真正的原因（dns error / connection refused / invalid peer certificate）
/// 藏在 `source()` 链里。展开成一行，界面和 debug.log 才有排查价值。
pub fn err_text(e: &reqwest::Error) -> String {
    let mut out = e.to_string();
    let mut src = std::error::Error::source(e);
    while let Some(s) = src {
        let t = s.to_string();
        if !t.is_empty() && !out.contains(&t) {
            out.push_str(" ← ");
            out.push_str(&t);
        }
        src = s.source();
    }
    out
}

/// 网络自检：DNS / HTTP / 代理相关环境变量，用于远程定位「打不开」。
pub async fn diagnose() -> Value {
    let host = url::Url::parse(BASE)
        .ok()
        .and_then(|u| u.host_str().map(|h| h.to_string()))
        .unwrap_or_default();

    // fake-ip（198.18.0.0/15）说明 DNS 被 TUN 接管：关掉 TUN 后这个地址根本不可路由。
    let dns = {
        let host = host.clone();
        tokio::task::spawn_blocking(move || {
            use std::net::ToSocketAddrs;
            match (host.as_str(), 443u16).to_socket_addrs() {
                Ok(iter) => json!(iter.map(|a| a.ip().to_string()).collect::<Vec<_>>()),
                Err(e) => json!(format!("解析失败: {e}")),
            }
        })
        .await
        .unwrap_or(Value::Null)
    };

    let started = Instant::now();
    let http = match CLIENT.get(format!("{BASE}/")).send().await {
        Ok(r) => json!({
            "status": r.status().as_u16(),
            "ms": started.elapsed().as_millis() as u64,
        }),
        Err(e) => json!({
            "error": err_text(&e),
            "ms": started.elapsed().as_millis() as u64,
        }),
    };

    json!({
        "base": BASE,
        "host": host,
        "dns": dns,
        "http": http,
        "env": {
            "HTTP_PROXY": std::env::var("HTTP_PROXY").ok(),
            "HTTPS_PROXY": std::env::var("HTTPS_PROXY").ok(),
            "ALL_PROXY": std::env::var("ALL_PROXY").ok(),
            "NO_PROXY": std::env::var("NO_PROXY").ok(),
            "HGHUB_PROXY": proxy_override(),
        },
    })
}

/// 站点偶发 TLS 抖动（实测），统一重试。
async fn with_retry<F, Fut, T>(mut f: F) -> Result<T, String>
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = Result<T, String>>,
{
    let mut last = String::from("network error");
    for attempt in 0..3 {
        match f().await {
            Ok(v) => return Ok(v),
            Err(e) => {
                last = e;
                if attempt < 2 {
                    tokio::time::sleep(Duration::from_millis(350 * (attempt as u64 + 1))).await;
                }
            }
        }
    }
    Err(last)
}

/// 业务请求：走边缘信封。
pub async fn edge(
    method: &str,
    path: &str,
    body: Option<Value>,
    token: Option<&str>,
) -> Result<Value, String> {
    let (encoded, ctype) = match &body {
        Some(v) => (
            B64.encode(serde_json::to_vec(v).map_err(|e| e.to_string())?),
            "application/json",
        ),
        None => (String::new(), ""),
    };
    let envelope = json!({
        "method": method,
        "path": path,
        "body": encoded,
        "content_type": ctype,
    });

    let url = format!("{BASE}/api/edge/request");
    let token = token.map(|s| s.to_string());

    let value = with_retry(|| {
        let envelope = envelope.clone();
        let token = token.clone();
        let url = url.clone();
        async move {
            let mut req = CLIENT
                .post(&url)
                .header("Accept", "application/json")
                .header("Referer", format!("{BASE}/"))
                .json(&envelope);
            if let Some(t) = token.as_deref() {
                if !t.is_empty() {
                    req = req.header("Authorization", format!("Bearer {t}"));
                }
            }
            let resp = req.send().await.map_err(|e| err_text(&e))?;
            let status = resp.status();
            let text = resp.text().await.map_err(|e| err_text(&e))?;
            if status.as_u16() == 401 || status.as_u16() == 403 {
                return Err("401".to_string());
            }
            if !status.is_success() {
                return Err(format!("HTTP {status}: {}", &text[..text.len().min(160)]));
            }
            if text.trim().is_empty() {
                return Ok(Value::Null);
            }
            serde_json::from_str::<Value>(&text).map_err(|e| format!("{e}: {}", &text[..text.len().min(160)]))
        }
    })
    .await?;

    if value.is_null() {
        return Ok(Value::Null);
    }

    if value.get("status").and_then(|v| v.as_i64()) == Some(1) {
        Ok(value.get("data").cloned().unwrap_or(Value::Null))
    } else {
        Err(value
            .get("msg")
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .unwrap_or("接口返回失败")
            .to_string())
    }
}

/// 拉原始字节（图片 / m3u8 / 分片）。
pub async fn fetch_bytes(url: &str) -> Result<(Option<String>, Vec<u8>), String> {
    let url = url.to_string();
    with_retry(|| {
        let url = url.clone();
        async move {
            let mut request = CLIENT
                .get(&url)
                .header("Referer", format!("{BASE}/"));
            if let Ok(parsed) = url::Url::parse(&url) {
                let auth = CDN_AUTH.lock().unwrap().get(&parsed.origin().ascii_serialization()).cloned();
                if let Some(cdn) = auth {
                    let path = parsed.path().to_ascii_lowercase();
                    let enabled = if path.ends_with(".m3u8") { "m3u8Enabled" }
                        else if [".ts", ".m4s", ".key"].iter().any(|ext| path.ends_with(ext)) { "otherEnabled" }
                        else { "" };
                    if cdn[enabled].as_bool() == Some(true) {
                        if let (Some(key), Some(value), Some(app)) = (cdn["authKey"].as_str(), cdn["authValue"].as_str(), cdn["app"].as_str()) {
                            request = request.header(key, value).header("x-cdn-app", app);
                        }
                    }
                }
            }
            let resp = request.send()
                .await
                .map_err(|e| err_text(&e))?;
            if !resp.status().is_success() {
                return Err(format!("HTTP {}", resp.status()));
            }
            let ctype = resp
                .headers()
                .get(reqwest::header::CONTENT_TYPE)
                .and_then(|v| v.to_str().ok())
                .map(|s| s.to_string());
            let bytes = resp.bytes().await.map_err(|e| err_text(&e))?.to_vec();
            Ok((ctype, bytes))
        }
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 手动跑：`cargo test --lib net::tests::diag -- --nocapture`
    /// 关掉 TUN、只留系统代理时跑一遍，能直接看出客户端有没有走代理。
    #[tokio::test]
    async fn diag() {
        println!(
            "{}",
            serde_json::to_string_pretty(&diagnose().await).expect("json")
        );
    }
}
