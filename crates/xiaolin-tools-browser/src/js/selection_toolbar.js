(function () {
  if (window.__XIAOLIN_SELECTION_TOOLBAR__) return;
  window.__XIAOLIN_SELECTION_TOOLBAR__ = true;

  var selectedText = "";

  var host = document.createElement("div");
  host.id = "__xiaolin_sel_host";
  host.style.cssText =
    "position:fixed;z-index:2147483647;pointer-events:none;top:0;left:0;";
  (document.documentElement || document.body).appendChild(host);

  var shadow = host.attachShadow({ mode: "closed" });

  var style = document.createElement("style");
  style.textContent =
    ".xl-bar{display:flex;gap:4px;padding:4px 6px;border-radius:8px;" +
    "background:#1a1a1a;border:1px solid rgba(255,255,255,0.12);" +
    "box-shadow:0 4px 16px rgba(0,0,0,0.35);font-family:system-ui,sans-serif;font-size:12px;" +
    "pointer-events:auto;white-space:nowrap;}" +
    ".xl-btn{cursor:pointer;border:none;border-radius:6px;padding:5px 10px;" +
    "background:rgba(255,255,255,0.08);color:#f5f5f5;font-size:12px;line-height:1.2;" +
    "transition:background 120ms;}" +
    ".xl-btn:hover{background:rgba(255,255,255,0.16);}";
  shadow.appendChild(style);

  var bar = document.createElement("div");
  bar.className = "xl-bar";
  bar.style.display = "none";
  shadow.appendChild(bar);

  function makeBtn(label, action) {
    var btn = document.createElement("button");
    btn.type = "button";
    btn.className = "xl-btn";
    btn.textContent = label;
    btn.addEventListener("mousedown", function (e) {
      e.preventDefault();
      e.stopPropagation();
    });
    btn.addEventListener("click", function (e) {
      e.preventDefault();
      e.stopPropagation();
      onAction(action);
    });
    return btn;
  }

  bar.appendChild(makeBtn("🤖 问 Agent", "ask"));
  bar.appendChild(makeBtn("📋 复制", "copy"));
  bar.appendChild(makeBtn("💬 引用", "quote"));

  function hide() {
    bar.style.display = "none";
    host.style.pointerEvents = "none";
  }

  function showAt(rect) {
    bar.style.display = "flex";
    var top = rect.top - bar.offsetHeight - 8;
    var left = rect.left + rect.width / 2 - bar.offsetWidth / 2;
    left = Math.max(8, Math.min(left, window.innerWidth - bar.offsetWidth - 8));
    top = Math.max(8, top);
    host.style.left = left + "px";
    host.style.top = top + "px";
    host.style.pointerEvents = "auto";
  }

  function onAction(action) {
    var text = selectedText;
    hide();
    try {
      window.getSelection()?.removeAllRanges();
    } catch (e) {}

    if (action === "copy") {
      if (navigator.clipboard && navigator.clipboard.writeText) {
        navigator.clipboard.writeText(text).catch(function () {});
      }
      return;
    }

    if (window.__XIAOLIN__ && typeof window.__XIAOLIN__.notify === "function") {
      window.__XIAOLIN__.notify("selection", {
        action: action,
        text: text,
        url: location.href,
      });
    }
  }

  document.addEventListener("selectionchange", function () {
    var sel = window.getSelection();
    if (!sel || sel.isCollapsed || sel.rangeCount === 0) {
      hide();
      return;
    }
    var text = sel.toString().trim();
    if (text.length <= 5) {
      hide();
      return;
    }
    var range = sel.getRangeAt(0);
    var rect = range.getBoundingClientRect();
    if (!rect.width && !rect.height) {
      hide();
      return;
    }
    selectedText = text;
    showAt(rect);
  });

  document.addEventListener("mousedown", function (e) {
    if (host.contains(e.target)) return;
    hide();
  });

  document.addEventListener("scroll", hide, true);
})();
