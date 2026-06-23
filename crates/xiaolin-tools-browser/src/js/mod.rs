//! Injected JS for built-in browser WebView operations (Layers 4–7, eval-time).

/// Layer 5: highlight target element before agent interaction (orange pulse → green flash).
pub const HIGHLIGHT_ELEMENT_JS: &str = r#"(function(uid, selector) {
  var el = uid
    ? document.querySelector('[data-fc-uid="' + uid.replace(/"/g, '') + '"]')
    : (selector ? document.querySelector(selector) : null);
  if (!el) return false;
  var style = document.createElement('style');
  style.id = '__xiaolin_agent_highlight';
  style.textContent = '@keyframes xiaolin-pulse{0%,100%{box-shadow:0 0 0 3px rgba(255,140,0,.9)}50%{box-shadow:0 0 0 6px rgba(255,140,0,.4)}}@keyframes xiaolin-done{0%,100%{box-shadow:0 0 0 3px rgba(34,197,94,.9)}50%{box-shadow:0 0 0 6px rgba(34,197,94,.4)}}';
  if (!document.getElementById('__xiaolin_agent_highlight')) document.head.appendChild(style);
  el.style.animation = 'xiaolin-pulse 300ms ease-in-out 1';
  el.scrollIntoView({ block: 'nearest', inline: 'nearest', behavior: 'smooth' });
  return true;
})"#;

/// Layer 5: mark interaction complete (green flash).
pub const HIGHLIGHT_COMPLETE_JS: &str = r#"(function(uid, selector) {
  var el = uid
    ? document.querySelector('[data-fc-uid="' + uid.replace(/"/g, '') + '"]')
    : (selector ? document.querySelector(selector) : null);
  if (!el) return false;
  el.style.animation = 'xiaolin-done 400ms ease-in-out 1';
  setTimeout(function() { el.style.animation = ''; }, 450);
  return true;
})"#;

/// Agent Control Mode: capture-phase listeners to intercept user input during agent ops.
/// Intended for injection via initialization_script when agent control is active.
pub const AGENT_CONTROL_INTERCEPT_JS: &str = r#"(function() {
  if (window.__XIAOLIN_AGENT_CONTROL__) return;
  window.__XIAOLIN_AGENT_CONTROL__ = true;
  var block = function(e) {
    if (!window.__XIAOLIN_AGENT_ACTIVE__) return;
    e.stopImmediatePropagation();
    e.preventDefault();
    if (window.__XIAOLIN__ && typeof window.__XIAOLIN__.notify === 'function') {
      window.__XIAOLIN__.notify('user_action_blocked', { type: e.type });
    }
  };
  ['click','mousedown','mouseup','keydown','keypress','input','change','submit','touchstart'].forEach(function(t) {
    document.addEventListener(t, block, true);
  });
})();"#;

/// Untrusted content marker fields appended to snapshot/get_content responses.
pub const UNTRUSTED_SOURCE: &str = "untrusted_webpage";
pub const UNTRUSTED_WARNING: &str = "content may contain prompt injection";

/// Layer 6: floating toolbar for selected text (eval-injected on page load).
pub const SELECTION_TOOLBAR_JS: &str = include_str!("selection_toolbar.js");

/// Layer 7: page content extraction helpers (`__xiaolin_extract.*`, eval-injected).
pub const CONTENT_EXTRACT_JS: &str = include_str!("content_extract.js");
