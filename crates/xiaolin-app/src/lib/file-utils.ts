/**
 * Pure utility functions for file-type detection and language mapping.
 * No UI / component dependencies — safe to import from stores and lib/ layer.
 */

const IMAGE_EXTENSIONS = new Set([
  "png",
  "jpg",
  "jpeg",
  "gif",
  "webp",
  "svg",
]);

export function isImagePath(path: string): boolean {
  const ext = path.split(".").pop()?.toLowerCase() ?? "";
  return IMAGE_EXTENSIONS.has(ext);
}

export function isSvgPath(path: string): boolean {
  return path.toLowerCase().endsWith(".svg");
}

export function formatFileSize(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
}

/**
 * Extension → internal language id.
 * Only languages that have a corresponding CM6 loader are included.
 * Languages without CM6 support fall through to plain text.
 */
export const EXT_TO_LANG: Record<string, string> = {
  ts: "typescript",
  tsx: "typescript",
  js: "javascript",
  jsx: "javascript",
  mjs: "javascript",
  cjs: "javascript",
  rs: "rust",
  py: "python",
  go: "go",
  java: "java",
  php: "php",
  c: "c",
  h: "c",
  cpp: "cpp",
  cc: "cpp",
  hpp: "cpp",
  css: "css",
  scss: "scss",
  less: "less",
  html: "html",
  htm: "html",
  xml: "xml",
  json: "json",
  yaml: "yaml",
  yml: "yaml",
  toml: "toml",
  md: "markdown",
  mdx: "markdown",
  sql: "sql",
  sh: "shell",
  bash: "shell",
  zsh: "shell",
};

export function languageFromExtension(ext: string): string {
  const normalized = ext.toLowerCase().replace(/^\./, "");
  return EXT_TO_LANG[normalized] ?? "plain";
}

export function languageFromPath(path: string): string {
  const name = path.split("/").pop() ?? "";
  const ext = name.includes(".") ? (name.split(".").pop()?.toLowerCase() ?? "") : "";
  return languageFromExtension(ext);
}
