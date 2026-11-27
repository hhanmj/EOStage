import type { Messages } from "../../types.ts";

export const common: Messages["common"] = {
  locale: {
    "zh-CN": "简体中文",
    en: "English",
    ja: "日本語",
  },
  sample: {
    greeting: "Hello, {name}",
    count_one: "{count} item",
    count_other: "{count} items",
  },
};
