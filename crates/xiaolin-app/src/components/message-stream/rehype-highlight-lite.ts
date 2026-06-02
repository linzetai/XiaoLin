import { createLowlight } from "lowlight";
import javascript from "highlight.js/lib/languages/javascript";
import typescript from "highlight.js/lib/languages/typescript";
import python from "highlight.js/lib/languages/python";
import rust from "highlight.js/lib/languages/rust";
import bash from "highlight.js/lib/languages/bash";
import json from "highlight.js/lib/languages/json";
import css from "highlight.js/lib/languages/css";
import xml from "highlight.js/lib/languages/xml";
import sql from "highlight.js/lib/languages/sql";
import go from "highlight.js/lib/languages/go";
import java from "highlight.js/lib/languages/java";
import c from "highlight.js/lib/languages/c";
import cpp from "highlight.js/lib/languages/cpp";
import yaml from "highlight.js/lib/languages/yaml";
import diff from "highlight.js/lib/languages/diff";
import markdown from "highlight.js/lib/languages/markdown";
import { toText } from "hast-util-to-text";
import { visit } from "unist-util-visit";
import type { Root, ElementContent, Element } from "hast";

const lowlight = createLowlight({
  javascript, typescript, python, rust, bash, json, css,
  html: xml, xml, sql, go, java, c, cpp, yaml,
  diff, markdown, toml: yaml,
});

function getLanguage(node: Element): string | false | undefined {
  const list = node.properties?.className;
  if (!Array.isArray(list)) return undefined;
  for (const cls of list) {
    const v = String(cls);
    if (v === "no-highlight" || v === "nohighlight") return false;
    if (v.startsWith("language-")) return v.slice(9);
    if (v.startsWith("lang-")) return v.slice(5);
  }
  return undefined;
}

export function rehypeHighlightLite() {
  return (tree: Root) => {
    visit(tree, "element", (node, _, parent) => {
      if (
        node.tagName !== "code" ||
        !parent ||
        parent.type !== "element" ||
        parent.tagName !== "pre"
      ) return;

      const lang = getLanguage(node);
      if (lang === false || !lang) return;

      if (!Array.isArray(node.properties.className)) {
        node.properties.className = [];
      }
      if (!node.properties.className.includes("hljs")) {
        node.properties.className.unshift("hljs");
      }

      const text = toText(node, { whitespace: "pre" });
      try {
        const result = lowlight.highlight(lang, text);
        if (result.children.length > 0) {
          node.children = result.children as ElementContent[];
        }
      } catch {
        // Unknown language — skip silently
      }
    });
  };
}
