import { useEffect, useMemo, useState, useSyncExternalStore } from "react";
import { querySourceMutationStatus } from "@/lib/bridge";
import type { DesktopSourceMutationQuery, DesktopSourceMutationStatusResult } from "@/lib/desktopShellBridge";
import { getPendingReacquireRecord, isInFlight, type ReacquireRecord } from "@/lib/reacquireRequestGate";
import { sourceOperationAttempt, sourceOperationSnapshot, subscribeSourceOperation } from "@/lib/sourceOperationCoordination";

function queryForRecord(record: ReacquireRecord): DesktopSourceMutationQuery {
  const request = record.request;
  if (request.kind === "active") {
    return { kind: "reacquire", request: {
      task_id: request.task_id, expected_revision: request.expected_revision,
      command: request.command, event_id: request.event_id, occurred_at: request.occurred_at,
    } };
  }
  return { kind: "historical_successor", request: {
    source_task_id: request.source_task_id, expected_source_task_revision: request.expected_source_task_revision,
    new_task_id: request.new_task_id, item_ids: request.item_ids, output_root: request.output_root,
    layout: request.layout, concurrency: request.concurrency, created_at: request.created_at,
    reacquire_item_ids: request.reacquire_item_ids,
  } };
}

/** A snapshot for the stored request only; querying never alters its gate. */
export function useReacquireStatus(taskId: string, record: ReacquireRecord | null): DesktopSourceMutationStatusResult | null {
  const snapshot = useSyncExternalStore(subscribeSourceOperation, () => sourceOperationSnapshot(taskId), () => "0:0");
  const [attemptText, busyText] = snapshot.split(":");
  const attempt = Number(attemptText);
  const busy = busyText === "1";
  const token = useMemo(() => ({}), [taskId, record, busy, attempt]);
  const [observation, setObservation] = useState<{ token: object; result: DesktopSourceMutationStatusResult } | null>(null);
  useEffect(() => {
    if (!record || record.sourceTaskId !== taskId || busy || isInFlight(taskId) || sourceOperationAttempt(taskId) !== attempt) return;
    let live = true;
    const save = (result: DesktopSourceMutationStatusResult) => {
      if (live && getPendingReacquireRecord(taskId) === record && !isInFlight(taskId) && sourceOperationAttempt(taskId) === attempt) {
        setObservation({ token, result });
      }
    };
    void querySourceMutationStatus(queryForRecord(record)).then(save).catch(() =>
      save({ ok: false, error: "Status query unavailable", code: "V3_APPLICATION_UNAVAILABLE" }));
    return () => { live = false; };
  }, [taskId, record, busy, attempt, token]);
  return record && !busy && record.sourceTaskId === taskId && observation?.token === token
    ? observation.result : null;
}
