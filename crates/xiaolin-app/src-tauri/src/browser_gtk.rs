//! Linux-only GTK reparenting for browser child WebViews.
//!
//! On WebKitGTK, Tauri places all WebViews (main + children) in a vertical
//! `GtkBox` with `pack_start(expand=true, fill=true)`. This causes the window
//! space to be split equally among all WebViews, and `set_position`/`set_size`
//! become no-ops because `wry::set_bounds` skips non-GtkFixed parents.
//!
//! This module replaces the GtkBox layout with a GtkFixed container so that
//! child WebViews (browser pages) can be positioned absolutely on top of the
//! main WebView.
//!
//! All public functions in this module MUST be called from the GTK main thread
//! (e.g., inside a `window.run_on_main_thread()` closure).

use gtk::prelude::*;
use std::cell::RefCell;
use std::collections::HashMap;
use std::path::Path;
use gtk::glib::object::ObjectExt;

use crate::browser_network::WebviewProxySetting;

thread_local! {
    static FIXED: RefCell<Option<gtk::Fixed>> = RefCell::new(None);
    static MAIN_WV: RefCell<Option<gtk::Widget>> = RefCell::new(None);
    static CHILD_WIDGETS: RefCell<HashMap<String, gtk::Widget>> = RefCell::new(HashMap::new());
}

/// One-time setup: replace the GtkBox content with a GtkFixed so that the main
/// WebView fills the window and child WebViews can be absolutely positioned.
///
/// Must be called on the GTK main thread.
pub fn ensure_fixed_container(vbox: &gtk::Box) {
    FIXED.with(|f| {
        if f.borrow().is_some() {
            return;
        }

        let children = vbox.children();
        if children.is_empty() {
            return;
        }

        let main_wv = children[0].clone();
        vbox.remove(&main_wv);

        let fixed = gtk::Fixed::new();
        fixed.put(&main_wv, 0, 0);

        let wv_for_resize = main_wv.clone();
        fixed.connect_size_allocate(move |_, alloc| {
            let (w, h) = (alloc.width(), alloc.height());
            wv_for_resize.set_size_request(w, h);
            wv_for_resize.size_allocate(&gtk::Allocation::new(0, 0, w, h));
        });

        vbox.pack_start(&fixed, true, true, 0);
        fixed.show_all();

        MAIN_WV.with(|m| m.borrow_mut().replace(main_wv));
        f.borrow_mut().replace(fixed);
    });
}

/// After `window.add_child()` appends a child WebView to the vbox,
/// move it into the GtkFixed for absolute positioning.
///
/// The child WebView may not be the last child of the vbox because
/// `ensure_fixed_container` adds the GtkFixed via `pack_start` which
/// appends after existing children. We scan for the first WebKit widget
/// that is a direct child of the vbox.
///
/// Must be called on the GTK main thread.
pub fn reparent_child_webview(vbox: &gtk::Box, label: &str) {
    let children = vbox.children();
    let webkit_child = children
        .iter()
        .find(|c| c.type_().name().contains("WebKit"));

    let Some(child) = webkit_child else {
        tracing::warn!("no WebKitWebView found in vbox children, skipping reparent");
        return;
    };

    vbox.remove(child);

    FIXED.with(|f| {
        if let Some(fixed) = f.borrow().as_ref() {
            fixed.put(child, -9999, -9999);
            child.set_size_request(1, 1);
            child.show();
        }
    });

    CHILD_WIDGETS.with(|w| {
        w.borrow_mut().insert(label.to_string(), child.clone());
    });
}

/// Position a child WebView at the given logical coordinates inside the window.
///
/// Must be called on the GTK main thread.
pub fn position_child(label: &str, x: i32, y: i32, width: i32, height: i32, visible: bool) {
    CHILD_WIDGETS.with(|widgets| {
        let widgets = widgets.borrow();
        let Some(widget) = widgets.get(label) else {
            return;
        };

        FIXED.with(|f| {
            let f = f.borrow();
            let Some(fixed) = f.as_ref() else { return };

            if visible {
                fixed.move_(widget, x, y);
                widget.set_size_request(width, height);
                widget.show();
            } else {
                fixed.move_(widget, -9999, -9999);
                widget.set_size_request(1, 1);
            }
        });
    });
}

/// Remove the tracking entry for a closed page.
///
/// Must be called on the GTK main thread.
/// The actual GTK widget is destroyed by Tauri when `webview.close()` is called.
pub fn remove_child(label: &str) {
    CHILD_WIDGETS.with(|w| {
        w.borrow_mut().remove(label);
    });
}

/// Configure cookie persistence and acceptance for a browser child WebView.
///
/// On WebKitGTK 2.52+ (GTK3 API), calling `set_persistent_storage` through the
/// Rust `webkit2gtk` crate's `WebsiteDataManager::cookie_manager()` is silently
/// ignored. We work around this by calling the C API directly via FFI on the
/// cookie manager obtained from the WebContext.
///
/// Must be called on the GTK main thread AFTER the WebView has been reparented.
pub fn configure_webview_cookies(label: &str, data_dir: &Path) {
    use std::ffi::CString;

    CHILD_WIDGETS.with(|widgets| {
        let widgets = widgets.borrow();
        let Some(widget) = widgets.get(label) else {
            tracing::error!(label, "configure_webview_cookies: widget not found");
            return;
        };

        let widget_type = widget.type_().name().to_string();
        tracing::info!(label, widget_type = %widget_type, "configure_webview_cookies: attempting downcast");

        let Some(webview) = widget.downcast_ref::<webkit2gtk::WebView>() else {
            tracing::error!(label, widget_type = %widget_type, "configure_webview_cookies: downcast failed");
            return;
        };

        let cookie_path = data_dir.join("cookies.sqlite");
        let cookie_path_c = match CString::new(cookie_path.to_string_lossy().as_bytes()) {
            Ok(c) => c,
            Err(e) => {
                tracing::error!(error = %e, "configure_webview_cookies: invalid cookie path");
                return;
            }
        };

        unsafe {
            let wv_ptr = webview.as_ptr() as *mut std::ffi::c_void;
            let ctx_ptr = ffi_webkit::webkit_web_view_get_context(wv_ptr);
            if ctx_ptr.is_null() {
                tracing::error!(label, "configure_webview_cookies: null WebContext");
                return;
            }

            let cm_ptr = ffi_webkit::webkit_web_context_get_cookie_manager(ctx_ptr);
            if cm_ptr.is_null() {
                tracing::error!(label, "configure_webview_cookies: null CookieManager from context");
                return;
            }

            tracing::info!(
                label,
                path = %cookie_path.display(),
                "configure_webview_cookies: FFI set_persistent_storage (SQLite) + accept_policy"
            );

            ffi_webkit::webkit_cookie_manager_set_persistent_storage(
                cm_ptr,
                cookie_path_c.as_ptr(),
                1, // WEBKIT_COOKIE_PERSISTENT_STORAGE_SQLITE = 1
            );
            ffi_webkit::webkit_cookie_manager_set_accept_policy(
                cm_ptr,
                0, // WEBKIT_COOKIE_ACCEPT_POLICY_ALWAYS = 0
            );
        }

        tracing::info!(label, "configure_webview_cookies: done (FFI path)");
    });
}

/// Configure network proxy for a browser child WebView via FFI.
///
/// On WebKitGTK 2.52+ (GTK3 API), `builder.proxy_url()` calls the deprecated
/// `WebsiteDataManager::set_network_proxy_settings` which breaks the cookie jar.
/// This function sets the proxy AFTER cookies are configured, preserving cookie
/// functionality. Call this AFTER `configure_webview_cookies`.
///
/// Must be called on the GTK main thread.
pub fn configure_webview_proxy(label: &str, proxy_url: &str) {
    reapply_webview_proxy(label, &WebviewProxySetting::Custom(proxy_url.to_string()));
}

/// Apply or clear WebView proxy settings (hot-reconfig after network settings change).
///
/// Must be called on the GTK main thread.
pub(crate) fn reapply_webview_proxy(label: &str, setting: &WebviewProxySetting) {
    use std::ffi::CString;

    const PROXY_MODE_DEFAULT: u32 = 0;
    const PROXY_MODE_NO_PROXY: u32 = 1;
    const PROXY_MODE_CUSTOM: u32 = 2;

    CHILD_WIDGETS.with(|widgets| {
        let widgets = widgets.borrow();
        let Some(widget) = widgets.get(label) else {
            tracing::error!(label, "reapply_webview_proxy: widget not found");
            return;
        };

        let Some(webview) = widget.downcast_ref::<webkit2gtk::WebView>() else {
            tracing::error!(label, "reapply_webview_proxy: downcast failed");
            return;
        };

        unsafe {
            let wv_ptr = webview.as_ptr() as *mut std::ffi::c_void;
            let ctx_ptr = ffi_webkit::webkit_web_view_get_context(wv_ptr);
            if ctx_ptr.is_null() {
                tracing::error!(label, "reapply_webview_proxy: null WebContext");
                return;
            }

            let dm_ptr = ffi_webkit::webkit_web_context_get_website_data_manager(ctx_ptr);
            if dm_ptr.is_null() {
                tracing::error!(label, "reapply_webview_proxy: null WebsiteDataManager");
                return;
            }

            match setting {
                WebviewProxySetting::Direct => {
                    ffi_webkit::webkit_website_data_manager_set_network_proxy_settings(
                        dm_ptr,
                        PROXY_MODE_NO_PROXY,
                        std::ptr::null_mut(),
                    );
                    tracing::info!(label, "reapply_webview_proxy: direct (no proxy)");
                }
                WebviewProxySetting::System => {
                    ffi_webkit::webkit_website_data_manager_set_network_proxy_settings(
                        dm_ptr,
                        PROXY_MODE_DEFAULT,
                        std::ptr::null_mut(),
                    );
                    tracing::info!(label, "reapply_webview_proxy: system proxy");
                }
                WebviewProxySetting::Custom(proxy_url) => {
                    let proxy_c = match CString::new(proxy_url.as_str()) {
                        Ok(c) => c,
                        Err(e) => {
                            tracing::error!(error = %e, label, "reapply_webview_proxy: invalid proxy URL");
                            return;
                        }
                    };

                    let ignore_localhost = [
                        CString::new("localhost").unwrap(),
                        CString::new("127.0.0.1").unwrap(),
                        CString::new("::1").unwrap(),
                    ];
                    let ignore_ptrs: Vec<*const std::ffi::c_char> = ignore_localhost
                        .iter()
                        .map(|s| s.as_ptr())
                        .chain(std::iter::once(std::ptr::null()))
                        .collect();
                    let settings_ptr = ffi_webkit::webkit_network_proxy_settings_new(
                        proxy_c.as_ptr(),
                        ignore_ptrs.as_ptr(),
                    );
                    if settings_ptr.is_null() {
                        tracing::error!(label, "reapply_webview_proxy: failed to create proxy settings");
                        return;
                    }

                    ffi_webkit::webkit_website_data_manager_set_network_proxy_settings(
                        dm_ptr,
                        PROXY_MODE_CUSTOM,
                        settings_ptr,
                    );

                    ffi_webkit::webkit_network_proxy_settings_free(settings_ptr);
                    tracing::info!(label, proxy_url, "reapply_webview_proxy: custom proxy set");
                }
            }
        }
    });
}

/// Register `xiaolin-internal` scheme as CORS-enabled on the WebView's context.
/// Without this, WebKitGTK blocks fetch() calls from https:// origins to custom schemes.
/// Must be called on the GTK main thread.
pub fn configure_webview_cors(label: &str) {
    use std::ffi::CString;

    CHILD_WIDGETS.with(|widgets| {
        let widgets = widgets.borrow();
        let Some(widget) = widgets.get(label) else {
            tracing::error!(label, "configure_webview_cors: widget not found");
            return;
        };

        let Some(webview) = widget.downcast_ref::<webkit2gtk::WebView>() else {
            tracing::error!(label, "configure_webview_cors: downcast failed");
            return;
        };

        let scheme = match CString::new("xiaolin-internal") {
            Ok(c) => c,
            Err(_) => return,
        };

        unsafe {
            let wv_ptr = webview.as_ptr() as *mut std::ffi::c_void;
            let ctx_ptr = ffi_webkit::webkit_web_view_get_context(wv_ptr);
            if ctx_ptr.is_null() {
                tracing::error!(label, "configure_webview_cors: null WebContext");
                return;
            }

            let sm_ptr = ffi_webkit::webkit_web_context_get_security_manager(ctx_ptr);
            if sm_ptr.is_null() {
                tracing::error!(label, "configure_webview_cors: null SecurityManager");
                return;
            }

            ffi_webkit::webkit_security_manager_register_uri_scheme_as_cors_enabled(
                sm_ptr,
                scheme.as_ptr(),
            );
        }

        tracing::info!(label, "configured xiaolin-internal scheme as CORS-enabled");
    });
}

mod ffi_webkit {
    use std::os::raw::c_char;

    extern "C" {
        pub fn webkit_web_view_get_context(
            webview: *mut std::ffi::c_void,
        ) -> *mut std::ffi::c_void;

        pub fn webkit_web_context_get_cookie_manager(
            context: *mut std::ffi::c_void,
        ) -> *mut std::ffi::c_void;

        pub fn webkit_cookie_manager_set_persistent_storage(
            cookie_manager: *mut std::ffi::c_void,
            filename: *const c_char,
            storage: u32,
        );

        pub fn webkit_cookie_manager_set_accept_policy(
            cookie_manager: *mut std::ffi::c_void,
            policy: u32,
        );

        pub fn webkit_web_context_get_website_data_manager(
            context: *mut std::ffi::c_void,
        ) -> *mut std::ffi::c_void;

        pub fn webkit_network_proxy_settings_new(
            default_proxy_uri: *const c_char,
            ignore_hosts: *const *const c_char,
        ) -> *mut std::ffi::c_void;

        pub fn webkit_website_data_manager_set_network_proxy_settings(
            manager: *mut std::ffi::c_void,
            proxy_mode: u32,
            proxy_settings: *mut std::ffi::c_void,
        );

        pub fn webkit_network_proxy_settings_free(
            settings: *mut std::ffi::c_void,
        );

        pub fn webkit_web_context_get_security_manager(
            context: *mut std::ffi::c_void,
        ) -> *mut std::ffi::c_void;

        pub fn webkit_security_manager_register_uri_scheme_as_cors_enabled(
            security_manager: *mut std::ffi::c_void,
            scheme: *const c_char,
        );
    }
}
