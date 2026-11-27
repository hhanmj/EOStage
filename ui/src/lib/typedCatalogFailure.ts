import { formatBridgeErrorValue } from "./bridgeErrorFormat.ts";

export function formatTypedCatalogFailure(sourceLabel: string, error: unknown): string {
  const record = typeof error === "object" && error !== null
    ? error as Record<string, unknown>
    : null;
  const code = typeof record?.code === "string" && /^[A-Z0-9_]{1,80}$/.test(record.code)
    ? record.code
    : null;
  const heading = `${sourceLabel.trim() || "数据集"} 检索失败`;
  const message = formatBridgeErrorValue(error);
  return code ? `${heading}\n${message}\n错误代码 ${code}` : `${heading}\n${message}`;
}
