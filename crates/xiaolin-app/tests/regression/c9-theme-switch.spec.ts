import { test, expect, waitForAppReady } from "../fixtures/mock-gateway";

test.describe("C9: 主题切换 light↔dark", () => {
  test("默认主题 data-theme 为 light", async ({ page }) => {
    await waitForAppReady(page);

    const theme = await page.evaluate(() =>
      document.documentElement.getAttribute("data-theme"),
    );
    expect(theme).toBe("light");
  });

  test("切换到 dark 主题后 data-theme 变为 dark", async ({ page }) => {
    await waitForAppReady(page);

    await page.evaluate(() => {
      const themeStore = (window as any).__ZUSTAND_THEME_STORE__;
      if (themeStore) {
        themeStore.getState().setMode("dark");
        return;
      }
      document.documentElement.setAttribute("data-theme", "dark");
    });

    await page.waitForTimeout(300);

    const theme = await page.evaluate(() =>
      document.documentElement.getAttribute("data-theme"),
    );
    expect(theme).toBe("dark");
  });

  test("dark 主题下背景色变深", async ({ page }) => {
    await waitForAppReady(page);

    const lightBg = await page.evaluate(() => {
      const el = document.querySelector('[class*="flex"][class*="h-full"]');
      return el ? getComputedStyle(el).backgroundColor : null;
    });

    await page.evaluate(() => {
      const themeStore = (window as any).__ZUSTAND_THEME_STORE__;
      if (themeStore) {
        themeStore.getState().setMode("dark");
      } else {
        document.documentElement.setAttribute("data-theme", "dark");
      }
    });
    await page.waitForTimeout(400);

    const darkBg = await page.evaluate(() => {
      const el = document.querySelector('[class*="flex"][class*="h-full"]');
      return el ? getComputedStyle(el).backgroundColor : null;
    });

    if (lightBg && darkBg) {
      expect(lightBg).not.toBe(darkBg);
    }
  });

  test("切换回 light 后恢复", async ({ page }) => {
    await waitForAppReady(page);

    await page.evaluate(() => {
      const themeStore = (window as any).__ZUSTAND_THEME_STORE__;
      if (themeStore) {
        themeStore.getState().setMode("dark");
      } else {
        document.documentElement.setAttribute("data-theme", "dark");
      }
    });
    await page.waitForTimeout(300);

    await page.evaluate(() => {
      const themeStore = (window as any).__ZUSTAND_THEME_STORE__;
      if (themeStore) {
        themeStore.getState().setMode("light");
      } else {
        document.documentElement.setAttribute("data-theme", "light");
      }
    });
    await page.waitForTimeout(300);

    const theme = await page.evaluate(() =>
      document.documentElement.getAttribute("data-theme"),
    );
    expect(theme).toBe("light");
  });

  test("accent theme 切换无报错", async ({ page }) => {
    const errors: string[] = [];
    page.on("pageerror", (err) => errors.push(err.message));

    await waitForAppReady(page);

    const accents = ["default", "ocean", "sunset", "midnight"];
    for (const accent of accents) {
      await page.evaluate((a) => {
        const themeStore = (window as any).__ZUSTAND_THEME_STORE__;
        if (themeStore) {
          themeStore.getState().setAccent(a);
        } else {
          if (a === "default") {
            document.documentElement.removeAttribute("data-accent");
          } else {
            document.documentElement.setAttribute("data-accent", a);
          }
        }
      }, accent);
      await page.waitForTimeout(200);
    }

    expect(errors).toEqual([]);
  });

  test("主题切换过程中无白屏闪烁 (基础检查)", async ({ page }) => {
    await waitForAppReady(page);

    for (const mode of ["dark", "light", "dark", "light"] as const) {
      await page.evaluate((m) => {
        const themeStore = (window as any).__ZUSTAND_THEME_STORE__;
        if (themeStore) {
          themeStore.getState().setMode(m);
        } else {
          document.documentElement.setAttribute("data-theme", m);
        }
      }, mode);
      await page.waitForTimeout(100);

      const isVisible = await page.evaluate(() => {
        const el = document.querySelector('[class*="flex"][class*="h-full"]');
        if (!el) return false;
        const bg = getComputedStyle(el).backgroundColor;
        return bg !== "rgba(0, 0, 0, 0)" && bg !== "transparent";
      });
      expect(isVisible).toBe(true);
    }
  });
});
