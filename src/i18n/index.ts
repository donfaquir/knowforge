import i18n from "i18next";
import { initReactI18next } from "react-i18next";
import en from "../locales/en.json";
import zh from "../locales/zh.json";

const LOCALE_KEY = "knowforge:locale";

export type AppLocale = "en" | "zh";

/**
 * 是否将语言码视为中文（应用内仅 zh 一种中文资源）。
 * 浏览器 navigator.language 与 i18next 可能返回 zh-CN、zh-TW、zh-Hans、zh-Hant 等；
 * 按 BCP 47 主语言子标签是否为 zh 判定，与上述变体一致。
 */
function isChineseLocale(lng: string): boolean {
  const primary = lng.split(/[-_]/)[0]?.toLowerCase() ?? "";
  return primary === "zh";
}

/** 将任意语言码规范为应用支持的 en | zh */
function normalizeToAppLocale(lng: string): AppLocale {
  return isChineseLocale(lng) ? "zh" : "en";
}

/** i18n 尚未就绪或异常时 language 可能为空，供 getAppLocale / 启动时 document.lang 使用 */
function rawI18nLanguageTag(): string {
  const raw = i18n.resolvedLanguage ?? i18n.language;
  return typeof raw === "string" ? raw.trim() : "";
}

function readStoredLocale(): AppLocale | null {
  try {
    const s = localStorage.getItem(LOCALE_KEY);
    if (s === "en" || s === "zh") {
      return s;
    }
  } catch {
    /* 存储不可用 */
  }
  return null;
}

function detectInitial(): AppLocale {
  const stored = readStoredLocale();
  if (stored) {
    return stored;
  }
  if (typeof navigator !== "undefined" && isChineseLocale(navigator.language)) {
    return "zh";
  }
  return "en";
}

function applyDocumentLang(locale: AppLocale) {
  if (typeof document === "undefined") {
    return;
  }
  // 资源仅区分简中/英文，HTML lang 对中文统一使用 zh-Hans
  document.documentElement.lang = locale === "zh" ? "zh-Hans" : "en";
}

void i18n.use(initReactI18next).init({
  resources: {
    en: { translation: en },
    zh: { translation: zh },
  },
  lng: detectInitial(),
  fallbackLng: "en",
  interpolation: { escapeValue: false },
});

{
  const lng0 = rawI18nLanguageTag();
  applyDocumentLang(lng0 ? normalizeToAppLocale(lng0) : detectInitial());
}

i18n.on("languageChanged", (lng) => {
  const locale = normalizeToAppLocale(lng);
  applyDocumentLang(locale);
  try {
    localStorage.setItem(LOCALE_KEY, locale);
  } catch {
    /* 忽略 */
  }
});

export function setAppLocale(lng: AppLocale) {
  void i18n.changeLanguage(lng);
}

/** 供 Tauri IPC（挑战回顾问句/点评语言）与当前界面语言对齐 */
export function getAppLocale(): AppLocale {
  const lng = rawI18nLanguageTag();
  if (!lng) {
    return detectInitial();
  }
  return normalizeToAppLocale(lng);
}

export default i18n;
