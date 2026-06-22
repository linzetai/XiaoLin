import type { Extension } from "@codemirror/state";
import { StreamLanguage } from "@codemirror/language";

export { EXT_TO_LANG, languageFromExtension, languageFromPath } from "../../lib/file-utils";

const MAX_LANG_CACHE = 24;

type LanguageLoader = () => Promise<Extension>;

const LOADERS: Record<string, LanguageLoader> = {
  typescript: async () => {
    const { javascript } = await import("@codemirror/lang-javascript");
    return javascript({ typescript: true, jsx: true });
  },
  javascript: async () => {
    const { javascript } = await import("@codemirror/lang-javascript");
    return javascript({ jsx: true });
  },
  rust: async () => {
    const { rust } = await import("@codemirror/lang-rust");
    return rust();
  },
  python: async () => {
    const { python } = await import("@codemirror/lang-python");
    return python();
  },
  go: async () => {
    const { go } = await import("@codemirror/lang-go");
    return go();
  },
  java: async () => {
    const { java } = await import("@codemirror/lang-java");
    return java();
  },
  php: async () => {
    const { php } = await import("@codemirror/lang-php");
    return php();
  },
  c: async () => {
    const { cpp } = await import("@codemirror/lang-cpp");
    return cpp();
  },
  cpp: async () => {
    const { cpp } = await import("@codemirror/lang-cpp");
    return cpp();
  },
  css: async () => {
    const { css } = await import("@codemirror/lang-css");
    return css();
  },
  html: async () => {
    const { html } = await import("@codemirror/lang-html");
    return html();
  },
  xml: async () => {
    const { xml } = await import("@codemirror/lang-xml");
    return xml();
  },
  json: async () => {
    const { json } = await import("@codemirror/lang-json");
    return json();
  },
  yaml: async () => {
    const { yaml } = await import("@codemirror/lang-yaml");
    return yaml();
  },
  toml: async () => {
    const { yaml } = await import("@codemirror/lang-yaml");
    return yaml();
  },
  markdown: async () => {
    const { markdown } = await import("@codemirror/lang-markdown");
    return markdown();
  },
  sql: async () => {
    const { sql } = await import("@codemirror/lang-sql");
    return sql();
  },
  shell: async () => {
    const { shell } = await import("@codemirror/legacy-modes/mode/shell");
    return StreamLanguage.define(shell);
  },
  scss: async () => {
    const { css } = await import("@codemirror/lang-css");
    return css();
  },
  less: async () => {
    const { css } = await import("@codemirror/lang-css");
    return css();
  },
};

const cache = new Map<string, Extension>();
const accessOrder: string[] = [];

function touchCacheKey(key: string) {
  const idx = accessOrder.indexOf(key);
  if (idx >= 0) accessOrder.splice(idx, 1);
  accessOrder.push(key);
}

function evictIfNeeded() {
  while (cache.size >= MAX_LANG_CACHE && accessOrder.length > 0) {
    const oldest = accessOrder.shift();
    if (oldest) cache.delete(oldest);
  }
}

/** Load a CM6 language extension by language id; returns [] for plain/unknown. */
export async function loadLanguageExtension(language: string): Promise<Extension[]> {
  const lang = language === "plain" ? "" : language;
  if (!lang || !LOADERS[lang]) return [];

  const cached = cache.get(lang);
  if (cached) {
    touchCacheKey(lang);
    return [cached];
  }

  try {
    const ext = await LOADERS[lang]();
    evictIfNeeded();
    cache.set(lang, ext);
    accessOrder.push(lang);
    return [ext];
  } catch (e) {
    console.warn("[CodeViewer] failed to load language:", lang, e);
    return [];
  }
}
