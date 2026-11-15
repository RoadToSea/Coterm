import { zhCN } from "../../../locales/zh-CN";
import type { DriverI18n } from "../../../../../shared/index";

export function createTranslator(i18n?: DriverI18n) {
  return (message: string, values: Record<string, string | number> = {}): string => {
    const template = i18n?.locale === "zh-CN" ? zhCN[message] ?? message : message;
    return template.replace(/\{(\w+)\}/g, (match, key: string) => String(values[key] ?? match));
  };
}

export type Translator = ReturnType<typeof createTranslator>;
