import type { AdminOption } from "./bridge.ts";

const HEAD_OPTIONS = new Set(["全国", "全部", "不限"]); // i18n-exempt: Chinese administrative region data names per Q7

export const CANONICAL_CHINA_PROVINCE_ORDER = [
  "北京市", "天津市", "河北省", "山西省", "内蒙古自治区", "辽宁省", "吉林省", "黑龙江省", // i18n-exempt: Chinese administrative region data names per Q7
  "上海市", "江苏省", "浙江省", "安徽省", "福建省", "江西省", "山东省", "河南省", // i18n-exempt: Chinese administrative region data names per Q7
  "湖北省", "湖南省", "广东省", "广西壮族自治区", "海南省", "重庆市", "四川省", "贵州省", // i18n-exempt: Chinese administrative region data names per Q7
  "云南省", "西藏自治区", "陕西省", "甘肃省", "青海省", "宁夏回族自治区", // i18n-exempt: Chinese administrative region data names per Q7
  "新疆维吾尔自治区", "台湾省", "香港特别行政区", "澳门特别行政区", // i18n-exempt: Chinese administrative region data names per Q7
] as const;

const PROVINCE_DISPLAY_ORDER = new Map<string, number>(
  CANONICAL_CHINA_PROVINCE_ORDER.map((name, index) => [name, index]),
);

function exactAdcode(option: AdminOption): string | null {
  const value = option.provider_object_id;
  return value && /^\d{6}$/.test(value) ? value : null;
}

/** Stable final display order without manufacturing identity for local fallbacks. */
export function stableAdminOptionOrder(options: readonly AdminOption[]): AdminOption[] {
  return [...options].sort((left, right) => {
    const leftHead = HEAD_OPTIONS.has(left.name);
    const rightHead = HEAD_OPTIONS.has(right.name);
    if (leftHead !== rightHead) return leftHead ? -1 : 1;
    const leftProvince = PROVINCE_DISPLAY_ORDER.get(left.name);
    const rightProvince = PROVINCE_DISPLAY_ORDER.get(right.name);
    if (leftProvince !== undefined && rightProvince !== undefined) return leftProvince - rightProvince;
    if (leftProvince !== undefined || rightProvince !== undefined) return leftProvince !== undefined ? -1 : 1;
    const leftAdcode = exactAdcode(left);
    const rightAdcode = exactAdcode(right);
    if (leftAdcode && rightAdcode) return leftAdcode.localeCompare(rightAdcode);
    if (leftAdcode !== rightAdcode) return leftAdcode ? -1 : 1;
    return 0;
  });
}
