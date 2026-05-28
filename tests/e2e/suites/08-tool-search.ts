/**
 * Suite 08: Deferred Tool Discovery — validates that the agent can discover
 * and activate deferred tools through tool_search when needed.
 */

import type { TestSuite, TestContext } from "../runner.js";
import { assertContains, assertNonEmpty, assertNoError } from "../helpers/assertions.js";
import { sleep } from "../helpers/fixtures.js";

const suite: TestSuite = {
  name: "08-tool-search",

  async setup(ctx: TestContext) {
    await ctx.chat.newSession();
    await sleep(500);
  },

  cases: [
    {
      name: "8.1 Discover time tool",
      async fn(ctx: TestContext) {
        // get_current_time is deferred — agent should use tool_search to find it
        const reply = await ctx.chat.sendAndWait("现在几点了？告诉我当前的时间");
        assertNonEmpty(reply);
        // Should contain some time-like information
        const hasTime = reply.match(/\d{1,2}[:\uff1a]\d{2}/) !== null
          || reply.includes("时") || reply.includes("点");
        if (!hasTime) {
          throw new Error(
            `Expected reply to contain time information, got: "${reply.substring(0, 200)}"`,
          );
        }
        await assertNoError(ctx.chat);
      },
    },

    {
      name: "8.2 Discover plan mode tools",
      async fn(ctx: TestContext) {
        await ctx.chat.newSession();
        await sleep(300);

        // enter_plan_mode is deferred — agent needs tool_search
        const reply = await ctx.chat.sendAndWait(
          "我想进入计划模式来规划一下任务，请切换到 plan mode",
        );
        assertNonEmpty(reply);
        // Verify agent acknowledged plan mode
        const mentioned = reply.includes("plan") || reply.includes("计划") || reply.includes("规划");
        if (!mentioned) {
          throw new Error(`Expected plan mode acknowledgment, got: "${reply.substring(0, 200)}"`);
        }
        await assertNoError(ctx.chat);
      },
    },

    {
      name: "8.3 Discover multi_edit tool",
      async fn(ctx: TestContext) {
        await ctx.chat.newSession();
        await sleep(300);

        // multi_edit is deferred
        const reply = await ctx.chat.sendAndWait(
          "我需要用 multi_edit 工具来同时修改多个地方，这个工具可用吗？请通过 tool_search 找一下。",
        );
        assertNonEmpty(reply);
        assertContains(reply, "multi_edit", "Should mention multi_edit tool");
        await assertNoError(ctx.chat);
      },
    },
  ],
};

export default suite;
