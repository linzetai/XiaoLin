import { memo, useState, useCallback, type ComponentPropsWithoutRef } from "react";
import Markdown from "react-markdown";
import remarkGfm from "remark-gfm";
import rehypeHighlight from "rehype-highlight";
import { Check, Copy } from "lucide-react";

interface MarkdownContentProps {
  content: string;
}

const remarkPlugins = [remarkGfm];
const rehypePlugins = [rehypeHighlight];

function sanitizeUrl(url: string): string {
  const trimmed = url.trim();
  if (
    trimmed.startsWith("/") ||
    trimmed.startsWith("./") ||
    trimmed.startsWith("../") ||
    trimmed.startsWith("#")
  ) {
    return trimmed;
  }
  if (trimmed.startsWith("data:image/")) {
    return trimmed;
  }
  try {
    const parsed = new URL(trimmed);
    const protocol = parsed.protocol.toLowerCase();
    if (
      protocol === "http:" ||
      protocol === "https:" ||
      protocol === "mailto:" ||
      protocol === "tel:"
    ) {
      return trimmed;
    }
  } catch {
    // Invalid URL should be treated as unsafe and neutralized.
  }
  return "#";
}

function CopyButton({ text }: { text: string }) {
  const [copied, setCopied] = useState(false);

  const handleCopy = useCallback(async () => {
    try {
      await navigator.clipboard.writeText(text);
      setCopied(true);
      setTimeout(() => setCopied(false), 2000);
    } catch { /* clipboard API may not be available */ }
  }, [text]);

  return (
    <button
      onClick={handleCopy}
      className="flex items-center gap-1 rounded-md px-2 py-1 text-[11px] font-medium transition-all duration-150 hover:bg-[var(--bg-hover)]"
      style={{
        color: copied ? "var(--green)" : "var(--fill-tertiary)",
        background: copied ? "color-mix(in srgb, var(--green) 10%, transparent)" : "transparent",
      }}
      title={copied ? "已复制" : "复制代码"}
      aria-label={copied ? "已复制" : "复制代码"}
    >
      {copied ? <Check size={12} strokeWidth={2} /> : <Copy size={12} strokeWidth={1.5} />}
      <span>{copied ? "已复制" : "复制"}</span>
    </button>
  );
}

function CodeBlock({ children, className, ...rest }: ComponentPropsWithoutRef<"code">) {
  const isInline = !className && typeof children === "string" && !children.includes("\n");
  if (isInline) {
    return <code className="md-inline-code" {...rest}>{children}</code>;
  }
  return <code className={className} {...rest}>{children}</code>;
}

function extractCodeInfo(children: React.ReactNode): { lang: string; text: string } {
  const child = (Array.isArray(children) ? children[0] : children) as
    React.ReactElement<{ className?: string; children?: React.ReactNode }> | undefined;
  if (!child?.props) return { lang: "", text: "" };

  const rawCls = child.props.className;
  const cls = Array.isArray(rawCls) ? rawCls.join(" ") : String(rawCls ?? "");

  let lang = "";
  const langMatch = cls.match(/\blanguage-(\S+)/);
  if (langMatch) {
    lang = langMatch[1];
  }

  const raw = child.props.children;
  const text = typeof raw === "string" ? raw.replace(/\n$/, "") : String(raw ?? "").replace(/\n$/, "");
  return { lang, text };
}

function PreBlock({ children, ...rest }: ComponentPropsWithoutRef<"pre">) {
  const { lang, text } = extractCodeInfo(children);
  return (
    <div className="md-code-block">
      <div className="md-code-header">
        {lang && <span className="md-code-lang">{lang}</span>}
        <CopyButton text={text} />
      </div>
      <pre {...rest}>{children}</pre>
    </div>
  );
}

function Link({
  href = "",
  children,
  ...rest
}: ComponentPropsWithoutRef<"a">) {
  const safeHref = sanitizeUrl(href);
  return (
    <a
      {...rest}
      href={safeHref}
      target="_blank"
      rel="noopener noreferrer nofollow"
    >
      {children}
    </a>
  );
}

function MarkdownImage({ src, alt, ...rest }: ComponentPropsWithoutRef<"img">) {
  const safeSrc = src ? sanitizeUrl(src) : "#";
  if (safeSrc === "#") return null;
  return (
    <img
      {...rest}
      src={safeSrc}
      alt={alt || ""}
      className="my-2 max-h-[400px] rounded-md object-contain"
      style={{ border: "0.5px solid var(--separator)", maxWidth: "100%" }}
      loading="lazy"
    />
  );
}

const components = {
  code: CodeBlock,
  pre: PreBlock,
  a: Link,
  img: MarkdownImage,
};

export const MarkdownContent = memo(function MarkdownContent({
  content,
}: MarkdownContentProps) {
  return (
    <div className="markdown-body">
      <Markdown
        remarkPlugins={remarkPlugins}
        rehypePlugins={rehypePlugins}
        components={components}
      >
        {content}
      </Markdown>
    </div>
  );
});
