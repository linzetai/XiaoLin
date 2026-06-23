(function () {
  if (window.__xiaolin_extract) return;

  function stripNoise(root) {
    if (!root) return document.createElement("div");
    var clone = root.cloneNode(true);
    ["script", "style", "nav", "header", "footer", "aside", "noscript", "iframe"].forEach(
      function (tag) {
        clone.querySelectorAll(tag).forEach(function (el) {
          el.remove();
        });
      },
    );
    return clone;
  }

  var api = {
    text: function (maxLen) {
      maxLen = maxLen || 50000;
      var root = document.body || document.documentElement;
      var el = stripNoise(root);
      return (el.innerText || "")
        .replace(/\s+/g, " ")
        .trim()
        .slice(0, maxLen);
    },
    tables: function () {
      return Array.from(document.querySelectorAll("table"))
        .slice(0, 20)
        .map(function (table) {
          var headers = Array.from(table.querySelectorAll("th")).map(function (th) {
            return (th.innerText || "").trim();
          });
          var rows = Array.from(table.querySelectorAll("tr"))
            .slice(0, 50)
            .map(function (tr) {
              return Array.from(tr.querySelectorAll("td,th")).map(function (c) {
                return (c.innerText || "").trim();
              });
            });
          return { headers: headers, rows: rows };
        });
    },
    links: function () {
      return Array.from(document.querySelectorAll("a[href]"))
        .slice(0, 200)
        .map(function (a) {
          var href = a.href || "";
          if (!/^https?:/i.test(href)) return null;
          return { href: href, text: (a.innerText || "").trim().slice(0, 200) };
        })
        .filter(Boolean);
    },
    metadata: function () {
      function meta(name) {
        var el = document.querySelector(
          'meta[name="' + name + '"],meta[property="' + name + '"]',
        );
        return el ? el.getAttribute("content") : null;
      }
      var jsonLd = [];
      document.querySelectorAll('script[type="application/ld+json"]').forEach(function (s) {
        try {
          jsonLd.push(JSON.parse(s.textContent || "null"));
        } catch (e) {}
      });
      return {
        title: document.title,
        description: meta("description") || meta("og:description"),
        og: {
          title: meta("og:title"),
          image: meta("og:image"),
          url: meta("og:url"),
        },
        jsonLd: jsonLd.slice(0, 3),
      };
    },
  };

  Object.freeze(api);
  Object.defineProperty(window, "__xiaolin_extract", {
    value: api,
    writable: false,
    configurable: false,
    enumerable: false,
  });
})();
