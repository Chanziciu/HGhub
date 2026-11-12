//! Loopback HTTP transport for AVFoundation, which cannot load WKURLSchemeHandler URLs.
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use std::time::Duration;

pub struct MediaOrigin(pub String);

pub fn start() -> std::io::Result<String> {
    let listener = std::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))?;
    listener.set_nonblocking(true)?;
    let port = listener.local_addr()?.port();
    let token = uuid::Uuid::new_v4().simple().to_string();
    let origin = format!("http://127.0.0.1:{port}/{token}");
    let public = origin.clone();
    tauri::async_runtime::spawn(async move {
        let listener = tokio::net::TcpListener::from_std(listener).expect("loopback listener");
        let limit = std::sync::Arc::new(tokio::sync::Semaphore::new(32));
        while let Ok((mut socket, _)) = listener.accept().await {
            let Ok(permit) = limit.clone().try_acquire_owned() else { continue };
            let token = token.clone();
            let origin = public.clone();
            tauri::async_runtime::spawn(async move {
                let _permit = permit;
                let _ = tokio::time::timeout(Duration::from_secs(90), async {
                    let mut request = Vec::new();
                    let mut chunk = [0u8; 2048];
                    while !request.windows(4).any(|w| w == b"\r\n\r\n") {
                        let n = socket.read(&mut chunk).await?;
                        if n == 0 || request.len() + n > 16384 { return Ok::<(), std::io::Error>(()); }
                        request.extend_from_slice(&chunk[..n]);
                    }
                    let text = String::from_utf8_lossy(&request);
                    let first: Vec<_> = text.lines().next().unwrap_or("").split_whitespace().collect();
                    if first.len() < 2 { return Ok(()); }
                    let method = first[0];
                    if !matches!(method, "GET" | "HEAD") { return Ok(()); }
                    let Some(target) = first[1].strip_prefix(&format!("/{token}")) else { return Ok(()); };
                    let (path, query) = target.split_once('?').unwrap_or((target, ""));
                    if !["/hls", "/seg", "/img"].contains(&path) { return Ok(()); }
                    let response = crate::media::handle(path, query, &origin).await;
                    let kind = response.headers().get("Content-Type").and_then(|v| v.to_str().ok()).unwrap_or("application/octet-stream").to_string();
                    let mut status = response.status().as_u16();
                    let body = response.into_body();
                    let total = body.len();
                    let mut start = 0;
                    let mut end = total;
                    let mut content_range = String::new();
                    if status == 200 {
                        if let Some(range) = text.lines().find_map(|line| {
                            let (key, value) = line.split_once(':')?;
                            key.eq_ignore_ascii_case("range").then_some(value.trim())
                        }).and_then(|range| range.strip_prefix("bytes=")) {
                            if let Some((a,b)) = range.split_once('-') {
                                if a.is_empty() {
                                    if let Ok(tail) = b.parse::<usize>() { start = total.saturating_sub(tail); }
                                } else if let Ok(n) = a.parse::<usize>() {
                                    start = n;
                                    if let Ok(n) = b.parse::<usize>() { end = total.min(n.saturating_add(1)); }
                                }
                                if start < end && end <= total {
                                    status = 206;
                                    content_range = format!("Content-Range: bytes {}-{}/{}\r\n", start, end-1, total);
                                } else {
                                    status = 416; start = 0; end = 0;
                                    content_range = format!("Content-Range: bytes */{total}\r\n");
                                }
                            }
                        }
                    }
                    let reason = match status { 200=>"OK",206=>"Partial Content",416=>"Range Not Satisfiable",_=>"Error" };
                    let header = format!("HTTP/1.1 {status} {reason}\r\nContent-Type: {kind}\r\nContent-Length: {}\r\n{content_range}Accept-Ranges: bytes\r\nAccess-Control-Allow-Origin: *\r\nAccess-Control-Allow-Headers: *\r\nConnection: close\r\n\r\n", end-start);
                    socket.write_all(header.as_bytes()).await?;
                    if method != "HEAD" { socket.write_all(&body[start..end]).await?; }
                    socket.shutdown().await
                }).await;
            });
        }
    });
    Ok(origin)
}
