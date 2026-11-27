export function taskTimestampMs(value: unknown): number {
  if (typeof value === "number" && Number.isFinite(value) && value > 0) {
    return value >= 1_000_000_000_000 ? value : value * 1_000;
  }
  if (typeof value !== "string") return 0;
  const text = value.trim();
  if (!text) return 0;
  if (/^\d+(?:\.\d+)?$/.test(text)) {
    return taskTimestampMs(Number(text));
  }
  const parsed = Date.parse(text);
  return Number.isFinite(parsed) ? parsed : 0;
}

/** Stable newest-first ordering for task and history surfaces. */
export function newestTaskFirst<T>(
  items: readonly T[],
  timestampFor: (item: T) => unknown,
): T[] {
  return items
    .map((item, index) => ({ item, index, timestamp: taskTimestampMs(timestampFor(item)) }))
    .sort((left, right) => right.timestamp - left.timestamp || left.index - right.index)
    .map(({ item }) => item);
}
