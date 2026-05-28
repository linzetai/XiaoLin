/**
 * Suite 05: Web/Network Tools — validates web_fetch, http_fetch.
 */

import type { TestSuite, TestContext } from "../runner.js";
import { assertContains, assertNonEmpty, assertNoError } from "../helpers/assertions.js";
import { sleep, PROMPTS } from "../helpers/fixtures.js";

const suite: TestSuite = {
  name: "05-web-tools",

  async setup(ctx: TestContext) {
    await ctx.chat.newSession();
    await sleep(500);
  },

  cases: [
    {
      name: "5.1 HTTP GET fetch",
      async fn(ctx: TestContext) {
        const reply = await ctx.chat.sendAndWait(
          PROMPTS.WEB_FETCH("https://httpbin.org/get"),
          30_000,
        );
        assertNonEmpty(reply);
        // httpbin.org/get returns JSON with "origin", "url", "headers" fields
        assertContains(reply, "httpbin.org", "Should contain response from httpbin");
        await assertNoError(ctx.chat);
      },
    },

    {
      name: "5.2 HTTP POST request",
      async fn(ctx: TestContext) {
        const reply = await ctx.chat.sendAndWait(
          `用 http_fetch 工具发送 POST 请求到 https://httpbin.org/post，请求体为 JSON {"name":"fastclaw","version":"0.0.6"}，并告诉我响应中的 json 字段内容`,
          30_000,
        );
        assertNonEmpty(reply);
        assertContains(reply, "fastclaw", "Should echo back our payload");
        await assertNoError(ctx.chat);
      },
    },
  ],
};

export default suite;
