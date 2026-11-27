export const ORBIT_LEGEND_LABELS_EN = {
  ascending: "Ascending",
  descending: "Descending",
  unknown: "Unknown",
} as const;

export function formatOrbitTerms(text: string): string {
  if (!text) {
    return text;
  }
  // 1. 先替换复合词「升降轨」，避免被子项拆分
  let result = text.replaceAll("升降轨", "Ascending/Descending"); // i18n-exempt: input orbit term normalization pattern
  // 2. 替换「升轨」与「降轨」
  result = result.replaceAll("升轨", "Ascending"); // i18n-exempt: input orbit term normalization pattern
  result = result.replaceAll("降轨", "Descending"); // i18n-exempt: input orbit term normalization pattern
  // 3. 全词大写 ASCENDING / DESCENDING 替换为 Ascending / Descending
  result = result.replace(/\bASCENDING\b/g, "Ascending");
  result = result.replace(/\bDESCENDING\b/g, "Descending");
  return result;
}
