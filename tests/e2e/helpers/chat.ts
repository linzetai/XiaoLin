/**
 * High-level chat interaction helpers built on top of TauriMcpClient.
 * Provides send-message, wait-for-reply, new-session, and related utilities.
 */

import { TauriMcpClient } from "./mcp-client.js";

const INPUT_SELECTOR = "textarea.mention-textarea";
const STREAMING_ATTR = "[data-streaming]";
const SEND_BUTTON = 'button[title="发送 ↩"]';
const STOP_BUTTON = 'button[title="停止生成"]';

export class ChatHelper {
  constructor(private mcp: TauriMcpClient) {}

  /** Send a message via the chat input and press Enter. */
  async sendMessage(text: string): Promise<void> {
    await this.mcp.click(INPUT_SELECTOR);
    await this.mcp.type(INPUT_SELECTOR, text);
    await sleep(100);
    await this.mcp.press("Enter");
  }

  /**
   * Wait for the assistant to finish streaming its reply.
   * Auto-approves any permission dialogs that appear during tool execution.
   * Returns the text content of the last assistant message.
   */
  async waitForReply(timeoutMs = 60_000): Promise<string> {
    const deadline = Date.now() + timeoutMs;

    // Wait for streaming to start (data-streaming="true")
    await this.mcp.waitForSelector('[data-streaming="true"]', 10_000).catch(() => {
      // May already be done by the time we check
    });

    // Poll until streaming finishes, auto-approving permission dialogs
    while (Date.now() < deadline) {
      // Auto-approve any permission dialogs
      await this.autoApprovePermission();

      const streaming = await this.mcp.executeJs<string>(
        `(() => {
          const el = document.querySelector('[data-streaming]');
          return el?.getAttribute('data-streaming') ?? 'false';
        })()`,
      );
      if (streaming === "false" || streaming === null) break;
      await sleep(500);
    }

    // Extract the last assistant message
    const content = await this.mcp.executeJs<string>(
      `(() => {
        const msgs = document.querySelectorAll('.markdown-body');
        if (msgs.length === 0) return '';
        return msgs[msgs.length - 1].textContent ?? '';
      })()`,
    );

    return content;
  }

  /** Send a message and wait for the full reply. */
  async sendAndWait(text: string, timeoutMs = 60_000): Promise<string> {
    await this.sendMessage(text);
    return this.waitForReply(timeoutMs);
  }

  /** Cancel the current generation by clicking the stop button. */
  async cancelGeneration(): Promise<void> {
    try {
      await this.mcp.click(STOP_BUTTON);
    } catch {
      // Button may not be visible if streaming already stopped
    }
  }

  /** Create a new chat session (Cmd/Ctrl+K). */
  async newSession(): Promise<void> {
    await this.mcp.press("k", ["Meta"]);
    await sleep(300);
  }

  /** Get the number of messages currently displayed. */
  async getMessageCount(): Promise<number> {
    return this.mcp.executeJs<number>(
      `(() => {
        const user = document.querySelectorAll('.user-bubble-content').length;
        const assistant = document.querySelectorAll('.markdown-body').length;
        return user + assistant;
      })()`,
    );
  }

  /** Get all tool cards displayed in the conversation. */
  async getToolCards(): Promise<string[]> {
    return this.mcp.executeJs<string[]>(
      `(() => {
        const cards = document.querySelectorAll('[class*="tool-card"], [class*="tool-use"]');
        return Array.from(cards).map(c => {
          return c.getAttribute('data-tool-name')
            || c.querySelector('[class*="tool-name"]')?.textContent
            || c.textContent?.substring(0, 80)
            || '';
        });
      })()`,
    );
  }

  /** Check if any error messages are displayed. */
  async hasError(): Promise<boolean> {
    return this.mcp.executeJs<boolean>(
      `(() => {
        const text = document.body.textContent ?? '';
        return text.includes('回合已中止') || text.includes('Error') || text.includes('error');
      })()`,
    );
  }

  /** Get the last assistant reply text (without waiting). */
  async getLastReply(): Promise<string> {
    return this.mcp.executeJs<string>(
      `(() => {
        const msgs = document.querySelectorAll('.markdown-body');
        if (msgs.length === 0) return '';
        return msgs[msgs.length - 1].textContent ?? '';
      })()`,
    );
  }

  /** Check if the app is currently streaming a response. */
  async isStreaming(): Promise<boolean> {
    return this.mcp.executeJs<boolean>(
      `(() => {
        const el = document.querySelector('[data-streaming]');
        return el?.getAttribute('data-streaming') === 'true';
      })()`,
    );
  }

  /**
   * Auto-approve any permission/approval dialog that is currently showing.
   * Clicks "本次全部批准" (approve all for this turn) if available.
   */
  async autoApprovePermission(): Promise<boolean> {
    return this.mcp.executeJs<boolean>(
      `(() => {
        const buttons = Array.from(document.querySelectorAll('button'));
        const approveAll = buttons.find(b => b.textContent?.includes('本次全部批准'));
        if (approveAll) {
          approveAll.click();
          return true;
        }
        const approve = buttons.find(b => b.textContent?.includes('批准') && !b.textContent?.includes('拒绝'));
        if (approve) {
          approve.click();
          return true;
        }
        return false;
      })()`,
    );
  }
}

function sleep(ms: number): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, ms));
}
