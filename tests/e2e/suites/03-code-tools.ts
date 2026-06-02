/**
 * Suite 03: Code Intelligence Tools — validates lsp, file_outline, code_sections.
 * These tests require a Rust workspace to be configured as the agent's work_dir.
 */

import type { TestSuite, TestContext } from "../runner.js";
import { assertContains, assertNonEmpty, assertNoError } from "../helpers/assertions.js";
import { sleep } from "../helpers/fixtures.js";

const PROJECT_ROOT = "/home/linzetai/workspace/my_tools/XiaoLin";

const suite: TestSuite = {
  name: "03-code-tools",

  async setup(ctx: TestContext) {
    await ctx.chat.newSession();
    await sleep(500);
    // Set up context: tell the agent about the project
    await ctx.chat.sendAndWait(
      `我的项目在 ${PROJECT_ROOT}，这是一个 Rust 项目。请确认。`,
    );
  },

  cases: [
    {
      name: "3.1 Find definition (search_in_files)",
      async fn(ctx: TestContext) {
        const reply = await ctx.chat.sendAndWait(
          `在 ${PROJECT_ROOT}/crates/xiaolin-core/src/ 中搜索 "pub struct ToolRegistry"，告诉我在哪个文件的哪一行`,
        );
        assertNonEmpty(reply);
        assertContains(reply, "tool.rs", "Should find ToolRegistry in tool.rs");
        await assertNoError(ctx.chat);
      },
    },

    {
      name: "3.2 File outline via search",
      async fn(ctx: TestContext) {
        const reply = await ctx.chat.sendAndWait(
          `读取 ${PROJECT_ROOT}/crates/xiaolin-core/src/tool.rs 的前50行，列出定义的主要 pub 类型和 trait`,
        );
        assertNonEmpty(reply);
        // Should mention key types
        assertContains(reply, "ToolGroup", "Should mention ToolGroup");
        await assertNoError(ctx.chat);
      },
    },

    {
      name: "3.3 Find references",
      async fn(ctx: TestContext) {
        const reply = await ctx.chat.sendAndWait(
          `在 ${PROJECT_ROOT}/crates/ 中搜索所有使用 "eager_definitions" 的地方`,
        );
        assertNonEmpty(reply);
        // Should find multiple references
        assertContains(reply, "mod.rs", "Should find reference in runtime/mod.rs");
        await assertNoError(ctx.chat);
      },
    },
  ],
};

export default suite;
