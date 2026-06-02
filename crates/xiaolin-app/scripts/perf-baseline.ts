#!/usr/bin/env npx tsx
/**
 * Performance baseline collection script.
 *
 * Connects to a running Vite dev server (localhost:1420) via Playwright,
 * executes the standard sequence defined in tasks.json, and writes a JSON
 * report to perf/<phase>-{before|after}.json.
 *
 * Usage:
 *   npx tsx scripts/perf-baseline.ts [--phase <name>] [--tag before|after]
 *
 * Requires: @playwright/test to be installed.
 */

import { chromium, type Page, type CDPSession, type Browser } from "@playwright/test";
import * as fs from "fs";
import * as path from "path";
import * as zlib from "zlib";

const ARGS = process.argv.slice(2);
function getArg(name: string, fallback: string): string {
  const idx = ARGS.indexOf(`--${name}`);
  return idx >= 0 && ARGS[idx + 1] ? ARGS[idx + 1] : fallback;
}

const PHASE = getArg("phase", "initial");
const TAG = getArg("tag", "before");
const BASE_URL = getArg("url", "http://localhost:1420");

interface PerfMetrics {
  phase: string;
  tag: string;
  timestamp: string;
  fcp: number | null;
  lcp: number | null;
  tti: number | null;
  buildChunks: ChunkInfo[];
  totalJsSize: number;
  totalJsGzip: number | null;
  memory: MemoryInfo | null;
  streamingFrameTime: FrameTimeInfo | null;
  reRenderCount: number | null;
}

interface ChunkInfo {
  name: string;
  size: number;
  gzip: number | null;
}

interface MemoryInfo {
  usedJSHeapSize: number;
  totalJSHeapSize: number;
  jsHeapSizeLimit: number;
}

interface FrameTimeInfo {
  p50: number;
  p95: number;
  max: number;
  samples: number;
}

async function collectWebVitals(page: Page): Promise<{ fcp: number | null; lcp: number | null }> {
  return page.evaluate(`
    new Promise(function(resolve) {
      var fcp = null;
      var lcp = null;

      try {
        var paintEntries = performance.getEntriesByType("paint");
        for (var i = 0; i < paintEntries.length; i++) {
          if (paintEntries[i].name === "first-contentful-paint") fcp = paintEntries[i].startTime;
        }
      } catch(e) {}

      try {
        var po = new PerformanceObserver(function(list) {
          var entries = list.getEntries();
          for (var j = 0; j < entries.length; j++) {
            if (entries[j].entryType === "paint" && entries[j].name === "first-contentful-paint") {
              fcp = entries[j].startTime;
            }
            if (entries[j].entryType === "largest-contentful-paint") {
              lcp = entries[j].startTime;
            }
          }
        });
        try { po.observe({ type: "paint", buffered: true }); } catch(e) {}
        try { po.observe({ type: "largest-contentful-paint", buffered: true }); } catch(e) {}
      } catch(e) {}

      setTimeout(function() {
        try { po.disconnect(); } catch(e) {}
        resolve({ fcp: fcp, lcp: lcp });
      }, 3000);
    })
  `);
}

async function collectTTI(page: Page): Promise<number | null> {
  return page.evaluate(`
    (function() {
      var nav = performance.getEntriesByType("navigation")[0];
      if (nav && nav.domInteractive) return nav.domInteractive;
      return null;
    })()
  `);
}

async function collectMemory(page: Page): Promise<MemoryInfo | null> {
  return page.evaluate(`
    (function() {
      var mem = performance.memory;
      if (!mem) return null;
      return {
        usedJSHeapSize: mem.usedJSHeapSize,
        totalJSHeapSize: mem.totalJSHeapSize,
        jsHeapSizeLimit: mem.jsHeapSizeLimit
      };
    })()
  `);
}

async function collectBuildChunks(): Promise<ChunkInfo[]> {
  const distDir = path.resolve(__dirname, "../dist/assets");
  if (!fs.existsSync(distDir)) return [];

  const files = fs.readdirSync(distDir).filter((f) => f.endsWith(".js"));
  return files.map((name) => {
    const content = fs.readFileSync(path.join(distDir, name));
    const gzipped = zlib.gzipSync(content);
    return { name, size: content.length, gzip: gzipped.length };
  });
}

async function collectFrameTimes(page: Page, cdp: CDPSession): Promise<FrameTimeInfo | null> {
  await cdp.send("Performance.enable");

  const samples: number[] = await page.evaluate(`
    new Promise(function(resolve) {
      var times = [];
      var last = performance.now();
      var count = 0;
      function tick() {
        var now = performance.now();
        times.push(now - last);
        last = now;
        count++;
        if (count < 60) requestAnimationFrame(tick);
        else resolve(times);
      }
      requestAnimationFrame(tick);
    })
  `);

  if (!samples || samples.length === 0) return null;

  const sorted = [...samples].sort((a, b) => a - b);
  return {
    p50: sorted[Math.floor(sorted.length * 0.5)],
    p95: sorted[Math.floor(sorted.length * 0.95)],
    max: sorted[sorted.length - 1],
    samples: sorted.length,
  };
}

function percentile(arr: number[], p: number): number {
  const sorted = [...arr].sort((a, b) => a - b);
  return sorted[Math.floor(sorted.length * p)] ?? 0;
}

async function main() {
  console.log(`\n=== Perf Baseline: phase=${PHASE}, tag=${TAG} ===\n`);

  const browser: Browser = await chromium.launch({
    args: ["--disable-extensions", "--no-sandbox"],
  });
  const context = await browser.newContext();
  const page = await context.newPage();
  const cdp: CDPSession = await context.newCDPSession(page);

  try {
    // --- Mock gateway for non-Tauri mode ---
    await page.addInitScript(() => {
      class MockWS {
        static CONNECTING = 0;
        static OPEN = 1;
        static CLOSING = 2;
        static CLOSED = 3;
        readyState = 0;
        CONNECTING = 0; OPEN = 1; CLOSING = 2; CLOSED = 3;
        url: string;
        onopen: any = null; onclose: any = null; onmessage: any = null; onerror: any = null;
        constructor(url: string) {
          this.url = url;
          queueMicrotask(() => { this.readyState = 1; this.onopen?.(new Event("open")); });
        }
        send(d: string) {
          try { const m = JSON.parse(d); if (m.method === "ping") queueMicrotask(() => this.onmessage?.(new MessageEvent("message", { data: JSON.stringify({ id: m.id, type: "pong" }) }))); } catch {}
        }
        close() { this.readyState = 3; this.onclose?.(new CloseEvent("close", { code: 1000 })); }
        addEventListener() {}
        removeEventListener() {}
        dispatchEvent() { return true; }
      }
      (window as any).WebSocket = MockWS;
    });

    await page.route("http://127.0.0.1:18888/health", (route) =>
      route.fulfill({ status: 200, contentType: "application/json", body: '{"status":"ok"}' }),
    );
    await page.route("**/api/v1/agents", (route) =>
      route.fulfill({ status: 200, contentType: "application/json", body: JSON.stringify([{ agentId: "assistant", name: "Assistant", model: "gpt-4o" }]) }),
    );
    await page.route("**/api/v1/sessions*", (route) =>
      route.fulfill({ status: 200, contentType: "application/json", body: '{"sessions":[]}' }),
    );
    await page.route("**/api/v1/config/*", (route) =>
      route.fulfill({ status: 200, contentType: "application/json", body: '{"key":"onboarding","value":{"completed":true}}' }),
    );
    await page.route("**/api/v1/models", (route) =>
      route.fulfill({ status: 200, contentType: "application/json", body: '[{"id":"gpt-4o","name":"GPT-4o","provider":"openai_compatible"}]' }),
    );

    // --- Step 1: Cold start → FCP/LCP/TTI ---
    console.log("Step 1: Cold start metrics...");
    const t0 = Date.now();
    await page.goto(BASE_URL, { waitUntil: "domcontentloaded" });
    await page.waitForSelector("main", { state: "visible", timeout: 15_000 }).catch(() => {});
    const coldStartMs = Date.now() - t0;
    console.log(`  Cold start: ${coldStartMs}ms`);

    const vitals = await collectWebVitals(page);
    const tti = await collectTTI(page);
    const memory = await collectMemory(page);

    console.log(`  FCP: ${vitals.fcp?.toFixed(1) ?? "N/A"}ms`);
    console.log(`  LCP: ${vitals.lcp?.toFixed(1) ?? "N/A"}ms`);
    console.log(`  TTI: ${tti?.toFixed(1) ?? "N/A"}ms`);

    // --- Step 2: Frame time baseline ---
    console.log("Step 2: Frame time baseline...");
    const frameTimes = await collectFrameTimes(page, cdp);
    if (frameTimes) {
      console.log(`  Frame P50: ${frameTimes.p50.toFixed(2)}ms, P95: ${frameTimes.p95.toFixed(2)}ms`);
    }

    // --- Step 3: Build chunk analysis ---
    console.log("Step 3: Build chunks...");
    const chunks = await collectBuildChunks();
    const totalJs = chunks.reduce((acc, c) => acc + c.size, 0);
    const totalGzip = chunks.reduce((acc, c) => acc + (c.gzip ?? 0), 0);
    console.log(`  Total JS: ${(totalJs / 1024).toFixed(1)}KB raw, ${(totalGzip / 1024).toFixed(1)}KB gzip (${chunks.length} chunks)`);
    for (const c of chunks) {
      console.log(`    ${c.name}: ${(c.size / 1024).toFixed(1)}KB raw, ${((c.gzip ?? 0) / 1024).toFixed(1)}KB gzip`);
    }

    // --- Write report ---
    const report: PerfMetrics = {
      phase: PHASE,
      tag: TAG,
      timestamp: new Date().toISOString(),
      fcp: vitals.fcp,
      lcp: vitals.lcp,
      tti,
      buildChunks: chunks,
      totalJsSize: totalJs,
      totalJsGzip: totalGzip,
      memory,
      streamingFrameTime: frameTimes,
      reRenderCount: null,
    };

    const outDir = path.resolve(__dirname, "../perf");
    if (!fs.existsSync(outDir)) fs.mkdirSync(outDir, { recursive: true });
    const outFile = path.join(outDir, `${PHASE}-${TAG}.json`);
    fs.writeFileSync(outFile, JSON.stringify(report, null, 2));
    console.log(`\nReport written to ${outFile}`);

    // --- Gate checks (CI hard gate uses gzip sizes) ---
    console.log("\n--- Gate Checks ---");
    let pass = true;
    for (const c of chunks) {
      const gzipKB = (c.gzip ?? 0) / 1024;
      if (gzipKB > 80) {
        console.log(`  FAIL: ${c.name} = ${gzipKB.toFixed(1)}KB gzip > 80KB (hard gate)`);
        pass = false;
      }
    }
    if (vitals.fcp && vitals.fcp > 1500) {
      console.log(`  FAIL: FCP = ${vitals.fcp.toFixed(1)}ms > 1500ms (hard gate)`);
      pass = false;
    }
    if (frameTimes && frameTimes.p95 > 12) {
      console.log(`  FAIL: Frame P95 = ${frameTimes.p95.toFixed(2)}ms > 12ms (hard gate)`);
      pass = false;
    }
    const totalGzipKB = totalGzip / 1024;
    if (totalGzipKB > 400) {
      console.log(`  WARN: Total JS gzip = ${totalGzipKB.toFixed(1)}KB > 400KB (soft gate)`);
    }
    if (frameTimes && frameTimes.p50 > 5) {
      console.log(`  WARN: Frame P50 = ${frameTimes.p50.toFixed(2)}ms > 5ms (soft gate)`);
    }

    console.log(pass ? "\n✓ All hard gates passed" : "\n✗ Some hard gates failed");
    process.exitCode = pass ? 0 : 1;
  } finally {
    await browser.close();
  }
}

main().catch((e) => {
  console.error("Fatal:", e);
  process.exit(2);
});
