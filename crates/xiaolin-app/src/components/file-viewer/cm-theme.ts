import { HighlightStyle, syntaxHighlighting } from "@codemirror/language";
import { EditorView } from "@codemirror/view";
import { tags as t } from "@lezer/highlight";

/** CM6 theme using XiaoLin CSS variables (matches hljs token colors). */
export function createXiaolinTheme() {
  return EditorView.theme(
    {
      "&": {
        color: "var(--fill-primary)",
        backgroundColor: "var(--code-bg)",
        height: "100%",
      },
      ".cm-scroller": {
        overflow: "auto",
        fontFamily: "var(--font-mono)",
      },
      ".cm-content": {
        fontFamily: "var(--font-mono)",
        fontSize: "var(--code-font-size)",
        caretColor: "var(--fill-primary)",
        padding: "4px 0",
      },
      ".cm-gutters": {
        backgroundColor: "var(--code-bg)",
        color: "var(--fill-quaternary)",
        borderRight: "0.5px solid var(--separator)",
        fontFamily: "var(--font-mono)",
        fontSize: "var(--code-font-size)",
      },
      ".cm-activeLineGutter": {
        backgroundColor: "color-mix(in srgb, var(--fill-quaternary) 10%, transparent)",
      },
      ".cm-lineNumbers .cm-gutterElement": {
        padding: "0 8px 0 4px",
        minWidth: "3.5ch",
      },
      ".cm-foldGutter span": {
        cursor: "pointer",
        color: "var(--fill-quaternary)",
      },
      ".cm-selectionBackground, &.cm-focused .cm-selectionBackground": {
        backgroundColor: "var(--selection-bg) !important",
      },
      "&.cm-focused .cm-cursor": {
        borderLeftColor: "var(--fill-primary)",
      },
      ".cm-searchMatch": {
        backgroundColor: "color-mix(in srgb, var(--selection-bg) 70%, transparent)",
        outline: "1px solid var(--separator)",
      },
      ".cm-searchMatch.cm-searchMatch-selected": {
        backgroundColor: "var(--selection-bg)",
      },
      ".cm-panels": {
        backgroundColor: "var(--bg-secondary)",
        color: "var(--fill-primary)",
      },
      ".cm-panels.cm-panels-top": {
        borderBottom: "0.5px solid var(--separator)",
      },
      ".cm-textfield": {
        backgroundColor: "var(--bg-primary)",
        color: "var(--fill-primary)",
        border: "0.5px solid var(--separator)",
        borderRadius: 4,
        fontFamily: "var(--font-mono)",
        fontSize: "var(--code-font-size)",
      },
      ".cm-button": {
        backgroundColor: "var(--bg-tertiary)",
        color: "var(--fill-primary)",
        border: "0.5px solid var(--separator)",
        borderRadius: 4,
        fontSize: "var(--code-font-size)",
      },
      ".cm-line-highlight": {
        backgroundColor: "color-mix(in srgb, var(--selection-bg) 90%, var(--code-bg))",
      },
      ".cm-line-highlight-fade": {
        backgroundColor: "transparent",
        transition: "background-color 1.5s ease-out",
      },
    },
    { dark: false },
  );
}

function createHighlightStyle() {
  return HighlightStyle.define([
    { tag: t.keyword, color: "var(--hljs-keyword)" },
    { tag: [t.name, t.deleted, t.character, t.macroName], color: "var(--hljs-variable)" },
    { tag: [t.propertyName], color: "var(--hljs-property)" },
    { tag: [t.processingInstruction, t.string, t.inserted, t.special(t.string)], color: "var(--hljs-string)" },
    { tag: [t.function(t.variableName), t.labelName], color: "var(--hljs-function)" },
    { tag: [t.color, t.constant(t.name), t.standard(t.name)], color: "var(--hljs-number)" },
    { tag: [t.definition(t.name), t.separator], color: "var(--hljs-variable)" },
    { tag: [t.className], color: "var(--hljs-class)" },
    { tag: [t.number, t.changed, t.annotation, t.modifier, t.self, t.namespace], color: "var(--hljs-number)" },
    { tag: [t.typeName], color: "var(--hljs-type)" },
    { tag: [t.operator, t.operatorKeyword], color: "var(--hljs-keyword)" },
    { tag: [t.url, t.escape, t.regexp, t.link], color: "var(--hljs-string)" },
    { tag: t.meta, color: "var(--hljs-meta)" },
    { tag: [t.comment, t.quote], color: "var(--hljs-comment)", fontStyle: "italic" },
    { tag: t.strong, fontWeight: "bold" },
    { tag: t.emphasis, fontStyle: "italic" },
    { tag: t.strikethrough, textDecoration: "line-through" },
    { tag: t.link, color: "var(--md-link)", textDecoration: "underline" },
    { tag: t.heading, fontWeight: "bold", color: "var(--hljs-function)" },
    { tag: [t.atom, t.bool], color: "var(--hljs-built-in)" },
    { tag: t.invalid, color: "var(--hljs-class)" },
    { tag: [t.tagName], color: "var(--hljs-tag)" },
    { tag: [t.attributeName], color: "var(--hljs-attr)" },
  ]);
}

export function createThemeExtensions() {
  return [createXiaolinTheme(), syntaxHighlighting(createHighlightStyle())];
}
