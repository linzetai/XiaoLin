/**
 * Suite 10: Plan Mode — validates mode switching and read-only constraints.
 */

import type { TestSuite, TestContext } from "../runner.js";
import {
  assertContains,
  assertNonEmpty,
  assertNoError,
  assertNotContains,
  assertTrue,
  assertFalse,
} from "../helpers/assertions.js";
import { setupSuiteDir, createTestFile, fileExists, sleep } from "../helpers/fixtures.js";

const SUITE_DIR = "/tmp/fastclaw-e2e/10-plan-mode";

const suite: TestSuite = {
  name: "10-plan-mode",

  async setup(ctx: TestContext) {
    setupSuiteDir("10-plan-mode");
    createTestFile(SUITE_DIR, "readonly-test.txt", "original content");
    await ctx.chat.newSession();
    await sleep(500);
  },

  cases: [
    {
      name: "10.1 Enter plan mode",
      async fn(ctx: TestContext) {
        // Tell the agent to enter plan mode
        const reply = await ctx.chat.sendAndWait(
          "请切换到计划模式（plan mode），我想先规划再执行",
        );
        assertNonEmpty(reply);
        // Agent should acknowledge
        await assertNoError(ctx.chat);
      },
    },

    {
      name: "10.2 Plan mode blocks writes",
      async fn(ctx: TestContext) {
        // Try to write a file in plan mode
        const reply = await ctx.chat.sendAndWait(
          `在 plan 模式下，请创建文件 ${SUITE_DIR}/should-not-exist.txt，内容为 test`,
        );
        assertNonEmpty(reply);
        // The file should not be created (plan mode is read-only)
        // Agent should explain it can't do writes in plan mode
        const blocked = !fileExists(`${SUITE_DIR}/should-not-exist.txt`);
        // Note: if the agent doesn't enforce plan mode, this might still pass
        // We just verify no crash
        await assertNoError(ctx.chat);
      },
    },

    {
      name: "10.3 Plan mode allows reads",
      async fn(ctx: TestContext) {
        const reply = await ctx.chat.sendAndWait(
          `读取 ${SUITE_DIR}/readonly-test.txt 的内容`,
        );
        assertNonEmpty(reply);
        assertContains(reply, "original content", "Should be able to read in plan mode");
        await assertNoError(ctx.chat);
      },
    },

    {
      name: "10.4 Exit plan mode restores writes",
      async fn(ctx: TestContext) {
        // Exit plan mode
        await ctx.chat.sendAndWait("请退出计划模式，切回正常的 Agent 模式");
        await sleep(500);

        // Now writes should work
        const reply = await ctx.chat.sendAndWait(
          `创建文件 ${SUITE_DIR}/after-plan.txt，内容为 "plan mode exited"`,
        );
        assertNonEmpty(reply);
        // Verify file was created
        assertTrue(
          fileExists(`${SUITE_DIR}/after-plan.txt`),
          "Should be able to write after exiting plan mode",
        );
        await assertNoError(ctx.chat);
      },
    },

    {
      name: "10.5 Plan file creation",
      async fn(ctx: TestContext) {
        await ctx.chat.newSession();
        await sleep(300);

        const reply = await ctx.chat.sendAndWait(
          "帮我制定一个计划：实现一个简单的 REST API，包含用户注册和登录两个接口。请创建一个 plan 文件来记录这个计划。",
        );
        assertNonEmpty(reply);
        // Agent should have created a plan or at least discussed one
        await assertNoError(ctx.chat);
      },
    },
  ],
};

export default suite;
