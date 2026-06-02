import { memo, Suspense, lazy } from "react";

const MarkdownContent = lazy(() =>
  import("./MarkdownContent").then((m) => ({ default: m.MarkdownContent })),
);

function hasUnclosedCodeBlock(text: string): boolean {
  let count = 0;
  let i = 0;
  while (i < text.length) {
    if (text[i] === '`' && text[i + 1] === '`' && text[i + 2] === '`') {
      count++;
      i += 3;
      while (i < text.length && text[i] === '`') i++;
    } else {
      i++;
    }
  }
  return count % 2 !== 0;
}

const FrozenMarkdown = memo(function FrozenMarkdown({ content }: { content: string }) {
  return (
    <Suspense fallback={<div className="animate-pulse rounded py-1" style={{ background: "var(--bg-tertiary)", height: 16 }} />}>
      <MarkdownContent content={content} />
    </Suspense>
  );
});

function ActiveLine({ text }: { text: string }) {
  return (
    <span className="markdown-body-active-line" style={{ whiteSpace: "pre-wrap" }}>
      {text}
    </span>
  );
}

export function StreamingMarkdown({ content }: { content: string }) {
  const lastNewline = content.lastIndexOf('\n');

  if (lastNewline <= 0 || hasUnclosedCodeBlock(content.slice(0, lastNewline))) {
    return (
      <Suspense fallback={<div className="animate-pulse rounded py-1" style={{ background: "var(--bg-tertiary)", height: 16 }} />}>
        <MarkdownContent content={content} streaming />
      </Suspense>
    );
  }

  const frozen = content.slice(0, lastNewline);
  const active = content.slice(lastNewline + 1);

  return (
    <>
      <FrozenMarkdown content={frozen} />
      {active && <ActiveLine text={active} />}
    </>
  );
}
