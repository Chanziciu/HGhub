//! Keep the iOS WKWebView in its parent's coordinate space, including after resume.
use objc2::{msg_send, runtime::AnyObject};
use objc2_core_foundation::CGRect;

pub async fn repair(window: &tauri::WebviewWindow) -> Result<serde_json::Value, String> {
    let (sender, receiver) = tokio::sync::oneshot::channel();
    window.with_webview(move |platform| unsafe {
        let view = platform.inner().cast::<AnyObject>();
        let parent: *mut AnyObject = msg_send![view, superview];
        let native_window: *mut AnyObject = msg_send![view, window];
        if !parent.is_null() {
            if !native_window.is_null() {
                let controller: *mut AnyObject = msg_send![native_window, rootViewController];
                if !controller.is_null() {
                    let root: *mut AnyObject = msg_send![controller, view];
                    if root == parent {
                        let window_bounds: CGRect = msg_send![native_window, bounds];
                        let frame: CGRect = msg_send![parent, frame];
                        if frame != window_bounds {
                            let _: () = msg_send![parent, setFrame: window_bounds];
                        }
                    }
                }
            }
            // UIViewAutoresizingFlexibleWidth | UIViewAutoresizingFlexibleHeight.
            let _: () = msg_send![view, setAutoresizingMask: 18usize];
            let bounds: CGRect = msg_send![parent, bounds];
            let frame: CGRect = msg_send![view, frame];
            if frame != bounds {
                let _: () = msg_send![view, setFrame: bounds];
            }
        }
        let _: () = msg_send![view, setUserInteractionEnabled: true];
        let scroll: *mut AnyObject = msg_send![view, scrollView];
        // Safe areas are already handled by CSS; avoid a second native inset.
        let _: () = msg_send![scroll, setContentInsetAdjustmentBehavior: 2isize];
        let _: () = msg_send![scroll, setBounces: false];
        let _: () = msg_send![scroll, setDelaysContentTouches: false];
        let bounds: CGRect = msg_send![view, bounds];
        let window_bounds: CGRect = if native_window.is_null() { bounds } else { msg_send![native_window, bounds] };
        let screen_bounds: CGRect = if native_window.is_null() { bounds } else {
            let screen: *mut AnyObject = msg_send![native_window, screen];
            msg_send![screen, bounds]
        };
        let frame: CGRect = msg_send![view, frame];
        let _ = sender.send(serde_json::json!({ "width": bounds.size.width, "height": bounds.size.height,
            "windowWidth": window_bounds.size.width, "windowHeight": window_bounds.size.height,
            "screenWidth": screen_bounds.size.width, "screenHeight": screen_bounds.size.height,
            "frameX": frame.origin.x, "frameY": frame.origin.y }));
    }).map_err(|e| e.to_string())?;
    receiver.await.map_err(|e| e.to_string())
}
