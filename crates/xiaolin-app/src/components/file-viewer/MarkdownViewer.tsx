import { useEffect, useRef, useCallback, memo } from "react";
import { open } from "@tauri-apps/plugin-shell";
import { MarkdownContent } from "../message-stream/MarkdownContent";
import { CodeViewer } from "./CodeViewer";
import { openLightbox } from "../common/ImageLightbox";
import { readBinaryForViewer, isTauri } from "../../lib/transport";

export interface MarkdownViewerProps {
  content: string;
  filePath: string;
  workDir: string;
  viewMode: "code" | "preview";
  line?: number;
  wordWrap?: boolean;
}

const OPENABLE_EXT =
  /\.(md|mdx|py|js|ts|tsx|jsx|rs|go|java|c|cpp|h|hpp|rb|php|sh|sql|css|html|vue|svelte|toml|json|ya?ml|xml|txt|env|cfg|ini|conf|log|dockerfile)$/i;

function isExternalUrl(href: string): boolean {
  return (
    /^https?:\/\//i.test(href) ||
    href.startsWith("mailto:") ||
    href.startsWith("tel:")
  );
}

function isLocalRelativeRef(ref: string): boolean {
  if (!ref || ref.startsWith("#") || ref.startsWith("data:") || ref.startsWith("blob:")) {
    return false;
  }
  return !isExternalUrl(ref);
}

/** Resolve a relative href/src against the markdown file's directory. */
function resolveRelativeFromFile(href: string, filePath: string): string {
  const trimmed = href.trim().split("#")[0]?.split("?")[0] ?? "";
  if (!trimmed) return "";

  if (trimmed.startsWith("/") || /^[A-Za-z]:[\\/]/.test(trimmed)) {
    return normalizePath(trimmed);
  }

  const base = filePath.includes("/")
    ? filePath.slice(0, filePath.lastIndexOf("/"))
    : "";
  const combined = base ? `${base}/${trimmed}` : trimmed;
  return normalizePath(combined);
}

function normalizePath(raw: string): string {
  const isAbsolute = raw.startsWith("/");
  const isWindows = /^[A-Za-z]:[\\/]/.test(raw);

  const parts = raw.split(/[/\\]/);
  const stack: string[] = [];

  for (const part of parts) {
    if (!part || part === ".") continue;
    if (part === "..") {
      if (stack.length > 0 && stack[stack.length - 1] !== "..") stack.pop();
      continue;
    }
    stack.push(part);
  }

  if (isAbsolute) return `/${stack.join("/")}`;
  if (isWindows && parts[0]?.includes(":")) {
    return `${parts[0]}/${stack.slice(1).join("/")}`.replace(/\\/g, "/");
  }
  return stack.join("/");
}

function base64ToBlobUrl(base64: string, mime: string): string {
  const binary = atob(base64);
  const bytes = new Uint8Array(binary.length);
  for (let i = 0; i < binary.length; i++) bytes[i] = binary.charCodeAt(i);
  return URL.createObjectURL(new Blob([bytes], { type: mime }));
}

function MarkdownPreview({
  content,
  filePath,
  workDir,
}: {
  content: string;
  filePath: string;
  workDir: string;
}) {
  const containerRef = useRef<HTMLDivElement>(null);
  const blobUrlsRef = useRef<string[]>([]);

  const handleLinkClick = useCallback(
    (e: MouseEvent) => {
      const anchor = (e.target as HTMLElement).closest("a");
      if (!anchor || !containerRef.current?.contains(anchor)) return;

      const href = anchor.getAttribute("href");
      if (!href || href === "#") return;

      if (href.startsWith("#")) {
        e.preventDefault();
        const id = decodeURIComponent(href.slice(1));
        const target = containerRef.current.querySelector(
          `[id="${CSS.escape(id)}"], [name="${CSS.escape(id)}"]`,
        );
        target?.scrollIntoView({ behavior: "smooth", block: "start" });
        return;
      }

      if (isExternalUrl(href)) {
        e.preventDefault();
        void open(href).catch((err) => {
          console.warn("[MarkdownViewer] failed to open external link:", href, err);
        });
        return;
      }

      if (isLocalRelativeRef(href)) {
        const resolved = resolveRelativeFromFile(href, filePath);
        if (OPENABLE_EXT.test(resolved)) {
          e.preventDefault();
          window.dispatchEvent(
            new CustomEvent("xiaolin:open-file", {
              detail: { path: resolved, workDir },
            }),
          );
        }
      }
    },
    [filePath, workDir],
  );

  useEffect(() => {
    const container = containerRef.current;
    if (!container) return;

    container.addEventListener("click", handleLinkClick);
    return () => container.removeEventListener("click", handleLinkClick);
  }, [handleLinkClick]);

  useEffect(() => {
    const container = containerRef.current;
    if (!container || !isTauri) return;

    let cancelled = false;
    blobUrlsRef.current = [];

    const loadLocalImages = async () => {
      const imgs = container.querySelectorAll<HTMLImageElement>(".markdown-body img");
      for (const img of imgs) {
        const src = img.getAttribute("src");
        if (!src || !isLocalRelativeRef(src)) continue;
        if (img.dataset.localLoaded === "true") continue;

        const resolved = resolveRelativeFromFile(src, filePath);
        try {
          const result = await readBinaryForViewer(resolved, workDir);
          if (cancelled) return;

          const blobUrl = base64ToBlobUrl(result.base64, result.mime);
          blobUrlsRef.current.push(blobUrl);
          img.src = blobUrl;
          img.dataset.localLoaded = "true";
          img.style.maxWidth = "100%";
          img.style.cursor = "pointer";
          img.onclick = (ev) => {
            ev.stopPropagation();
            openLightbox(blobUrl, img.alt || "");
          };
        } catch (err) {
          console.warn("[MarkdownViewer] failed to load image:", resolved, err);
        }
      }
    };

    const observer = new MutationObserver(() => {
      void loadLocalImages();
    });
    observer.observe(container, { childList: true, subtree: true });
    void loadLocalImages();

    return () => {
      cancelled = true;
      observer.disconnect();
      for (const url of blobUrlsRef.current) {
        URL.revokeObjectURL(url);
      }
      blobUrlsRef.current = [];
    };
  }, [content, filePath, workDir]);

  return (
    <div
      ref={containerRef}
      style={{
        flex: 1,
        minHeight: 0,
        overflow: "auto",
        padding: "12px 16px",
      }}
    >
      <MarkdownContent content={content} />
    </div>
  );
}

export const MarkdownViewer = memo(function MarkdownViewer({
  content,
  filePath,
  workDir,
  viewMode,
  line,
  wordWrap,
}: MarkdownViewerProps) {
  if (viewMode === "code") {
    return (
      <CodeViewer
        content={content}
        language="markdown"
        line={line}
        wordWrap={wordWrap}
      />
    );
  }

  return (
    <MarkdownPreview content={content} filePath={filePath} workDir={workDir} />
  );
});
