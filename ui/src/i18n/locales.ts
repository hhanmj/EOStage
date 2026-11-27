import { en } from "./locales/en/index.ts";
import { ja } from "./locales/ja/index.ts";
import { zhCN } from "./locales/zh-CN/index.ts";
import type { Locale, Messages } from "./types.ts";

export const MESSAGES: Record<Locale, Messages> = { "zh-CN": zhCN, en, ja };
