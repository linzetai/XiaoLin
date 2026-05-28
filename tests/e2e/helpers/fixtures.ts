/**
 * Test fixtures — setup/teardown utilities and test data.
 */

import { TauriMcpClient } from "./mcp-client.js";
import { execSync } from "node:child_process";
import { mkdirSync, writeFileSync, rmSync, existsSync, readFileSync } from "node:fs";
import path from "node:path";

/** Base directory for test artifacts. */
export const E2E_WORK_DIR = "/tmp/fastclaw-e2e";

/** Ensure a clean working directory for a suite. */
export function setupSuiteDir(suiteName: string): string {
  const dir = path.join(E2E_WORK_DIR, suiteName);
  if (existsSync(dir)) {
    rmSync(dir, { recursive: true });
  }
  mkdirSync(dir, { recursive: true });
  return dir;
}

/** Create a test file with given content. */
export function createTestFile(dir: string, name: string, content: string): string {
  const filePath = path.join(dir, name);
  mkdirSync(path.dirname(filePath), { recursive: true });
  writeFileSync(filePath, content, "utf-8");
  return filePath;
}

/** Read a test file (for post-execution verification). */
export function readTestFile(filePath: string): string {
  return readFileSync(filePath, "utf-8");
}

/** Check if a file exists. */
export function fileExists(filePath: string): boolean {
  return existsSync(filePath);
}

/** Clean up a suite's directory. */
export function teardownSuiteDir(suiteName: string): void {
  const dir = path.join(E2E_WORK_DIR, suiteName);
  if (existsSync(dir)) {
    rmSync(dir, { recursive: true });
  }
}

/** Wait for a condition to become true, with polling. */
export async function waitUntil(
  condition: () => Promise<boolean>,
  timeoutMs = 10_000,
  intervalMs = 200,
): Promise<void> {
  const deadline = Date.now() + timeoutMs;
  while (Date.now() < deadline) {
    if (await condition()) return;
    await sleep(intervalMs);
  }
  throw new Error(`waitUntil timed out after ${timeoutMs}ms`);
}

export function sleep(ms: number): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, ms));
}

/** Prompts that are useful for testing. */
export const PROMPTS = {
  SIMPLE_GREETING: "你好，请简短回答：1+1等于几？",
  READ_FILE: (path: string) => `读取 ${path} 的内容并告诉我里面写了什么`,
  WRITE_FILE: (path: string, content: string) =>
    `创建文件 ${path}，内容为：${content}`,
  EDIT_FILE: (path: string, oldText: string, newText: string) =>
    `编辑文件 ${path}，把 "${oldText}" 替换为 "${newText}"`,
  LIST_DIR: (path: string) => `列出 ${path} 目录下的所有文件和文件夹`,
  GLOB: (path: string, pattern: string) =>
    `在 ${path} 目录下找出所有匹配 ${pattern} 的文件`,
  SEARCH_CONTENT: (path: string, query: string) =>
    `在 ${path} 中搜索包含 "${query}" 的文件`,
  SHELL_EXEC: (cmd: string) => `执行命令: ${cmd}`,
  WEB_FETCH: (url: string) => `获取 ${url} 的内容`,
  REMEMBER: (fact: string) => `请记住这个信息：${fact}`,
  RECALL: (question: string) => question,
};
