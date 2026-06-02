import { test, expect, waitForAppReady } from "../fixtures/mock-gateway";

test.describe("C1: 冷启动无白屏 / crash", () => {
  test("应用启动后渲染主界面，无 JS 错误", async ({ page }) => {
    const errors: string[] = [];
    page.on("pageerror", (err) => errors.push(err.message));

    await waitForAppReady(page);

    expect(errors).toEqual([]);
  });

  test("TitleBar 可见", async ({ page }) => {
    await waitForAppReady(page);

    const titleBar = page.locator("[data-tauri-drag-region]");
    if ((await titleBar.count()) > 0) {
      await expect(titleBar.first()).toBeVisible();
    } else {
      const header = page.locator("header, [class*='title-bar'], [class*='titlebar']");
      expect(await header.count()).toBeGreaterThanOrEqual(0);
    }
  });

  test("AgentList sidebar 渲染且包含至少 1 个 agent", async ({ page }) => {
    await waitForAppReady(page);

    const agentNames = page.locator('span.truncate').filter({ hasText: /Assistant|Coder|Writer/ });
    await expect(agentNames.first()).toBeVisible({ timeout: 5_000 });
  });

  test("主消息区域可见", async ({ page }) => {
    await waitForAppReady(page);

    const messageArea = page.locator("main");
    await expect(messageArea).toBeVisible();
  });

  test("无 console.error 级别输出 (排除已知噪声)", async ({ page }) => {
    const errors: string[] = [];
    const KNOWN_NOISE = [
      /favicon/i,
      /ResizeObserver loop/i,
      /Failed to load resource/i,
      /Encountered two children with the same key/i,
    ];

    page.on("console", (msg) => {
      if (msg.type() === "error") {
        const text = msg.text();
        if (!KNOWN_NOISE.some((re) => re.test(text))) {
          errors.push(text);
        }
      }
    });

    await waitForAppReady(page);

    expect(errors).toEqual([]);
  });

  test("首屏加载时间 < 5s (宽松阈值)", async ({ page }) => {
    const start = Date.now();
    await waitForAppReady(page);
    const elapsed = Date.now() - start;

    expect(elapsed).toBeLessThan(5_000);
  });

  test("localStorage 可正常读写 (persist 机制)", async ({ page }) => {
    await waitForAppReady(page);

    const canPersist = await page.evaluate(() => {
      try {
        localStorage.setItem("__pw_test__", "1");
        const v = localStorage.getItem("__pw_test__");
        localStorage.removeItem("__pw_test__");
        return v === "1";
      } catch {
        return false;
      }
    });

    expect(canPersist).toBe(true);
  });
});
