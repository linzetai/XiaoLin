/**
 * Test assertion utilities for e2e tests.
 */

import { TauriMcpClient } from "./mcp-client.js";
import { ChatHelper } from "./chat.js";

export class AssertionError extends Error {
  constructor(message: string) {
    super(message);
    this.name = "AssertionError";
  }
}

/** Assert a string contains an expected substring (case-insensitive). */
export function assertContains(actual: string, expected: string, message?: string): void {
  if (!actual.toLowerCase().includes(expected.toLowerCase())) {
    throw new AssertionError(
      message ?? `Expected text to contain "${expected}", but got: "${actual.substring(0, 200)}..."`,
    );
  }
}

/** Assert a string does NOT contain a substring. */
export function assertNotContains(actual: string, unwanted: string, message?: string): void {
  if (actual.toLowerCase().includes(unwanted.toLowerCase())) {
    throw new AssertionError(
      message ?? `Expected text NOT to contain "${unwanted}", but it does`,
    );
  }
}

/** Assert a value is truthy. */
export function assertTrue(value: unknown, message?: string): void {
  if (!value) {
    throw new AssertionError(message ?? `Expected truthy value, got: ${JSON.stringify(value)}`);
  }
}

/** Assert a value is falsy. */
export function assertFalse(value: unknown, message?: string): void {
  if (value) {
    throw new AssertionError(message ?? `Expected falsy value, got: ${JSON.stringify(value)}`);
  }
}

/** Assert two values are equal. */
export function assertEqual<T>(actual: T, expected: T, message?: string): void {
  if (actual !== expected) {
    throw new AssertionError(
      message ?? `Expected ${JSON.stringify(expected)}, got ${JSON.stringify(actual)}`,
    );
  }
}

/** Assert a number is greater than a threshold. */
export function assertGreaterThan(actual: number, threshold: number, message?: string): void {
  if (actual <= threshold) {
    throw new AssertionError(
      message ?? `Expected ${actual} > ${threshold}`,
    );
  }
}

/** Assert the agent called a specific tool (by checking tool cards in DOM). */
export async function assertToolCalled(chat: ChatHelper, toolName: string): Promise<void> {
  const cards = await chat.getToolCards();
  const found = cards.some(
    (c) => c.toLowerCase().includes(toolName.toLowerCase()),
  );
  if (!found) {
    throw new AssertionError(
      `Expected tool "${toolName}" to be called. Found tools: [${cards.join(", ")}]`,
    );
  }
}

/** Assert no error messages are present in the UI. */
export async function assertNoError(chat: ChatHelper): Promise<void> {
  const hasErr = await chat.hasError();
  if (hasErr) {
    const lastReply = await chat.getLastReply();
    throw new AssertionError(
      `Unexpected error in UI. Last reply: "${lastReply.substring(0, 200)}"`,
    );
  }
}

/** Assert the reply is not empty. */
export function assertNonEmpty(text: string, message?: string): void {
  if (!text || text.trim().length === 0) {
    throw new AssertionError(message ?? "Expected non-empty text");
  }
}

/** Assert a file exists on disk (via shell check through mcp JS exec). */
export async function assertFileExists(mcp: TauriMcpClient, path: string): Promise<void> {
  const exists = await mcp.executeJs<boolean>(
    `(async () => {
      try {
        const { exists } = await window.__TAURI__.fs;
        return await exists('${path.replace(/'/g, "\\'")}');
      } catch {
        return false;
      }
    })()`,
  );
  if (!exists) {
    throw new AssertionError(`Expected file to exist: ${path}`);
  }
}
