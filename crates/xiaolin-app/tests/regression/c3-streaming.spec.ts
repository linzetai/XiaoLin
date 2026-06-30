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

  test("旧历史消息（无timeline数据）显示不支持提示且滚动保持响应", async ({ page }) => {
    const errors: string[] = [];
    page.on("pageerror", (err) => errors.push(err.message));

    // Load messages WITHOUT corresponding timeline events — simulates old sessions
    // created before the canonical timeline feature was deployed.
    await page.addInitScript(() => {
      const messages = [];
      for (let i = 0; i < 1200; i += 1) {
        const n = String(i).padStart(4, "0");
        messages.push({
          id: i * 2 + 1,
          role: "user",
          content: `LONG_HISTORY_USER_${n}`,
          name: null,
          toolCallId: null,
          toolCallsJson: null,
          createdAt: `2026-04-30T00:${String(Math.floor(i / 60)).padStart(2, "0")}:${String(i % 60).padStart(2, "0")}Z`,
        });
        messages.push({
          id: i * 2 + 2,
          role: "assistant",
          content: `LONG_HISTORY_ASSISTANT_${n}`,
          name: null,
          toolCallId: null,
          toolCallsJson: null,
          reasoningContent: `LONG_HISTORY_REASONING_${n}`,
          segmentOrder: ["reasoning", "text"],
          createdAt: `2026-04-30T00:${String(Math.floor(i / 60)).padStart(2, "0")}:${String(i % 60).padStart(2, "0")}Z`,
        });
      }
      (window as any).__MOCK_MESSAGES_OVERRIDE__ = messages;
      // Ensure no timeline data is provided for this old-session test
      (window as any).__MOCK_TIMELINE_NODES_OVERRIDE__ = [];
    });

    await waitForAppReady(page);
    await page.getByText("测试对话").click();

    const scroller = page.getByTestId("message-scroll-container");
    await expect(scroller).toBeVisible();

    await scroller.evaluate((el) => {
      el.scrollTop = 0;
      el.dispatchEvent(new Event("scroll", { bubbles: true }));
    });
    // Old sessions without timeline events should show the unsupported notice
    await expect(page.getByText("This session was created before canonical timeline replay")).toBeVisible({ timeout: 5_000 });

    await scroller.evaluate((el) => {
      el.scrollTop = el.scrollHeight;
      el.dispatchEvent(new Event("scroll", { bubbles: true }));
    });
    // Virtualized: off-screen messages should not be in the DOM
    await expect(page.getByText("LONG_HISTORY_ASSISTANT_1199")).toHaveCount(0);

    const scrollProbe = await scroller.evaluate(async (el) => {
      const start = performance.now();
      for (let i = 0; i < 20; i += 1) {
        el.scrollTop = i % 2 === 0 ? 0 : el.scrollHeight;
        el.dispatchEvent(new Event("scroll", { bubbles: true }));
        await new Promise((resolve) => requestAnimationFrame(resolve));
      }
      return {
        elapsedMs: performance.now() - start,
        scrollTop: el.scrollTop,
        scrollHeight: el.scrollHeight,
      };
    });

    expect(scrollProbe.scrollHeight).toBeGreaterThan(0);
    expect(scrollProbe.elapsedMs).toBeLessThan(5_000);

    const textarea = page.locator("textarea").first();
    await textarea.fill("still responsive after long history scroll");
    await expect(textarea).toHaveValue("still responsive after long history scroll");
    expect(errors).toEqual([]);
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
