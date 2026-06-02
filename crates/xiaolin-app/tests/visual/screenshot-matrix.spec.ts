import { test, expect, waitForAppReady } from "../fixtures/mock-gateway";

/**
 * Screenshot matrix: 5 page states × 5 theme combos = 25 screenshots.
 *
 * Page states:
 *   1. AgentList (sidebar focus)
 *   2. MessageStream empty (no messages)
 *   3. MessageStream with messages
 *   4. Settings panel
 *   5. Theme switch (dark mode)
 *
 * Theme combos:
 *   1. light + default
 *   2. dark + default
 *   3. light + ocean
 *   4. dark + ocean
 *   5. dark + midnight
 */

const THEME_COMBOS = [
  { mode: "light" as const, accent: "default" as const, label: "light-default" },
  { mode: "dark" as const, accent: "default" as const, label: "dark-default" },
  { mode: "light" as const, accent: "ocean" as const, label: "light-ocean" },
  { mode: "dark" as const, accent: "ocean" as const, label: "dark-ocean" },
  { mode: "dark" as const, accent: "midnight" as const, label: "dark-midnight" },
];

type ThemeMode = "light" | "dark";
type AccentTheme = "default" | "ocean" | "sunset" | "midnight";

async function applyTheme(page: import("@playwright/test").Page, mode: ThemeMode, accent: AccentTheme) {
  await page.evaluate(
    ({ m, a }) => {
      document.documentElement.setAttribute("data-theme", m);
      if (a === "default") {
        document.documentElement.removeAttribute("data-accent");
      } else {
        document.documentElement.setAttribute("data-accent", a);
      }

      const ts = (window as any).__ZUSTAND_THEME_STORE__;
      if (ts) {
        ts.getState().setMode(m);
        ts.getState().setAccent(a);
      }
    },
    { m: mode, a: accent },
  );
  await page.waitForTimeout(400);
}

test.describe("截图矩阵 (5 × 5)", () => {
  for (const theme of THEME_COMBOS) {
    test.describe(`Theme: ${theme.label}`, () => {

      test(`AgentList sidebar - ${theme.label}`, async ({ page }) => {
        await waitForAppReady(page);
        await applyTheme(page, theme.mode, theme.accent);
        await expect(page).toHaveScreenshot(`agent-list-${theme.label}.png`, {
          fullPage: false,
          animations: "disabled",
        });
      });

      test(`MessageStream empty - ${theme.label}`, async ({ page }) => {
        await waitForAppReady(page);
        await applyTheme(page, theme.mode, theme.accent);
        const main = page.locator("main");
        await expect(main).toHaveScreenshot(`msg-empty-${theme.label}.png`, {
          animations: "disabled",
        });
      });

      test(`MessageStream with messages - ${theme.label}`, async ({ page }) => {
        await waitForAppReady(page);
        await applyTheme(page, theme.mode, theme.accent);

        await page.evaluate(() => {
          const store = (window as any).__ZUSTAND_AGENT_STORE__;
          if (!store) return;
          const state = store.getState();
          const agentId = state.activeAgentId;
          state.addMessage(agentId, {
            role: "user",
            content: "测试截图消息",
            timestamp: new Date(),
          });
          state.addMessage(agentId, {
            role: "assistant",
            content: "这是助手的回复，包含 **Markdown 格式** 和 `代码`。",
            timestamp: new Date(),
          });
        });
        await page.waitForTimeout(500);

        const main = page.locator("main");
        await expect(main).toHaveScreenshot(`msg-content-${theme.label}.png`, {
          animations: "disabled",
        });
      });

      test(`Settings panel - ${theme.label}`, async ({ page }) => {
        await waitForAppReady(page);
        await applyTheme(page, theme.mode, theme.accent);

        const settingsBtn = page.locator('button:has(svg[class*="lucide-settings"]), button[aria-label*="settings"], button[aria-label*="设置"]');
        if ((await settingsBtn.count()) > 0) {
          await settingsBtn.first().click();
          await page.waitForTimeout(500);
        }

        await expect(page).toHaveScreenshot(`settings-${theme.label}.png`, {
          fullPage: false,
          animations: "disabled",
        });
      });

      test(`Full app dark mode - ${theme.label}`, async ({ page }) => {
        await waitForAppReady(page);
        await applyTheme(page, theme.mode, theme.accent);
        await expect(page).toHaveScreenshot(`full-app-${theme.label}.png`, {
          fullPage: false,
          animations: "disabled",
        });
      });

    });
  }
});
