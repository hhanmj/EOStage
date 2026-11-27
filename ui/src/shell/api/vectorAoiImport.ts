import { t } from "../../i18n/index.ts";
import type { Bbox } from "../../lib/bridge";

/**
 * 从 Data URL (data:...;base64,XXXX) 中提取纯 base64 数据
 */
export function extractBase64FromDataUrl(dataUrl: string): string {
  if (!dataUrl || typeof dataUrl !== "string") return "";
  const commaIndex = dataUrl.indexOf(",");
  if (commaIndex >= 0) {
    return dataUrl.slice(commaIndex + 1);
  }
  return dataUrl;
}

/**
 * 校验并转换 4 元组数组为 Bbox 对象，无效或含 NaN 时返回 null
 */
export function bboxArrayToBbox(bboxArray: unknown, crs: string = "EPSG:4326"): Bbox | null {
  if (!Array.isArray(bboxArray) || bboxArray.length !== 4) {
    return null;
  }
  const [west, south, east, north] = bboxArray;
  if (
    typeof west !== "number" ||
    !Number.isFinite(west) ||
    typeof south !== "number" ||
    !Number.isFinite(south) ||
    typeof east !== "number" ||
    !Number.isFinite(east) ||
    typeof north !== "number" ||
    !Number.isFinite(north)
  ) {
    return null;
  }
  return { west, south, east, north, crs };
}

/**
 * 读取 File 对象为纯 base64 字符串
 */
export function readFileAsBase64(file: File): Promise<string> {
  return new Promise((resolve, reject) => {
    const reader = new FileReader();
    reader.onload = () => {
      const result = reader.result;
      if (typeof result === "string") {
        resolve(extractBase64FromDataUrl(result));
      } else {
        reject(new Error(t("shared.vectorAoi.fileReadFailed")));
      }
    };
    reader.onerror = () => reject(reader.error || new Error(t("shared.vectorAoi.fileReadFailed")));
    reader.readAsDataURL(file);
  });
}
