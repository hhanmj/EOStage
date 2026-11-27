/**
 * 来源显示名：只经数据源名称映射（键为 resource_id）取得，映射不到即为 null。
 * 不用启发式猜测某个字符串「像不像」显示名——provider_id、catalog id 等原文一律不显示。
 */
export function mappedSourceName(
  id: unknown,
  sourceNames: Readonly<Record<string, string>> | null | undefined,
): string | null {
  if (typeof id !== "string" || !sourceNames) return null;
  const key = id.trim();
  if (!key) return null;
  const mapped = sourceNames[key];
  if (typeof mapped !== "string") return null;
  const name = mapped.trim();
  return name.length > 0 ? name : null;
}
