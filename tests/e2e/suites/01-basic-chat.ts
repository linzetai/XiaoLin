/**
 * Suite 01: Basic Chat — validates message send/receive, streaming, multi-turn context.
 */

import type { TestSuite, TestContext } from "../runner.js";
import { assertContains, assertNonEmpty, assertNoError, assertTrue, assertFalse } from "../helpers/assertions.js";
import { sleep } from "../helpers/fixtures.js";

const suite: TestSuite = {
  name: "01-basic-chat",

  async setup(ctx: TestContext) {
    // Ensure we're on a fresh chat
    await ctx.chat.newSession();
    await sleep(500);
  },

  cases: [
    {
      name: "1.1 Simple question-answer",
      async fn(ctx: TestContext) {
        const reply = await ctx.chat.sendAndWait("你好，请简短回答：1+1等于几？");
        assertNonEmpty(reply, "Reply should not be empty");
        assertContains(reply, "2", "Reply should contain the answer '2'");
        await assertNoError(ctx.chat);
      },
    },

    {
      name: "1.2 Multi-turn context retention",
      async fn(ctx: TestContext) {
        await ctx.chat.newSession();
        await sleep(300);

        // Turn 1: define a variable
        await ctx.chat.sendAndWait("从现在起，我定义一个变量 X = 42。请确认你记住了。");

        // Turn 2: reference the variable
        await ctx.chat.sendAndWait("X 的值加上 8 等于多少？");

        // Turn 3: ask for the original value
        const reply = await ctx.chat.sendAndWait("最初我设定 X 的值是多少？");
        assertContains(reply, "42", "Agent should remember X = 42 from earlier turns");
      },
    },

    {
      name: "1.3 Long text input",
      async fn(ctx: TestContext) {
        await ctx.chat.newSession();
        await sleep(300);

        // Generate a long prompt (~2000 chars)
        const longText = "请总结以下内容：\n" + "这是一段测试文本，用于验证长输入的处理能力。".repeat(80);
        const reply = await ctx.chat.sendAndWait(longText, 90_000);
        assertNonEmpty(reply, "Reply to long input should not be empty");
        await assertNoError(ctx.chat);
      },
    },

    {
      name: "1.4 Message queue during streaming",
      async fn(ctx: TestContext) {
        await ctx.chat.newSession();
        await sleep(300);

        // Send a message that should trigger a non-trivial reply
        await ctx.chat.sendMessage("写一首关于编程的四行诗");

        // Wait briefly then check if streaming
        await sleep(1500);
        const streaming = await ctx.chat.isStreaming();

        if (streaming) {
          // Send another message while streaming — should queue
          await ctx.chat.sendMessage("再写一首关于月亮的");
          // Wait for everything to finish
          await ctx.chat.waitForReply(60_000);
          await sleep(2000);
          // The second reply should eventually appear
          await ctx.chat.waitForReply(60_000);
        }

        // Just verify no crash/error occurred
        await assertNoError(ctx.chat);
      },
    },

    {
      name: "1.5 Cancel generation",
      async fn(ctx: TestContext) {
        await ctx.chat.newSession();
        await sleep(300);

        // Send a message that will produce a long response
        await ctx.chat.sendMessage("请详细解释量子计算的原理，至少写2000字");
        await sleep(2000);

        // Cancel
        await ctx.chat.cancelGeneration();
        await sleep(1000);

        // Verify no crash — the streaming should have stopped
        const stillStreaming = await ctx.chat.isStreaming();
        assertFalse(stillStreaming, "Streaming should have stopped after cancel");
      },
    },
  ],
};

export default suite;
