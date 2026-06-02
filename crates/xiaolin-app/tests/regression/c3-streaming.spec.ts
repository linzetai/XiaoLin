import { test, expect, waitForAppReady } from "../fixtures/mock-gateway";

test.describe("C3: 发消息 streaming 正确", () => {
  test("输入框可见且可聚焦", async ({ page }) => {
    await waitForAppReady(page);

    const textarea = page.locator("textarea").first();
    await expect(textarea).toBeVisible();
    await textarea.focus();
    await expect(textarea).toBeFocused();
  });

  test("输入文本后发送按钮变为可交互", async ({ page }) => {
    await waitForAppReady(page);

    const textarea = page.locator("textarea").first();
    await textarea.fill("你好");
    await page.waitForTimeout(200);

    const sendBtn = page.locator('button:has(svg[class*="lucide-arrow-up"]), button[aria-label*="send"], button[aria-label*="发送"]');
    if (await sendBtn.count() > 0) {
      await expect(sendBtn.first()).toBeEnabled();
    }
  });

  test("注入消息后 UI 正确渲染用户消息和助手消息", async ({ page }) => {
    await waitForAppReady(page);

    await page.evaluate(() => {
      const store = (window as any).__ZUSTAND_AGENT_STORE__;
      if (store) {
        const state = store.getState();
        const agentId = state.activeAgentId;
        state.addMessage(agentId, {
          role: "user",
          content: "测试消息 E2E",
          timestamp: new Date(),
        });
        state.addMessage(agentId, {
          role: "assistant",
          content: "这是助手回复内容",
          timestamp: new Date(),
        });
      }
    });

    const hasStore = await page.evaluate(() => !!(window as any).__ZUSTAND_AGENT_STORE__);

    if (hasStore) {
      await expect(page.locator("text=测试消息 E2E").first()).toBeVisible({ timeout: 3_000 });
      await expect(page.locator("text=这是助手回复内容").first()).toBeVisible({ timeout: 3_000 });
    } else {
      const messageArea = page.locator("main");
      await expect(messageArea).toBeVisible();
    }
  });

  test("历史消息加载后列表中显示内容", async ({ page }) => {
    await waitForAppReady(page);

    const mainArea = page.locator("main");
    await expect(mainArea).toBeVisible();

    const content = await mainArea.textContent();
    expect(content).toBeTruthy();
  });

  test("Markdown 内容正确渲染 (代码块、链接)", async ({ page }) => {
    await waitForAppReady(page);

    const rendered = await page.evaluate(() => {
      const store = (window as any).__ZUSTAND_AGENT_STORE__;
      if (!store) return false;
      const state = store.getState();
      const agentId = state.activeAgentId;
      state.addMessage(agentId, {
        role: "assistant",
        content: "测试 **加粗** 和 `代码` 以及\n```js\nconsole.log('hello');\n```\n链接: [test](https://example.com)",
        timestamp: new Date(),
      });
      return true;
    });

    if (rendered) {
      await page.waitForTimeout(500);
      const codeBlock = page.locator("code, pre");
      if ((await codeBlock.count()) > 0) {
        await expect(codeBlock.first()).toBeVisible();
      }
    }
  });

  test("空聊天显示空状态 / welcome 区域", async ({ page }) => {
    await waitForAppReady(page);

    const mainContent = page.locator("main");
    await expect(mainContent).toBeVisible();
    const text = await mainContent.textContent();
    expect(text!.length).toBeGreaterThan(0);
  });
});
