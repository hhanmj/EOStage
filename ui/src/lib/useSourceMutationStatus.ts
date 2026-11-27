import { useEffect, useMemo, useState, useSyncExternalStore } from "react";
import { querySourceMutationStatus } from "@/lib/bridge";
import type { DesktopSourceMutationQuery, DesktopSourceMutationStatusResult } from "@/lib/desktopShellBridge";
import { getSourceMutationGate, type SourceMutationGate } from "@/lib/taskSourceMutationGate";
import { isSourceOperationInFlight, sourceOperationAttempt, sourceOperationSnapshot, subscribeSourceOperation } from "@/lib/sourceOperationCoordination";

function queryForGate(gate: SourceMutationGate): DesktopSourceMutationQuery | null {
  const frozen = gate.frozenMutation;
  if (!frozen) return null;
  switch (frozen.kind) {
    case "append_source_batch": return { kind: "source_append", request: frozen.request };
    case "successor_source_batch": return { kind: "historical_successor", request: frozen.request };
    case "burst_append": return { kind: "burst_append", request: frozen.request };
    case "burst_successor": return { kind: "burst_successor", request: frozen.request };
    case "nisar_append": return { kind: "nisar_append", request: frozen.request };
    case "nisar_successor": return { kind: "nisar_successor", request: frozen.request };
  }
}

/** Read-only evidence for this exact local gate; it never changes the gate. */
export function useSourceMutationStatus(taskId: string, gate: SourceMutationGate | null): DesktopSourceMutationStatusResult | null {
  const pending = gate?.pending ?? false;
  const snapshot = useSyncExternalStore(subscribeSourceOperation, () => sourceOperationSnapshot(taskId), () => "0:0");
  const [attemptText, busyText] = snapshot.split(":");
  const attempt = Number(attemptText);
  const busy = busyText === "1";
  const token = useMemo(() => ({}), [taskId, gate, pending, busy, attempt]);
  const [observation, setObservation] = useState<{
    token: object;
    result: DesktopSourceMutationStatusResult;
  } | null>(null);

  useEffect(() => {
    if (!gate || gate.sourceTaskId !== taskId || gate.pending || busy || isSourceOperationInFlight(taskId) || sourceOperationAttempt(taskId) !== attempt) return;
    const query = queryForGate(gate);
    if (!query) return;
    let live = true;
    void querySourceMutationStatus(query).then((result) => {
      if (live && getSourceMutationGate(taskId) === gate && !gate.pending && !isSourceOperationInFlight(taskId) && sourceOperationAttempt(taskId) === attempt) {
        setObservation({ token, result });
      }
    }).catch(() => {
      if (live && getSourceMutationGate(taskId) === gate && !gate.pending && !isSourceOperationInFlight(taskId) && sourceOperationAttempt(taskId) === attempt) {
        setObservation({ token, result: { ok: false, error: "Status query unavailable", code: "V3_APPLICATION_UNAVAILABLE" } });
      }
    });
    return () => { live = false; };
  }, [taskId, gate, pending, busy, attempt, token]);

  return gate && !gate.pending && !busy && gate.sourceTaskId === taskId && observation?.token === token
    ? observation.result : null;
}
