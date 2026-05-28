/**
 * Suite 11: Goal / Todo Tools — validates task management capabilities.
 */

import type { TestSuite, TestContext } from "../runner.js";
import { assertContains, assertNonEmpty, assertNoError } from "../helpers/assertions.js";
import { sleep } from "../helpers/fixtures.js";

const suite: TestSuite = {
  name: "11-goal-todo",

  async setup(ctx: TestContext) {
    await ctx.chat.newSession();
    await sleep(500);
  },

  cases: [
    {
      name: "11.1 Create todo list",
      async fn(ctx: TestContext) {
        const reply = await ctx.chat.sendAndWait(
          "帮我规划实现一个用户登录功能的步骤，创建一个待办列表包含：1.设计数据库表 2.实现注册接口 3.实现登录接口 4.添加JWT认证",
        );
        assertNonEmpty(reply);
        // Agent should have created todos
        assertContains(reply, "数据库", "Should mention database step");
        await assertNoError(ctx.chat);
      },
    },

    {
      name: "11.2 Read todo list",
      async fn(ctx: TestContext) {
        const reply = await ctx.chat.sendAndWait(
          "我当前的待办列表是什么？列出所有任务。",
        );
        assertNonEmpty(reply);
        // Should contain the previously created items
        assertContains(reply, "登录", "Should contain login task");
        await assertNoError(ctx.chat);
      },
    },

    {
      name: "11.3 Create goal",
      async fn(ctx: TestContext) {
        const reply = await ctx.chat.sendAndWait(
          "设定一个目标：本周完成用户认证模块的全部开发和测试",
        );
        assertNonEmpty(reply);
        assertContains(reply, "认证", "Should acknowledge the goal about authentication");
        await assertNoError(ctx.chat);
      },
    },

    {
      name: "11.4 Query goal",
      async fn(ctx: TestContext) {
        const reply = await ctx.chat.sendAndWait(
          "我当前设定的目标是什么？",
        );
        assertNonEmpty(reply);
        assertContains(reply, "认证", "Should recall the authentication goal");
        await assertNoError(ctx.chat);
      },
    },

    {
      name: "11.5 Goal persists across turns",
      async fn(ctx: TestContext) {
        // Do a few unrelated turns
        await ctx.chat.sendAndWait("1+1=?");
        await ctx.chat.sendAndWait("什么是 Rust？一句话回答");

        // Then ask about the goal again
        const reply = await ctx.chat.sendAndWait(
          "回顾一下，我之前设定的目标是什么？还有我的待办列表呢？",
        );
        assertNonEmpty(reply);
        assertContains(reply, "认证", "Should still remember the goal");
        await assertNoError(ctx.chat);
      },
    },
  ],
};

export default suite;
