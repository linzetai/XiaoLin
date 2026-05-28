/**
 * Suite 06: Cross-Session Memory — validates memory store/recall across sessions.
 */

import type { TestSuite, TestContext } from "../runner.js";
import { assertContains, assertNonEmpty, assertNoError } from "../helpers/assertions.js";
import { sleep, PROMPTS } from "../helpers/fixtures.js";

const suite: TestSuite = {
  name: "06-memory",

  async setup(ctx: TestContext) {
    await ctx.chat.newSession();
    await sleep(500);
  },

  cases: [
    {
      name: "6.1 Store a preference",
      async fn(ctx: TestContext) {
        const reply = await ctx.chat.sendAndWait(
          PROMPTS.REMEMBER("我最喜欢的编程语言是 Rust，我每天都用它开发"),
        );
        assertNonEmpty(reply);
        await assertNoError(ctx.chat);
      },
    },

    {
      name: "6.2 Recall in new session",
      async fn(ctx: TestContext) {
        // Create a brand new session
        await ctx.chat.newSession();
        await sleep(1000);

        const reply = await ctx.chat.sendAndWait(
          PROMPTS.RECALL("我最喜欢的编程语言是什么？你知道吗？"),
        );
        assertNonEmpty(reply);
        assertContains(reply, "Rust", "Should recall the stored preference from memory");
        await assertNoError(ctx.chat);
      },
    },

    {
      name: "6.3 Store a project fact",
      async fn(ctx: TestContext) {
        const reply = await ctx.chat.sendAndWait(
          PROMPTS.REMEMBER("我正在开发的项目叫 FastClaw，是一个 AI Agent 框架"),
        );
        assertNonEmpty(reply);
        await assertNoError(ctx.chat);
      },
    },

    {
      name: "6.4 Cross-session recall of project fact",
      async fn(ctx: TestContext) {
        await ctx.chat.newSession();
        await sleep(1000);

        const reply = await ctx.chat.sendAndWait(
          PROMPTS.RECALL("我正在做的项目叫什么名字？"),
        );
        assertNonEmpty(reply);
        assertContains(reply, "FastClaw", "Should recall the project name");
        await assertNoError(ctx.chat);
      },
    },

    {
      name: "6.5 Multiple facts accumulated",
      async fn(ctx: TestContext) {
        await ctx.chat.newSession();
        await sleep(1000);

        const reply = await ctx.chat.sendAndWait(
          "根据你对我的了解，我用什么语言开发什么项目？",
        );
        assertNonEmpty(reply);
        // Should recall both facts
        assertContains(reply, "Rust", "Should recall Rust preference");
        assertContains(reply, "FastClaw", "Should recall FastClaw project");
        await assertNoError(ctx.chat);
      },
    },
  ],
};

export default suite;
