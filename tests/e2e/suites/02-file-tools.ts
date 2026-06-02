/**
 * Suite 02: File System Tools — validates read_file, write_file, edit_file,
 * list_directory, glob, search_in_files.
 */

import type { TestSuite, TestContext } from "../runner.js";
import {
  assertContains,
  assertNonEmpty,
  assertNoError,
  assertToolCalled,
} from "../helpers/assertions.js";
import {
  setupSuiteDir,
  createTestFile,
  readTestFile,
  fileExists,
  sleep,
  PROMPTS,
} from "../helpers/fixtures.js";

const SUITE_DIR = "/tmp/xiaolin-e2e/02-file-tools";

const suite: TestSuite = {
  name: "02-file-tools",

  async setup(ctx: TestContext) {
    setupSuiteDir("02-file-tools");
    createTestFile(SUITE_DIR, "hello.txt", "test content for reading");
    createTestFile(SUITE_DIR, "data.json", '{"key": "value"}');
    createTestFile(SUITE_DIR, "notes.md", "# Notes\n\nSome markdown content here.");
    await ctx.chat.newSession();
    await sleep(500);
  },

  cases: [
    {
      name: "2.1 Read file",
      async fn(ctx: TestContext) {
        const reply = await ctx.chat.sendAndWait(
          PROMPTS.READ_FILE(`${SUITE_DIR}/hello.txt`),
        );
        assertNonEmpty(reply);
        assertContains(reply, "test content", "Reply should contain the file content");
        await assertNoError(ctx.chat);
      },
    },

    {
      name: "2.2 Write file",
      async fn(ctx: TestContext) {
        const targetPath = `${SUITE_DIR}/greeting.txt`;
        const reply = await ctx.chat.sendAndWait(
          PROMPTS.WRITE_FILE(targetPath, "Hello World from XiaoLin"),
        );
        assertNonEmpty(reply);
        // Verify the file was actually created
        const exists = fileExists(targetPath);
        if (!exists) {
          throw new Error(`File was not created at ${targetPath}`);
        }
        const content = readTestFile(targetPath);
        assertContains(content, "Hello World", "File content should match");
        await assertNoError(ctx.chat);
      },
    },

    {
      name: "2.3 Edit file",
      async fn(ctx: TestContext) {
        // First ensure greeting.txt exists with known content
        createTestFile(SUITE_DIR, "editable.txt", "The quick brown fox jumps over the lazy dog");

        const reply = await ctx.chat.sendAndWait(
          PROMPTS.EDIT_FILE(`${SUITE_DIR}/editable.txt`, "quick brown fox", "fast red cat"),
        );
        assertNonEmpty(reply);

        const content = readTestFile(`${SUITE_DIR}/editable.txt`);
        assertContains(content, "fast red cat", "Edit should have been applied");
        await assertNoError(ctx.chat);
      },
    },

    {
      name: "2.4 List directory",
      async fn(ctx: TestContext) {
        const reply = await ctx.chat.sendAndWait(PROMPTS.LIST_DIR(SUITE_DIR));
        assertNonEmpty(reply);
        assertContains(reply, "hello.txt", "Should list hello.txt");
        assertContains(reply, "data.json", "Should list data.json");
        await assertNoError(ctx.chat);
      },
    },

    {
      name: "2.5 Glob search",
      async fn(ctx: TestContext) {
        const reply = await ctx.chat.sendAndWait(
          PROMPTS.GLOB(SUITE_DIR, "*.txt"),
        );
        assertNonEmpty(reply);
        assertContains(reply, "hello.txt", "Glob should find hello.txt");
        await assertNoError(ctx.chat);
      },
    },

    {
      name: "2.6 Search in files",
      async fn(ctx: TestContext) {
        const reply = await ctx.chat.sendAndWait(
          PROMPTS.SEARCH_CONTENT(SUITE_DIR, "markdown"),
        );
        assertNonEmpty(reply);
        assertContains(reply, "notes.md", "Search should find notes.md containing 'markdown'");
        await assertNoError(ctx.chat);
      },
    },
  ],
};

export default suite;
