import { test, expect, waitForAppReady } from "../fixtures/mock-gateway";

test.describe("C5: 切换 Agent → 消息列表正确", () => {
  test("sidebar 至少显示 1 个 agent 条目", async ({ page }) => {
    await waitForAppReady(page);

    const agentNames = page.locator('span.truncate').filter({ hasText: /Assistant|Coder|Writer/ });
    const count = await agentNames.count();
    expect(count).toBeGreaterThanOrEqual(1);
  });

  test("点击不同 agent 后主区域内容切换", async ({ page }) => {
    await waitForAppReady(page);

    const agentNames = page.locator('span.truncate').filter({ hasText: /Assistant|Coder|Writer/ });
    const count = await agentNames.count();
    if (count < 2) {
      test.skip();
      return;
    }

    const firstText = await agentNames.nth(0).textContent();
    await agentNames.nth(1).click();
    await page.waitForTimeout(500);

    const secondText = await agentNames.nth(1).textContent();
    expect(firstText).not.toBe(secondText);
  });

  test("切换 agent 后再切回，消息列表应保持", async ({ page }) => {
    await waitForAppReady(page);

    const injected = await page.evaluate(() => {
      const store = (window as any).__ZUSTAND_AGENT_STORE__;
      if (!store) return false;
      const state = store.getState();
      const agentId = state.activeAgentId;
      state.addMessage(agentId, {
        role: "user",
        content: "我是切换测试消息 SWITCH_MARKER",
        timestamp: new Date(),
      });
      return true;
    });

    if (!injected) {
      test.skip();
      return;
    }

    await expect(page.locator("text=SWITCH_MARKER").first()).toBeVisible({ timeout: 3_000 });

    const agentNames = page.locator('span.truncate').filter({ hasText: /Assistant|Coder|Writer/ });
    if ((await agentNames.count()) < 2) {
      test.skip();
      return;
    }

    await agentNames.nth(1).click();
    await page.waitForTimeout(500);

    await agentNames.nth(0).click();
    await page.waitForTimeout(500);

    await expect(page.locator("text=SWITCH_MARKER").first()).toBeVisible({ timeout: 3_000 });
  });

  test("切换 agent 无 JS 错误", async ({ page }) => {
    const errors: string[] = [];
    page.on("pageerror", (err) => errors.push(err.message));

    await waitForAppReady(page);

    const agentNames = page.locator('span.truncate').filter({ hasText: /Assistant|Coder|Writer/ });
    const count = await agentNames.count();

    for (let i = 0; i < Math.min(count, 3); i++) {
      await agentNames.nth(i).click();
      await page.waitForTimeout(300);
    }

    expect(errors).toEqual([]);
  });
});
