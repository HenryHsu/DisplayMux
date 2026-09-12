import en from "./locales/en";
import zhTW from "./locales/zh-TW";

export type AppLocale = "en" | "zh-TW";
export type MessageKey = keyof typeof en;
type Parameters = Record<string, string | number>;

function detectLocale(languageTags: readonly string[]): AppLocale {
  for (const tag of languageTags) {
    const normalized = tag.toLowerCase();
    if (normalized.startsWith("zh-tw") || normalized.startsWith("zh-hant") ||
      normalized.startsWith("zh-hk") || normalized.startsWith("zh-mo")) return "zh-TW";
    if (normalized.startsWith("en") || normalized.startsWith("zh")) return "en";
  }
  return "en";
}

export const locale = detectLocale(
  typeof navigator === "undefined" ? [] : (navigator.languages.length ? navigator.languages : [navigator.language]),
);

const messages: Record<AppLocale, Record<MessageKey, string>> = { en, "zh-TW": zhTW };

export function t(key: MessageKey, parameters: Parameters = {}): string {
  const message = messages[locale][key] ?? en[key];
  return message.replace(/\{(\w+)\}/g, (placeholder, name: string) =>
    Object.prototype.hasOwnProperty.call(parameters, name) ? String(parameters[name]) : placeholder,
  );
}

export const __test__ = { detectLocale };
