/**
 * Suite 07: Multi-Turn Conversation / Context Compression —
 * validates long conversations maintain context across many turns.
 */

import type { TestSuite, TestContext } from "../runner.js";
import {
  assertContains,
  assertNonEmpty,
  assertNoError,
  assertGreaterThan,
} from "../helpers/assertions.js";
import { setupSuiteDir, sleep } from "../helpers/fixtures.js";

const SUITE_DIR = "/tmp/fastclaw-e2e/07-multi-turn";

const suite: TestSuite = {
  name: "07-multi-turn",

  async setup(ctx: TestContext) {
    setupSuiteDir("07-multi-turn");
    await ctx.chat.newSession();
    await sleep(500);
  },

  cases: [
    {
      name: "7.1 Context retained across 10 turns",
      async fn(ctx: TestContext) {
        // Turn 1: Establish a fact
        await ctx.chat.sendAndWait(
          "从现在起，我们约定一个暗号：当我说'魔法数字'时，答案是 7749。请确认。",
        );

        // Turns 2-9: Ask various unrelated questions to push context
        const fillers = [
          "1+1等于多少？",
          "Python 和 Rust 的区别是什么？简短回答。",
          "HTTP 状态码 404 表示什么？",
          "解释下什么是递归，一句话。",
          "TCP 和 UDP 的区别？简短回答。",
          "什么是 Docker？一句话回答。",
          "JSON 是什么的缩写？",
          "Git 的作者是谁？",
        ];

        for (const q of fillers) {
          await ctx.chat.sendAndWait(q);
        }

        // Turn 10: Ask for the secret
        const reply = await ctx.chat.sendAndWait("魔法数字是多少？");
        assertNonEmpty(reply);
        assertContains(reply, "7749", "Should remember the magic number after 10 turns");
        await assertNoError(ctx.chat);
      },
    },

    {
      name: "7.2 Accumulating file operations context",
      async fn(ctx: TestContext) {
        await ctx.chat.newSession();
        await sleep(500);

        // Create multiple files in sequence
        await ctx.chat.sendAndWait(
          `创建文件 ${SUITE_DIR}/step1.txt 内容为 "first step"`,
        );
        await ctx.chat.sendAndWait(
          `创建文件 ${SUITE_DIR}/step2.txt 内容为 "second step"`,
        );
        await ctx.chat.sendAndWait(
          `创建文件 ${SUITE_DIR}/step3.txt 内容为 "third step"`,
        );

        // Ask agent to recall all files created
        const reply = await ctx.chat.sendAndWait(
          "列出这次对话中我让你创建的所有文件路径",
        );
        assertNonEmpty(reply);
        assertContains(reply, "step1", "Should mention step1");
        assertContains(reply, "step2", "Should mention step2");
        assertContains(reply, "step3", "Should mention step3");
        await assertNoError(ctx.chat);
      },
    },

    {
      name: "7.3 Conversation with task continuity",
      async fn(ctx: TestContext) {
        await ctx.chat.newSession();
        await sleep(500);

        // Step 1: Define a task
        await ctx.chat.sendAndWait(
          "我需要你帮我完成一个小任务：在 /tmp/fastclaw-e2e/07-multi-turn/final.txt 中写入三行，分别是 apple, banana, cherry。先写第一行 apple。",
        );

        // Step 2: Continue
        await ctx.chat.sendAndWait("继续，把 banana 追加到文件里");

        // Step 3: Finish
        await ctx.chat.sendAndWait("最后追加 cherry 完成任务");

        // Verify: Ask agent to read back
        const reply = await ctx.chat.sendAndWait(
          `读取 ${SUITE_DIR}/final.txt 的内容`,
        );
        assertNonEmpty(reply);
        assertContains(reply, "apple", "Should contain apple");
        assertContains(reply, "banana", "Should contain banana");
        assertContains(reply, "cherry", "Should contain cherry");
        await assertNoError(ctx.chat);
      },
    },
  ],
};

export default suite;
