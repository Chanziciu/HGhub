mod api;
mod media;
mod net;
mod session;
#[cfg(target_os = "ios")]
mod ios;
#[cfg(target_os = "ios")]
mod ios_dns;
#[cfg(target_os = "ios")]
mod media_server;

#[tauri::command]
async fn media_origin(app: tauri::AppHandle) -> Result<String, String> {
    #[cfg(target_os = "ios")]
    { return Ok(app.state::<media_server::MediaOrigin>().0.clone()); }
    #[cfg(not(target_os = "ios"))]
    { let _ = app; Ok(LOCAL_ORIGIN.to_string()) }
}

use api::Api;
use tauri::Manager;

#[tauri::command]
async fn repair_mobile_layout(window: tauri::WebviewWindow) -> Result<serde_json::Value, String> {
    #[cfg(target_os = "ios")]
    { return ios::repair(&window).await; }
    #[cfg(not(target_os = "ios"))]
    { let _ = window; Ok(serde_json::Value::Null) }
}

/// 自定义协议 hg 在 WebView 里的源：Windows / Android 是 http://<scheme>.localhost，
/// iOS（WKWebView）是 <scheme>://localhost。播放列表里的分片地址按它改写，必须和前端 api.ts 的 LOCAL 一致。
/// 注意：不能拿请求 URI 的 scheme 来拼（各平台到这里都是 "hg"），否则 Windows / Android 的 WebView 解析不了。
#[cfg(target_os = "ios")]
const LOCAL_ORIGIN: &str = "hg://localhost";
#[cfg(not(target_os = "ios"))]
const LOCAL_ORIGIN: &str = "http://hg.localhost";

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .manage(Api::new())
        .setup(|app| {
            #[cfg(target_os = "ios")]
            app.manage(media_server::MediaOrigin(media_server::start()?));
            if let Ok(dir) = app.path().app_data_dir() {
                let _ = std::fs::create_dir_all(&dir);
                let state: tauri::State<Api> = app.state();
                state.session.bind(dir);
            }
            Ok(())
        })
        .register_asynchronous_uri_scheme_protocol("hg", |_app, request, responder| {
            let uri = request.uri().clone();
            let path = uri.path().to_string();
            let query = uri.query().unwrap_or("").to_string();
            tauri::async_runtime::spawn(async move {
                let resp = media::handle(&path, &query, LOCAL_ORIGIN).await;
                responder.respond(resp);
            });
        })
        .invoke_handler(tauri::generate_handler![
            media_origin,
            repair_mobile_layout,
            api::app_boot,
            api::debug_log,
            api::net_diag,
            api::api_login,
            api::api_register,
            api::api_logout,
            api::api_me,
            api::api_feed,
            api::api_category,
            api::api_rank,
            api::api_search,
            api::cache_clear,
            api::api_posts,
            api::api_post,
            api::api_post_like,
            api::api_post_collect,
            api::api_post_comments,
            api::api_video,
            api::api_play,
            api::api_comments,
            api::api_comment_send,
            api::api_like,
            api::api_collect,
            api::api_follow,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
