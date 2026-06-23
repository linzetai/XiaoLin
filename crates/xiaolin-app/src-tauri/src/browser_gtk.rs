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
