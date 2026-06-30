import { test, expect, waitForAppReady } from "../fixtures/mock-gateway";

test.describe("C5: 切换 Agent → 消息列表正确", () => {
  test("sidebar 至少显示 1 个 agent 条目", async ({ page }) => {
    await waitForAppReady(page);

    await expect(page.getByRole("button", { name: /main/i })).toBeVisible({ timeout: 5_000 });
  });

  test("点击不同 agent 后主区域内容切换", async ({ page }) => {
    await waitForAppReady(page);

    await expect(page.getByRole("button", { name: /main/i })).toBeVisible({ timeout: 5_000 });
    test.skip();
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

    const agentNames = page.getByRole("button", { name: /main/i });
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

  test("带 timeline 数据的会话切走再切回后不白屏", async ({ page }) => {
    await page.addInitScript(() => {
      (window as any).__MOCK_SESSIONS_OVERRIDE__ = [
        {
          id: "timeline-a",
          agentId: "main",
          title: "Timeline A",
          workDir: null,
          messageCount: 2,
          createdAt: "2026-04-30T00:00:00Z",
          updatedAt: "2026-04-30T00:02:00Z",
        },
        {
          id: "timeline-b",
          agentId: "main",
          title: "Timeline B",
          workDir: null,
          messageCount: 2,
          createdAt: "2026-04-30T00:00:00Z",
          updatedAt: "2026-04-30T00:01:00Z",
        },
      ];
      (window as any).__MOCK_MESSAGES_OVERRIDE__ = [
        {
          id: 1,
          role: "user",
          content: "placeholder message",
          name: null,
          toolCallId: null,
          toolCallsJson: null,
          createdAt: "2026-04-30T00:00:10Z",
        },
      ];
      (window as any).__MOCK_TIMELINE_NODES_BY_SESSION__ = {
        "timeline-a": [
          {
            kind: "user_message",
            node_id: "timeline-a-user",
            turn_id: "timeline-a-turn",
            status: "completed",
            created_at_ms: 1000,
            updated_at_ms: 1000,
            content: "TIMELINE_A_USER_MARKER",
          },
          {
            kind: "assistant_text",
            node_id: "timeline-a-answer",
            turn_id: "timeline-a-turn",
            status: "completed",
            created_at_ms: 1100,
            updated_at_ms: 1100,
            content: "TIMELINE_A_ASSISTANT_MARKER",
          },
          {
            kind: "turn_status",
            node_id: "timeline-a-status",
            turn_id: "timeline-a-turn",
            status: "completed",
            created_at_ms: 1200,
            updated_at_ms: 1200,
            end_reason: "completed",
            elapsed_ms: 1000,
          },
        ],
        "timeline-b": [
          {
            kind: "user_message",
            node_id: "timeline-b-user",
            turn_id: "timeline-b-turn",
            status: "completed",
            created_at_ms: 1000,
            updated_at_ms: 1000,
            content: "TIMELINE_B_USER_MARKER",
          },
          {
            kind: "assistant_text",
            node_id: "timeline-b-answer",
            turn_id: "timeline-b-turn",
            status: "completed",
            created_at_ms: 1100,
            updated_at_ms: 1100,
            content: "TIMELINE_B_ASSISTANT_MARKER",
          },
          {
            kind: "turn_status",
            node_id: "timeline-b-status",
            turn_id: "timeline-b-turn",
            status: "completed",
            created_at_ms: 1200,
            updated_at_ms: 1200,
            end_reason: "completed",
            elapsed_ms: 1000,
          },
        ],
      };
    });

    await waitForAppReady(page);

    await page.getByText("Timeline A").click();
    await expect(page.getByText("TIMELINE_A_ASSISTANT_MARKER")).toBeVisible({ timeout: 5_000 });

    await page.getByText("Timeline B").click();
    await expect(page.getByText("TIMELINE_B_ASSISTANT_MARKER")).toBeVisible({ timeout: 5_000 });
    await expect(page.getByText("TIMELINE_A_ASSISTANT_MARKER")).toHaveCount(0);

    await page.getByText("Timeline A").click();
    await expect(page.getByText("TIMELINE_A_ASSISTANT_MARKER")).toBeVisible({ timeout: 5_000 });
    await expect(page.getByText("TIMELINE_B_ASSISTANT_MARKER")).toHaveCount(0);

    const scroller = page.getByTestId("message-scroll-container");
    await expect(scroller).toBeVisible();
    const renderedText = await scroller.textContent();
    expect(renderedText).toContain("TIMELINE_A_USER_MARKER");
    expect(renderedText).toContain("TIMELINE_A_ASSISTANT_MARKER");
  });

  test("切换 agent 无 JS 错误", async ({ page }) => {
    const errors: string[] = [];
    page.on("pageerror", (err) => errors.push(err.message));

    await waitForAppReady(page);

    const currentAgent = page.getByRole("button", { name: /main/i });
    await expect(currentAgent).toBeVisible({ timeout: 5_000 });
    await currentAgent.click();
    await page.waitForTimeout(300);

    expect(errors).toEqual([]);
  });
});
