export function interpolatedTaskElapsed(
  baseSeconds: number,
  state: string,
  observedAtMs: number,
  nowMs: number,
): number {
  void state;
  void observedAtMs;
  void nowMs;
  return Math.max(0, Number.isFinite(baseSeconds) ? baseSeconds : 0);
}
