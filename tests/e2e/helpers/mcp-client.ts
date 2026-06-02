/**
 * Thin wrapper around the MCP SDK to interact with tauri-mcp.
 * Provides typed methods matching the tauri-mcp tool surface.
 */

import { Client } from "@modelcontextprotocol/sdk/client/index.js";
import { StdioClientTransport } from "@modelcontextprotocol/sdk/client/stdio.js";

export interface McpClientConfig {
  /** Path to the tauri-mcp-server binary */
  serverBin?: string;
  /** Arguments to pass to the server */
  serverArgs?: string[];
  /** Port the Tauri app's MCP bridge listens on */
  port?: number;
  /** Host address */
  host?: string;
}

export class TauriMcpClient {
  private client: Client;
  private transport: StdioClientTransport | null = null;

  constructor() {
    this.client = new Client(
      { name: "xiaolin-e2e", version: "0.0.1" },
      { capabilities: {} },
    );
  }

  async connect(config: McpClientConfig = {}): Promise<void> {
    const bin = config.serverBin ?? "tauri-mcp-server";
    const args = config.serverArgs ?? [];

    this.transport = new StdioClientTransport({
      command: bin,
      args,
    });

    await this.client.connect(this.transport);
  }

  async disconnect(): Promise<void> {
    await this.client.close();
  }

  private async callTool(name: string, args: Record<string, unknown>): Promise<unknown> {
    const result = await this.client.callTool({ name, arguments: args });
    if (result.isError) {
      throw new Error(`MCP tool ${name} failed: ${JSON.stringify(result.content)}`);
    }
    const content = result.content as Array<{ type: string; text?: string }> | undefined;
    const textContent = content?.find((c) => c.type === "text");
    if (textContent?.text) {
      try {
        return JSON.parse(textContent.text);
      } catch {
        return textContent.text;
      }
    }
    return result.content;
  }

  // ─── Session management ────────────────────────────────────────────

  async startSession(port?: number, host?: string): Promise<void> {
    await this.callTool("driver_session", {
      action: "start",
      ...(port && { port }),
      ...(host && { host }),
    });
  }

  async stopSession(): Promise<void> {
    await this.callTool("driver_session", { action: "stop" });
  }

  async sessionStatus(): Promise<unknown> {
    return this.callTool("driver_session", { action: "status" });
  }

  // ─── DOM interaction ───────────────────────────────────────────────

  async click(selector: string, strategy: "css" | "xpath" | "text" = "css"): Promise<void> {
    await this.callTool("webview_interact", { action: "click", selector, strategy });
  }

  async doubleClick(selector: string, strategy: "css" | "xpath" | "text" = "css"): Promise<void> {
    await this.callTool("webview_interact", { action: "double-click", selector, strategy });
  }

  async scroll(selector: string, scrollY: number): Promise<void> {
    await this.callTool("webview_interact", { action: "scroll", selector, scrollY });
  }

  async focus(selector: string, strategy: "css" | "xpath" | "text" = "css"): Promise<void> {
    await this.callTool("webview_interact", { action: "focus", selector, strategy });
  }

  // ─── Keyboard ──────────────────────────────────────────────────────

  async type(selector: string, text: string, strategy: "css" | "xpath" | "text" = "css"): Promise<void> {
    await this.callTool("webview_keyboard", { action: "type", selector, text, strategy });
  }

  async press(key: string, modifiers?: string[]): Promise<void> {
    await this.callTool("webview_keyboard", {
      action: "press",
      key,
      ...(modifiers && { modifiers }),
    });
  }

  // ─── Waiting ───────────────────────────────────────────────────────

  async waitForSelector(selector: string, timeout = 10000, strategy: "css" | "xpath" | "text" = "css"): Promise<void> {
    await this.callTool("webview_wait_for", { type: "selector", value: selector, timeout, strategy });
  }

  async waitForText(text: string, timeout = 10000): Promise<void> {
    await this.callTool("webview_wait_for", { type: "text", value: text, timeout });
  }

  // ─── DOM inspection ────────────────────────────────────────────────

  async snapshot(type: "accessibility" | "structure" = "accessibility", selector?: string): Promise<string> {
    const args: Record<string, unknown> = { type };
    if (selector) args.selector = selector;
    return (await this.callTool("webview_dom_snapshot", args)) as string;
  }

  async findElement(selector: string, strategy: "css" | "xpath" | "text" = "css"): Promise<string> {
    return (await this.callTool("webview_find_element", { selector, strategy })) as string;
  }

  // ─── JavaScript execution ──────────────────────────────────────────

  async executeJs<T = unknown>(script: string): Promise<T> {
    return (await this.callTool("webview_execute_js", { script })) as T;
  }

  // ─── Screenshots ───────────────────────────────────────────────────

  async screenshot(filePath?: string): Promise<unknown> {
    const args: Record<string, unknown> = { format: "png" };
    if (filePath) args.filePath = filePath;
    return this.callTool("webview_screenshot", args);
  }

  // ─── Console logs ──────────────────────────────────────────────────

  async readConsoleLogs(lines = 50, filter?: string): Promise<unknown> {
    const args: Record<string, unknown> = { source: "console", lines };
    if (filter) args.filter = filter;
    return this.callTool("read_logs", args);
  }

  // ─── IPC ───────────────────────────────────────────────────────────

  async ipcCommand(command: string, ipcArgs?: Record<string, unknown>): Promise<unknown> {
    const params: Record<string, unknown> = { command };
    if (ipcArgs) params.args = ipcArgs;
    return this.callTool("ipc_execute_command", params);
  }

  // ─── Window management ─────────────────────────────────────────────

  async listWindows(): Promise<unknown> {
    return this.callTool("manage_window", { action: "list" });
  }
}
