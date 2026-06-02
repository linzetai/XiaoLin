// @vitest-environment jsdom
import { describe, it, expect, vi } from "vitest";
import { render, screen, fireEvent } from "@testing-library/react";
import { ChatTabsBar, type ChatTabsBarProps } from "../ChatTabsBar";
import type { Chat } from "../../../lib/stores/types";

function makeChat(overrides: Partial<Chat> = {}): Chat {
  const id = overrides.id ?? `chat-${Math.random().toString(36).slice(2, 8)}`;
  return {
    id,
    localKey: id,
    title: "新对话",
    workDir: null,
    source: "client",
    stream: [],
    createdAt: new Date(),
    messageCount: 0,
    open: true,
    subAgentRuns: {},
    executionMode: "agent" as const,
    ...overrides,
  };
}

function renderTabs(overrides: Partial<ChatTabsBarProps> = {}) {
  const defaultChats = [
    makeChat({ id: "c1", title: "Chat One" }),
    makeChat({ id: "c2", title: "Chat Two" }),
    makeChat({ id: "c3", title: "Chat Three" }),
  ];

  const props: ChatTabsBarProps = {
    agentId: "main",
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

  const result = render(<ChatTabsBar {...props} />);
  return { ...result, props };
}

describe("ChatTabsBar", () => {
  // ═══════════════════════════════════════════════════════════════════
  // Tab rendering
  // ═══════════════════════════════════════════════════════════════════

  describe("rendering", () => {
    it("renders all open tabs", () => {
      renderTabs();
      expect(screen.getByText("Chat One")).toBeInTheDocument();
      expect(screen.getByText("Chat Two")).toBeInTheDocument();
      expect(screen.getByText("Chat Three")).toBeInTheDocument();
    });

    it("does not render closed tabs", () => {
      renderTabs({
        chats: [
          makeChat({ id: "c1", title: "Open Chat", open: true }),
          makeChat({ id: "c2", title: "Closed Chat", open: false }),
        ],
      });

      expect(screen.getByText("Open Chat")).toBeInTheDocument();
      expect(screen.queryByText("Closed Chat")).not.toBeInTheDocument();
    });

    it("visually highlights the active tab", () => {
      const { container } = renderTabs({ activeChatId: "c2" });

      const tabs = container.querySelectorAll('[class*="rounded-t-lg"]');
      const activeTab = Array.from(tabs).find((tab) =>
        tab.textContent?.includes("Chat Two"),
      ) as HTMLElement;
      expect(activeTab).toBeDefined();
      expect(activeTab.style.fontWeight).toBe("600");
    });
  });

  // ═══════════════════════════════════════════════════════════════════
  // Tab selection
  // ═══════════════════════════════════════════════════════════════════

  describe("tab selection", () => {
    it("calls onSelect when clicking a tab", () => {
      const { props } = renderTabs();

      fireEvent.click(screen.getByText("Chat Two"));
      expect(props.onSelect).toHaveBeenCalledWith("c2");
    });

    it("calls onSelect with correct id for each tab", () => {
      const { props } = renderTabs();

      fireEvent.click(screen.getByText("Chat Three"));
      expect(props.onSelect).toHaveBeenCalledWith("c3");
    });
  });

  // ═══════════════════════════════════════════════════════════════════
  // New tab
  // ═══════════════════════════════════════════════════════════════════

  describe("new tab", () => {
    it("calls onNew when clicking the new chat button", () => {
      const { props } = renderTabs();

      const newBtn = screen.getByTitle("新建会话");
      fireEvent.click(newBtn);
      expect(props.onNew).toHaveBeenCalledTimes(1);
    });
  });

  // ═══════════════════════════════════════════════════════════════════
  // Close tab
  // ═══════════════════════════════════════════════════════════════════

  describe("close tab", () => {
    it("calls onClose when clicking the close button", () => {
      const { props, container } = renderTabs();

      // Hover to make close button visible, then click
      const tab = screen.getByText("Chat One").closest('[class*="rounded-t-lg"]')!;
      fireEvent.mouseEnter(tab);

      // Find close buttons — they are the small X inside each tab
      const closeButtons = container.querySelectorAll('button');
      // Find the close button that belongs to the first tab
      const closeBtn = Array.from(closeButtons).find(
        (btn) => btn.closest('[class*="rounded-t-lg"]') === tab && btn !== tab,
      );
      expect(closeBtn).toBeDefined();
      fireEvent.click(closeBtn!);
      expect(props.onClose).toHaveBeenCalledWith("c1");
    });
  });

  // ═══════════════════════════════════════════════════════════════════
  // Rename tab (double-click)
  // ═══════════════════════════════════════════════════════════════════

  describe("rename tab", () => {
    it("enters edit mode on double-click and commits on Enter", () => {
      const { props } = renderTabs();

      const tab = screen.getByText("Chat One").closest('[class*="rounded-t-lg"]')!;
      fireEvent.doubleClick(tab);

      const input = screen.getByDisplayValue("Chat One");
      expect(input).toBeInTheDocument();

      fireEvent.change(input, { target: { value: "Renamed Chat" } });
      fireEvent.keyDown(input, { key: "Enter" });

      expect(props.onRename).toHaveBeenCalledWith("c1", "Renamed Chat");
    });

    it("cancels edit on Escape", () => {
      const { props } = renderTabs();

      const tab = screen.getByText("Chat One").closest('[class*="rounded-t-lg"]')!;
      fireEvent.doubleClick(tab);

      const input = screen.getByDisplayValue("Chat One");
      fireEvent.change(input, { target: { value: "Changed" } });
      fireEvent.keyDown(input, { key: "Escape" });

      // Should not have committed
      expect(props.onRename).not.toHaveBeenCalled();
    });

    it("commits edit on blur", () => {
      const { props } = renderTabs();

      const tab = screen.getByText("Chat One").closest('[class*="rounded-t-lg"]')!;
      fireEvent.doubleClick(tab);

      const input = screen.getByDisplayValue("Chat One");
      fireEvent.change(input, { target: { value: "Blurred Title" } });
      fireEvent.blur(input);

      expect(props.onRename).toHaveBeenCalledWith("c1", "Blurred Title");
    });
  });

  // ═══════════════════════════════════════════════════════════════════
  // Streaming indicator
  // ═══════════════════════════════════════════════════════════════════

  describe("streaming indicator", () => {
    it("shows streaming dot for non-active streaming tabs", () => {
      const { container } = renderTabs({
        activeChatId: "c1",
        streamingChatIds: new Set(["c2"]),
      });

      // The streaming indicator is a small dot with animation
      const dots = container.querySelectorAll('[style*="animation"]');
      // At least one should be the streaming indicator for c2
      expect(dots.length).toBeGreaterThanOrEqual(1);
    });
  });
});
