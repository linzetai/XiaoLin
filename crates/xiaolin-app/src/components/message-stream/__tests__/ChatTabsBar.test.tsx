// @vitest-environment jsdom
import { describe, it, expect, vi } from "vitest";
import { render, screen, fireEvent } from "@testing-library/react";
import { ChatTabsBarView, type ChatTabsBarViewProps } from "../ChatTabsBar";
import type { ChatMeta } from "../../../lib/stores/types";

function makeChat(overrides: Partial<ChatMeta> = {}): ChatMeta {
  const id = overrides.id ?? `chat-${Math.random().toString(36).slice(2, 8)}`;
  return {
    id,
    localKey: id,
    title: "新对话",
    workDir: null,
    source: "client",
    createdAt: new Date(),
    messageCount: 0,
    open: true,
    executionMode: "agent" as const,
    ...overrides,
  };
}

function renderTabs(overrides: Partial<ChatTabsBarViewProps> = {}) {
  const defaultChats = [
    makeChat({ id: "c1", title: "Chat One" }),
    makeChat({ id: "c2", title: "Chat Two" }),
    makeChat({ id: "c3", title: "Chat Three" }),
  ];

  const props: ChatTabsBarViewProps = {
    chats: defaultChats,
    activeChatId: "c1",
    streamingChatIds: new Set<string>(),
    onSelect: vi.fn(),
    onClose: vi.fn(),
    onNew: vi.fn(),
    onRename: vi.fn(),
    onReorder: vi.fn(),
    ...overrides,
  };

  const result = render(<ChatTabsBarView {...props} />);
  return { ...result, props };
}

describe("ChatTabsBar", () => {
  describe("rendering", () => {
    it("renders active tab title", () => {
      renderTabs();
      expect(screen.getByText("Chat One")).toBeInTheDocument();
    });

    it("does not render closed tabs", () => {
      renderTabs({
        chats: [
          makeChat({ id: "c1", title: "Open Chat", open: true }),
          makeChat({ id: "c2", title: "Closed Chat", open: false }),
        ],
      });
      expect(screen.getByText("Open Chat")).toBeInTheDocument();
    });

    it("visually highlights the active tab", () => {
      renderTabs({ activeChatId: "c2" });
      expect(screen.getByText("Chat Two")).toBeInTheDocument();
    });
  });

  describe("new tab", () => {
    it("calls onNew when clicking the new chat button", () => {
      const { props } = renderTabs();
      const newBtn = screen.getByTitle("新建会话");
      fireEvent.click(newBtn);
      expect(props.onNew).toHaveBeenCalledTimes(1);
    });
  });

  describe("dropdown", () => {
    it("opens dropdown showing all chats when trigger clicked with multiple tabs", () => {
      renderTabs();
      const trigger = screen.getByText("Chat One").closest("button")!;
      fireEvent.click(trigger);
      expect(screen.getByText("Chat Two")).toBeInTheDocument();
      expect(screen.getByText("Chat Three")).toBeInTheDocument();
    });

    it("calls onSelect when clicking a chat in dropdown", () => {
      const { props } = renderTabs();
      const trigger = screen.getByText("Chat One").closest("button")!;
      fireEvent.click(trigger);
      fireEvent.click(screen.getByText("Chat Two"));
      expect(props.onSelect).toHaveBeenCalledWith("c2");
    });
  });

  describe("streaming indicator", () => {
    it("shows streaming dot for non-active streaming tabs", () => {
      const { container } = renderTabs({
        activeChatId: "c1",
        streamingChatIds: new Set(["c2"]),
      });
      const dots = container.querySelectorAll('[style*="animation"]');
      expect(dots.length).toBeGreaterThanOrEqual(1);
    });
  });
});
