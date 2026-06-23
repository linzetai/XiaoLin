import i18n from "i18next";
import { initReactI18next } from "react-i18next";

import zhCommon from "./locales/zh/common.json";
import zhChat from "./locales/zh/chat.json";
import zhSettings from "./locales/zh/settings.json";
import zhSidebar from "./locales/zh/sidebar.json";
import zhHeader from "./locales/zh/header.json";
import zhOnboarding from "./locales/zh/onboarding.json";
import zhNotification from "./locales/zh/notification.json";
import zhPlugins from "./locales/zh/plugins.json";
import zhCost from "./locales/zh/cost.json";
import zhBrowser from "./locales/zh/browser.json";
import zhAutomation from "./locales/zh/automation.json";
import zhFileViewer from "./locales/zh/fileViewer.json";

import enCommon from "./locales/en/common.json";
import enChat from "./locales/en/chat.json";
import enSettings from "./locales/en/settings.json";
import enSidebar from "./locales/en/sidebar.json";
import enHeader from "./locales/en/header.json";
import enOnboarding from "./locales/en/onboarding.json";
import enNotification from "./locales/en/notification.json";
import enPlugins from "./locales/en/plugins.json";
import enCost from "./locales/en/cost.json";
import enBrowser from "./locales/en/browser.json";
import enAutomation from "./locales/en/automation.json";
import enFileViewer from "./locales/en/fileViewer.json";

const LOCALE_STORAGE_KEY = "xiaolin-locale";

function getSavedLocale(): string {
  try {
    const raw = localStorage.getItem(LOCALE_STORAGE_KEY);
    if (raw) {
      const parsed = JSON.parse(raw);
      if (parsed?.state?.locale) return parsed.state.locale;
    }
  } catch { /* ignore */ }
  return "zh";
}

i18n.use(initReactI18next).init({
  resources: {
    zh: {
      common: zhCommon,
      chat: zhChat,
      settings: zhSettings,
      sidebar: zhSidebar,
      header: zhHeader,
      onboarding: zhOnboarding,
      notification: zhNotification,
      plugins: zhPlugins,
      cost: zhCost,
      browser: zhBrowser,
      automation: zhAutomation,
      fileViewer: zhFileViewer,
    },
    en: {
      common: enCommon,
      chat: enChat,
      settings: enSettings,
      sidebar: enSidebar,
      header: enHeader,
      onboarding: enOnboarding,
      notification: enNotification,
      plugins: enPlugins,
      cost: enCost,
      browser: enBrowser,
      automation: enAutomation,
      fileViewer: enFileViewer,
    },
  },
  lng: getSavedLocale(),
  fallbackLng: "zh",
  ns: ["common", "chat", "settings", "sidebar", "header", "onboarding", "notification", "plugins", "cost", "browser", "automation", "fileViewer"],
  defaultNS: "common",
  interpolation: {
    escapeValue: false,
  },
});

export default i18n;
