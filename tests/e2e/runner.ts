/**
 * E2E test runner for FastClaw agent usability tests.
 *
 * Usage:
 *   tsx runner.ts                        # Run all suites
 *   tsx runner.ts --suite 01-basic-chat   # Run a specific suite
 *   tsx runner.ts --suite 02 --suite 04   # Run multiple suites
 */

import { TauriMcpClient } from "./helpers/mcp-client.js";
import { ChatHelper } from "./helpers/chat.js";
import { sleep } from "./helpers/fixtures.js";

// ─── Test case definition ────────────────────────────────────────────────

export interface TestCase {
  name: string;
  fn: (ctx: TestContext) => Promise<void>;
}

export interface TestSuite {
  name: string;
  setup?: (ctx: TestContext) => Promise<void>;
  teardown?: (ctx: TestContext) => Promise<void>;
  cases: TestCase[];
}

export interface TestContext {
  mcp: TauriMcpClient;
  chat: ChatHelper;
}

export interface TestResult {
  suite: string;
  case: string;
  passed: boolean;
  error?: string;
  durationMs: number;
}

// ─── Suite registry ──────────────────────────────────────────────────────

const suiteModules: Record<string, () => Promise<{ default: TestSuite }>> = {
  "01-basic-chat": () => import("./suites/01-basic-chat.js"),
  "02-file-tools": () => import("./suites/02-file-tools.js"),
  "03-code-tools": () => import("./suites/03-code-tools.js"),
  "04-shell-tools": () => import("./suites/04-shell-tools.js"),
  "05-web-tools": () => import("./suites/05-web-tools.js"),
  "06-memory": () => import("./suites/06-memory.js"),
  "07-multi-turn": () => import("./suites/07-multi-turn.js"),
  "08-tool-search": () => import("./suites/08-tool-search.js"),
  "09-session-mgmt": () => import("./suites/09-session-mgmt.js"),
  "10-plan-mode": () => import("./suites/10-plan-mode.js"),
  "11-goal-todo": () => import("./suites/11-goal-todo.js"),
};

// ─── Runner logic ────────────────────────────────────────────────────────

async function runSuite(suite: TestSuite, ctx: TestContext): Promise<TestResult[]> {
  const results: TestResult[] = [];
  console.log(`\n${"═".repeat(60)}`);
  console.log(`  Suite: ${suite.name}`);
  console.log(`${"═".repeat(60)}`);

  if (suite.setup) {
    try {
      await suite.setup(ctx);
    } catch (e: any) {
      console.log(`  ✗ SETUP FAILED: ${e.message}`);
      return suite.cases.map((c) => ({
        suite: suite.name,
        case: c.name,
        passed: false,
        error: `Setup failed: ${e.message}`,
        durationMs: 0,
      }));
    }
  }

  for (const tc of suite.cases) {
    const start = Date.now();
    try {
      await tc.fn(ctx);
      const dur = Date.now() - start;
      results.push({ suite: suite.name, case: tc.name, passed: true, durationMs: dur });
      console.log(`  ✓ ${tc.name} (${dur}ms)`);
    } catch (e: any) {
      const dur = Date.now() - start;
      results.push({
        suite: suite.name,
        case: tc.name,
        passed: false,
        error: e.message,
        durationMs: dur,
      });
      console.log(`  ✗ ${tc.name} (${dur}ms)`);
      console.log(`    Error: ${e.message}`);
      // Take a screenshot on failure
      try {
        const screenshotPath = `/tmp/fastclaw-e2e/screenshots/${suite.name}-${tc.name.replace(/\s/g, "_")}.png`;
        await ctx.mcp.screenshot(screenshotPath);
      } catch { /* ignore screenshot failures */ }
    }
  }

  if (suite.teardown) {
    try {
      await suite.teardown(ctx);
    } catch (e: any) {
      console.log(`  ⚠ TEARDOWN WARNING: ${e.message}`);
    }
  }

  return results;
}

async function main(): Promise<void> {
  const args = process.argv.slice(2);
  let suiteFilter: string[] = [];

  for (let i = 0; i < args.length; i++) {
    if (args[i] === "--suite" && args[i + 1]) {
      suiteFilter.push(args[++i]);
    }
  }

  // Determine which suites to run
  const suiteKeys = suiteFilter.length > 0
    ? Object.keys(suiteModules).filter((k) => suiteFilter.some((f) => k.includes(f)))
    : Object.keys(suiteModules);

  if (suiteKeys.length === 0) {
    console.error("No matching suites found.");
    process.exit(1);
  }

  console.log("FastClaw E2E Test Runner");
  console.log(`Suites to run: ${suiteKeys.join(", ")}`);
  console.log("");

  // Connect to tauri-mcp
  const mcp = new TauriMcpClient();
  try {
    await mcp.connect();
  } catch (e: any) {
    console.error(`Failed to connect to tauri-mcp: ${e.message}`);
    console.error("Make sure the FastClaw app is running with the MCP bridge plugin enabled.");
    process.exit(1);
  }

  // Start driver session
  try {
    await mcp.startSession();
  } catch (e: any) {
    console.error(`Failed to start driver session: ${e.message}`);
    process.exit(1);
  }

  const chat = new ChatHelper(mcp);
  const ctx: TestContext = { mcp, chat };

  // Ensure screenshots directory exists
  const { mkdirSync } = await import("node:fs");
  mkdirSync("/tmp/fastclaw-e2e/screenshots", { recursive: true });

  // Run suites
  const allResults: TestResult[] = [];
  for (const key of suiteKeys) {
    try {
      const mod = await suiteModules[key]();
      const suite = mod.default;
      const results = await runSuite(suite, ctx);
      allResults.push(...results);
    } catch (e: any) {
      console.error(`Failed to load suite "${key}": ${e.message}`);
    }
    await sleep(1000);
  }

  // Stop session and disconnect
  try {
    await mcp.stopSession();
    await mcp.disconnect();
  } catch { /* best-effort cleanup */ }

  // Print summary
  console.log(`\n${"═".repeat(60)}`);
  console.log("  SUMMARY");
  console.log(`${"═".repeat(60)}`);
  const passed = allResults.filter((r) => r.passed).length;
  const failed = allResults.filter((r) => !r.passed).length;
  const total = allResults.length;
  const totalDur = allResults.reduce((s, r) => s + r.durationMs, 0);

  console.log(`  Total: ${total} | Passed: ${passed} | Failed: ${failed} | Duration: ${(totalDur / 1000).toFixed(1)}s`);

  if (failed > 0) {
    console.log("\n  Failed tests:");
    for (const r of allResults.filter((r) => !r.passed)) {
      console.log(`    ✗ [${r.suite}] ${r.case}: ${r.error}`);
    }
    process.exit(1);
  }

  console.log("\n  All tests passed! ✓");
}

main().catch((e) => {
  console.error("Runner crashed:", e);
  process.exit(1);
});
