import { useState } from "react";
import { X, Pencil, Check, AlertCircle, ArrowUp, ArrowDown } from "lucide-react";
import type { QueuedMessage } from "../../lib/agent-store";
import { ICON } from "../../lib/ui-tokens";

interface QueuePanelProps {
  queue: QueuedMessage[];
  onEdit: (id: string, content: string) => void;
  onRemove: (id: string) => void;
  onReorder: (fromIndex: number, toIndex: number) => void;
  onRetry: (id: string) => void;
}

function StatusBadge({ status }: { status: QueuedMessage["status"] }) {
  switch (status) {
    case "pending":
      return (
        <span
          className="rounded px-1.5 py-0.5 text-[10px] font-medium"
          style={{ background: "rgba(0,0,0,0.1)", color: "var(--fill-secondary)" }}
        >
          等待中
        </span>
      );
    case "sending":
      return (
        <span
          className="rounded px-1.5 py-0.5 text-[10px] font-medium"
          style={{ background: "var(--tint)", color: "#fff", opacity: 0.7 }}
        >
          发送中
        </span>
      );
    case "failed":
      return (
        <span
          className="flex items-center gap-0.5 rounded px-1.5 py-0.5 text-[10px] font-medium"
          style={{ background: "rgba(252,129,129,0.15)", color: "var(--red, #FC8181)" }}
        >
          <AlertCircle {...ICON.sm} />
          失败
        </span>
      );
  }
}

function QueueItem({
  item,
  index,
  total,
  onEdit,
  onRemove,
  onReorder,
  onRetry,
}: {
  item: QueuedMessage;
  index: number;
  total: number;
  onEdit: (id: string, content: string) => void;
  onRemove: (id: string) => void;
  onReorder: (fromIndex: number, toIndex: number) => void;
  onRetry: (id: string) => void;
}) {
  const [editing, setEditing] = useState(false);
  const [editContent, setEditContent] = useState(item.content);

  const handleSave = () => {
    if (editContent.trim() && editContent !== item.content) {
      onEdit(item.id, editContent.trim());
    }
    setEditing(false);
  };

  const handleCancel = () => {
    setEditContent(item.content);
    setEditing(false);
  };

  return (
    <div
      className="flex flex-col gap-1 rounded-lg px-2.5 py-2"
      style={{
        background: "var(--bg-secondary)",
        border: "0.5px solid var(--separator)",
      }}
    >
      <div className="flex items-center gap-2">
        <span
          className="flex h-5 w-5 shrink-0 items-center justify-center rounded text-[10px] font-bold tabular-nums"
          style={{ color: "var(--fill-tertiary)", background: "var(--bg-tertiary, rgba(0,0,0,0.05))" }}
        >
          {index + 1}
        </span>
        <div className="flex flex-col gap-0.5">
          <button
            onClick={() => onReorder(index, index - 1)}
            disabled={index === 0}
            className="flex h-4 w-4 cursor-pointer items-center justify-center rounded opacity-50 transition-opacity hover:opacity-100 disabled:cursor-default disabled:opacity-20"
            style={{ color: "var(--fill-tertiary)" }}
          >
            <ArrowUp {...ICON.sm} />
          </button>
          <button
            onClick={() => onReorder(index, index + 1)}
            disabled={index === total - 1}
            className="flex h-4 w-4 cursor-pointer items-center justify-center rounded opacity-50 transition-opacity hover:opacity-100 disabled:cursor-default disabled:opacity-20"
            style={{ color: "var(--fill-tertiary)" }}
          >
            <ArrowDown {...ICON.sm} />
          </button>
        </div>
        <StatusBadge status={item.status} />
        <div className="flex-1" />
        {item.status === "failed" && (
          <button
            onClick={() => onRetry(item.id)}
            className="rounded px-1.5 py-0.5 text-[10px] font-medium transition-colors hover:bg-[var(--bg-hover)]"
            style={{ color: "var(--tint)" }}
          >
            重试
          </button>
        )}
        {!editing && item.status === "pending" && (
          <button
            onClick={() => setEditing(true)}
            className="flex h-5 w-5 cursor-pointer items-center justify-center rounded transition-colors hover:bg-[var(--bg-hover)]"
            style={{ color: "var(--fill-tertiary)" }}
          >
            <Pencil {...ICON.sm} />
          </button>
        )}
        <button
          onClick={() => onRemove(item.id)}
          className="flex h-5 w-5 cursor-pointer items-center justify-center rounded transition-colors hover:bg-[var(--bg-hover)]"
          style={{ color: "var(--fill-tertiary)" }}
        >
          <X {...ICON.sm} />
        </button>
      </div>
      {editing ? (
        <div className="flex items-center gap-1.5">
          <input
            type="text"
            value={editContent}
            onChange={(e) => setEditContent(e.target.value)}
            className="flex-1 rounded px-2 py-1 text-[11px] outline-none"
            style={{
              background: "var(--bg-primary)",
              border: "0.5px solid var(--tint)",
              color: "var(--fill-primary)",
            }}
            autoFocus
            onKeyDown={(e) => {
              if (e.key === "Enter") handleSave();
              if (e.key === "Escape") handleCancel();
            }}
          />
          <button
            onClick={handleSave}
            className="flex h-5 w-5 cursor-pointer items-center justify-center rounded"
            style={{ background: "var(--tint)", color: "#fff" }}
          >
            <Check {...ICON.sm} />
          </button>
        </div>
      ) : (
        <div
          className="truncate text-[11px]"
          style={{ color: "var(--fill-primary)" }}
        >
          {item.content}
        </div>
      )}
      {item.error && (
        <div className="text-[10px]" style={{ color: "var(--red, #FC8181)" }}>
          {item.error}
        </div>
      )}
    </div>
  );
}

export function QueuePanel({
  queue,
  onEdit,
  onRemove,
  onReorder,
  onRetry,
}: QueuePanelProps) {
  if (queue.length === 0) return null;

  return (
    <div
      className="flex flex-col gap-1.5 overflow-auto px-3 py-2"
      style={{
        background: "var(--bg-elevated)",
        borderBottom: "0.5px solid var(--separator)",
        maxHeight: "200px",
      }}
    >
      {queue.map((item, index) => (
        <QueueItem
          key={item.id}
          item={item}
          index={index}
          total={queue.length}
          onEdit={onEdit}
          onRemove={onRemove}
          onReorder={onReorder}
          onRetry={onRetry}
        />
      ))}
    </div>
  );
}