// @vitest-environment jsdom
import { describe, it, expect } from "vitest";
import { render, screen, fireEvent } from "@testing-library/react";
import { ToolCallCard, type ToolCall } from "../ToolCallCard";

function makeTool(overrides: Partial<ToolCall> = {}): ToolCall {
  return {
    id: "tc-1",
    name: "file_read",
    status: "success",
    args: '{"path":"src/main.rs"}',
    result: 'fn main() { println!("Hello"); }',
    duration: 120,
    ...overrides,
  };
}

describe("ToolCallCard", () => {
  // ═══════════════════════════════════════════════════════════════════
  // AC2: 默认折叠，显示工具名称和状态图标
  // ═══════════════════════════════════════════════════════════════════

  describe("default collapsed state", () => {
    it("renders tool name and status", () => {
      render(<ToolCallCard tool={makeTool()} />);

      expect(screen.getByText("读取文件")).toBeInTheDocument();
      expect(screen.getByText("src/main.rs")).toBeInTheDocument();
    });

    it("does not show args or result when collapsed", () => {
      render(<ToolCallCard tool={makeTool()} />);

      expect(screen.queryByText("参数")).not.toBeInTheDocument();
    });

    it("shows duration for completed tools", () => {
      render(<ToolCallCard tool={makeTool({ duration: 1500 })} />);

      expect(screen.getByText("1.5s")).toBeInTheDocument();
    });

    it("shows duration in ms for fast tools", () => {
      render(<ToolCallCard tool={makeTool({ duration: 85 })} />);

      expect(screen.getByText("85ms")).toBeInTheDocument();
    });
  });

  // ═══════════════════════════════════════════════════════════════════
  // AC2: 展开显示完整参数 JSON 和结果
  // ═══════════════════════════════════════════════════════════════════

  describe("expand / collapse", () => {
    it("expands to show args and result on click", () => {
      render(<ToolCallCard tool={makeTool()} />);

      const header = screen.getByText("读取文件").closest("button")!;
      fireEvent.click(header);

      expect(screen.getByText("参数")).toBeInTheDocument();
      expect(screen.getByText(/"path": "src\/main.rs"/)).toBeInTheDocument();
    });

    it("collapses back on second click", () => {
      render(<ToolCallCard tool={makeTool()} />);

      const header = screen.getByText("读取文件").closest("button")!;
      fireEvent.click(header); // expand
      expect(screen.getByText("参数")).toBeInTheDocument();

      fireEvent.click(header); // collapse
      expect(screen.queryByText("参数")).not.toBeInTheDocument();
    });

    it("does not expand when no args or result", () => {
      render(<ToolCallCard tool={makeTool({ args: undefined, result: undefined })} />);

      const header = screen.getByText("读取文件").closest("button")!;
      expect(header.getAttribute("aria-expanded")).toBeNull();
    });
  });

  // ═══════════════════════════════════════════════════════════════════
  // AC2: 状态流转 pending → running → done/error
  // ═══════════════════════════════════════════════════════════════════

  describe("status transitions", () => {
    it("renders running state with spinner", () => {
      const { container } = render(
        <ToolCallCard tool={makeTool({ status: "running", duration: undefined, startTime: Date.now() })} />,
      );

      // running spinner has animation: spin
      const spinner = container.querySelector('[style*="animation"]');
      expect(spinner).not.toBeNull();
    });

    it("renders success state with check icon", () => {
      render(<ToolCallCard tool={makeTool({ status: "success" })} />);

      // The check icon should be visible (no red styling)
      const card = screen.getByText("读取文件").closest('[class*="rounded-lg"]')!;
      expect(card.getAttribute("style")).not.toContain("var(--red)");
    });

    it("renders error state with error styling", () => {
      render(
        <ToolCallCard tool={makeTool({ status: "error", result: "File not found" })} />,
      );

      const card = screen.getByText("读取文件").closest('[class*="rounded-lg"]')!;
      expect(card.getAttribute("style")).toContain("var(--red)");
    });
  });

  // ═══════════════════════════════════════════════════════════════════
  // Tool type recognition
  // ═══════════════════════════════════════════════════════════════════

  describe("tool type labels", () => {
    const toolLabelMap: Array<[string, string]> = [
      ["file_read", "读取文件"],
      ["file_write", "写入文件"],
      ["shell", "执行命令"],
      ["web_search", "搜索网络"],
      ["edit_file", "编辑文件"],
    ];

    it.each(toolLabelMap)("recognizes %s as '%s'", (toolName, expectedLabel) => {
      render(<ToolCallCard tool={makeTool({ name: toolName })} />);
      expect(screen.getByText(expectedLabel)).toBeInTheDocument();
    });

    it("falls back to tool name for unknown tools", () => {
      render(<ToolCallCard tool={makeTool({ name: "custom_tool_xyz" })} />);
      expect(screen.getByText("custom_tool_xyz")).toBeInTheDocument();
    });

    it("recognizes MCP tools with server prefix", () => {
      render(<ToolCallCard tool={makeTool({ name: "mcp__github__list_repos" })} />);
      expect(screen.getByText("github/list_repos")).toBeInTheDocument();
    });
  });

  // ═══════════════════════════════════════════════════════════════════
  // Key info extraction
  // ═══════════════════════════════════════════════════════════════════

  describe("key info extraction", () => {
    it("extracts file path from args", () => {
      render(<ToolCallCard tool={makeTool({ args: '{"path":"src/lib.rs"}' })} />);
      expect(screen.getByText("src/lib.rs")).toBeInTheDocument();
    });

    it("extracts command from shell args", () => {
      render(<ToolCallCard tool={makeTool({ name: "shell", args: '{"command":"cargo test"}' })} />);
      expect(screen.getByText("cargo test")).toBeInTheDocument();
    });

    it("extracts query from search args", () => {
      render(<ToolCallCard tool={makeTool({ name: "web_search", args: '{"query":"Rust async"}' })} />);
      expect(screen.getByText("Rust async")).toBeInTheDocument();
    });
  });
});
