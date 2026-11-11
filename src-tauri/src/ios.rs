//! Keep the iOS WKWebView in its parent's coordinate space, including after resume.
use objc2::{msg_send, runtime::AnyObject};
use objc2_core_foundation::CGRect;

pub fn repair(window: &tauri::WebviewWindow) -> tauri::Result<()> {
    window.with_webview(|platform| unsafe {
        let view = platform.inner().cast::<AnyObject>();
        let parent: *mut AnyObject = msg_send![view, superview];
        if !parent.is_null() {
            let bounds: CGRect = msg_send![parent, bounds];
            let _: () = msg_send![view, setFrame: bounds];
            // UIViewAutoresizingFlexibleWidth | UIViewAutoresizingFlexibleHeight.
            let _: () = msg_send![view, setAutoresizingMask: 18usize];
            let _: () = msg_send![parent, setNeedsLayout];
            let _: () = msg_send![parent, layoutIfNeeded];
        }
        let _: () = msg_send![view, setUserInteractionEnabled: true];
        let scroll: *mut AnyObject = msg_send![view, scrollView];
        // Safe areas are already handled by CSS; avoid a second native inset.
        let _: () = msg_send![scroll, setContentInsetAdjustmentBehavior: 2isize];
        let _: () = msg_send![scroll, setBounces: false];
        let _: () = msg_send![scroll, setDelaysContentTouches: false];
    })
}
