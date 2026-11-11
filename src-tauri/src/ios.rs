//! Keep the iOS WKWebView in its parent's coordinate space, including after resume.
use objc2::{msg_send, runtime::AnyObject};
use objc2_core_foundation::CGRect;

pub async fn repair(window: &tauri::WebviewWindow) -> Result<serde_json::Value, String> {
    let (sender, receiver) = tokio::sync::oneshot::channel();
    window.with_webview(move |platform| unsafe {
        let view = platform.inner().cast::<AnyObject>();
        let parent: *mut AnyObject = msg_send![view, superview];
        let native_window: *mut AnyObject = msg_send![view, window];
        if !native_window.is_null() {
            // Read the active scene rather than assuming a device model or subtracting safe areas.
            let scene: *mut AnyObject = msg_send![native_window, windowScene];
            let coordinate_space: *mut AnyObject = if scene.is_null() {
                msg_send![native_window, screen]
            } else {
                msg_send![scene, coordinateSpace]
            };
            let scene_bounds: CGRect = msg_send![coordinate_space, bounds];
            let frame: CGRect = msg_send![native_window, frame];
            if frame != scene_bounds {
                let _: () = msg_send![native_window, setFrame: scene_bounds];
            }
        }
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
            let bounds: CGRect = msg_send![parent, bounds];
            let frame: CGRect = msg_send![view, frame];
            if frame != bounds {
                let _: () = msg_send![view, setFrame: bounds];
            }
            // Pin to the full content edges, NOT safeAreaLayoutGuide. CSS owns safe-area padding.
            // Install once; UIKit then tracks resizing without repeated frame arithmetic.
            let translates: bool = msg_send![view, translatesAutoresizingMaskIntoConstraints];
            if translates {
                let _: () = msg_send![view, setTranslatesAutoresizingMaskIntoConstraints: false];
                macro_rules! pin {
                    ($anchor:ident) => {{
                        let child: *mut AnyObject = msg_send![view, $anchor];
                        let container: *mut AnyObject = msg_send![parent, $anchor];
                        let constraint: *mut AnyObject = msg_send![child, constraintEqualToAnchor: container];
                        let _: () = msg_send![constraint, setActive: true];
                    }};
                }
                pin!(leadingAnchor);
                pin!(trailingAnchor);
                pin!(topAnchor);
                pin!(bottomAnchor);
                let _: () = msg_send![parent, layoutIfNeeded];
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
