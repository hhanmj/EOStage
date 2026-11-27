import type { DesktopApplicationError } from "./desktopShellBridge.ts";

export type OpticalPreparationProgress = {
  request_id: string;
  requested_count: number;
  prepared_count: number;
  admitted_count: number;
  rejected_count: number;
  phase: "preparing" | "committing";
};
export type OpticalPreparationResult = { ok: true; progress: OpticalPreparationProgress | null } | DesktopApplicationError;

export function parseOpticalPreparation(value: unknown, requestId: string): OpticalPreparationProgress | null {
  if (!value || typeof value !== "object" || Array.isArray(value)) return null;
  const raw = value as Record<string, unknown>;
  if (Object.keys(raw).some(key => !["request_id", "requested_count", "prepared_count", "admitted_count", "rejected_count", "phase"].includes(key))
    || raw.request_id !== requestId || !requestId.trim()
    || !["requested_count", "prepared_count", "admitted_count", "rejected_count"].every(key =>
      typeof raw[key] === "number" && Number.isSafeInteger(raw[key]) && (raw[key] as number) >= 0)) return null;
  const requested = raw.requested_count as number, prepared = raw.prepared_count as number;
  const admitted = raw.admitted_count as number, rejected = raw.rejected_count as number;
  if (requested < 1 || requested > 250 || prepared > requested || prepared !== admitted + rejected
    || raw.phase !== "preparing" && raw.phase !== "committing"
    || raw.phase === "committing" && (prepared !== requested || admitted === 0)) return null;
  return { request_id: requestId, requested_count: requested, prepared_count: prepared,
    admitted_count: admitted, rejected_count: rejected, phase: raw.phase };
}

/** One read at a time. Stopping observation never cancels or repeats the mutation. */
export function observeOpticalPreparation(
  requestId: string,
  read: (requestId: string) => Promise<OpticalPreparationResult>,
  publish: (progress: OpticalPreparationProgress | null, unavailable: boolean) => void,
  intervalMs = 350,
): () => void {
  let stopped = false, last: OpticalPreparationProgress | null = null;
  let timer: ReturnType<typeof setTimeout> | undefined;
  const tick = async () => {
    try {
      const result = await read(requestId);
      if (stopped) return;
      if (!result.ok) { publish(null, true); }
      else if (result.progress === null) { publish(last, false); }
      else {
        const progress = parseOpticalPreparation(result.progress, requestId);
        if (!progress || last && (progress.requested_count !== last.requested_count
          || progress.prepared_count < last.prepared_count || progress.admitted_count < last.admitted_count
          || progress.rejected_count < last.rejected_count || last.phase === "committing" && progress.phase !== last.phase)) {
          publish(null, true);
        } else { last = progress; publish(progress, false); }
      }
    } catch { if (!stopped) publish(null, true); }
    if (!stopped) timer = setTimeout(() => void tick(), intervalMs);
  };
  void tick();
  return () => { stopped = true; clearTimeout(timer); };
}
