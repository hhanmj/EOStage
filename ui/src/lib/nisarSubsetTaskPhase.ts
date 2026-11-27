// Task-card phase of a running NISAR subset task, derived only from Store
// facts (review 2026-10-17 round 2, P2-2; no persisted total is involved):
// - active_progress groups the task's active items by their current progress
//   row: none yet in this run, a positive transfer rate, a zero rate, or a
//   zero-rate row whose status is queued. A NISAR subset worker publishes a
//   zero-rate row (without a total) exactly when its science transfer
//   finished and the output is being produced, and a queued row while its
//   writer waits for the output (downloader D1: another writer of the same
//   artifact or a free output slot);
// - preparing/verifying: no active item has published progress in this run
//   (recovery checks, authentication and the source probe run first);
//   verifying when verified science bytes already exist.
// Several active items (task concurrency > 1) share one label, by precedence
// transferring > writing > waiting > verifying/preparing; the label also
// counts the items in a later phase (writing, waiting).
export type NisarSubsetRunningPhase = "preparing" | "verifying" | "transferring" | "writing" | "waiting";

export type NisarSubsetActiveProgress = {
  without_progress: number;
  transferring: number;
  idle: number;
  /** Absent on hosts before downloader D1. */
  waiting?: number;
};

export type NisarSubsetPhaseInput = {
  active_downloads?: ReadonlyArray<{ bytes: number }> | null;
  active_progress?: NisarSubsetActiveProgress | null;
  reusable_verified_bytes?: number | null;
};

export function nisarSubsetRunningPhase(status: NisarSubsetPhaseInput): NisarSubsetRunningPhase {
  const progress = status.active_progress;
  if (progress) {
    if (progress.transferring > 0) return "transferring";
    if (progress.idle > 0) return "writing";
    if ((progress.waiting ?? 0) > 0) return "waiting";
  } else if ((status.active_downloads ?? []).some((active) => active.bytes > 0)) {
    // A host without the phase facts: arriving bytes are still a transfer.
    return "transferring";
  }
  return (status.reusable_verified_bytes ?? 0) > 0 ? "verifying" : "preparing";
}

const PHASE_LABELS: Record<NisarSubsetRunningPhase, string> = {
  preparing: "正在准备裁剪任务",
  verifying: "正在复核已下载分段",
  transferring: "正在传输裁剪范围",
  writing: "正在生成裁剪结果",
  waiting: "等待写出",
};

export function nisarSubsetRunningPhaseLabel(status: NisarSubsetPhaseInput): string {
  const phase = nisarSubsetRunningPhase(status);
  const writing = status.active_progress?.idle ?? 0;
  const waiting = status.active_progress?.waiting ?? 0;
  const others: string[] = [];
  if (phase === "transferring" && writing > 0) others.push(`${writing} 项正在生成裁剪结果`);
  if ((phase === "transferring" || phase === "writing") && waiting > 0) others.push(`${waiting} 项等待写出`);
  return others.length > 0 ? `${PHASE_LABELS[phase]}（另有 ${others.join("，")}）` : PHASE_LABELS[phase];
}
