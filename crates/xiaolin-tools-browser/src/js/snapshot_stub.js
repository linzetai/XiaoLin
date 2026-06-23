// Minimal a11y snapshot for WebView engine (full version injected at eval time in future phases).
(() => {
  let uid = 0;
  function walk(el) {
    if (!el || el.nodeType !== 1) return null;
    const rect = el.getBoundingClientRect();
    if (rect.width <= 0 || rect.height <= 0) return null;
    const id = 'e' + (uid++);
    el.setAttribute('data-fc-uid', id);
    const tag = el.tagName.toLowerCase();
    const children = [];
    for (const child of el.children) {
      const c = walk(child);
      if (c) children.push(c);
    }
    return { uid: id, tag, children };
  }
  return JSON.stringify({ tree: walk(document.body), url: location.href, title: document.title });
})()
