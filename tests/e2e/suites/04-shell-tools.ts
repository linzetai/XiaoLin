/**
 * Suite 04: Shell / PTY Tools — validates shell_exec, exec_command, write_stdin.
 */

import type { TestSuite, TestContext } from "../runner.js";
import {
  assertContains,
  assertNonEmpty,
  assertNoError,
  assertTrue,
} from "../helpers/assertions.js";
import { setupSuiteDir, fileExists, sleep, PROMPTS } from "../helpers/fixtures.js";

const SUITE_DIR = "/tmp/xiaolin-e2e/04-shell-tools";

const suite: TestSuite = {
  name: "04-shell-tools",

  async setup(ctx: TestContext) {
    setupSuiteDir("04-shell-tools");
    await ctx.chat.newSession();
    await sleep(500);
  },

  cases: [
    {
      name: "4.1 Simple command execution",
      async fn(ctx: TestContext) {
        const reply = await ctx.chat.sendAndWait(
          PROMPTS.SHELL_EXEC("echo 'hello from shell'"),
        );
        assertNonEmpty(reply);
        assertContains(reply, "hello from shell", "Should contain echo output");
        await assertNoError(ctx.chat);
      },
    },

    {
      name: "4.2 Multi-command sequence",
      async fn(ctx: TestContext) {
        const reply = await ctx.chat.sendAndWait(
          `在 ${SUITE_DIR} 中创建子目录 subdir，然后在 subdir 里创建文件 test.txt 内容为 "created"，用 shell 命令实现`,
        );
        assertNonEmpty(reply);

        // Verify the results
        assertTrue(
          fileExists(`${SUITE_DIR}/subdir/test.txt`),
          "subdir/test.txt should exist",
        );
        await assertNoError(ctx.chat);
      },
    },

    {
      name: "4.3 Command with working directory",
      async fn(ctx: TestContext) {
        const reply = await ctx.chat.sendAndWait(
          `在 /home/linzetai/workspace/my_tools/XiaoLin 目录下执行 cargo --version`,
        );
        assertNonEmpty(reply);
        assertContains(reply, "cargo", "Should contain cargo version info");
        await assertNoError(ctx.chat);
      },
    },

    {
      name: "4.4 Command output capture",
      async fn(ctx: TestContext) {
        const reply = await ctx.chat.sendAndWait(
          PROMPTS.SHELL_EXEC(`ls -la ${SUITE_DIR}`),
        );
        assertNonEmpty(reply);
        assertContains(reply, "subdir", "Should list the subdir we created");
        await assertNoError(ctx.chat);
      },
    },
  ],
};

export default suite;
