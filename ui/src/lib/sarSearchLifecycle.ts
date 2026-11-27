export type SarSearchLifecyclePhase =
  | "idle"
  | "running"
  | "cancelling"
  | "cancelled"
  | "completed"
  | "failed";

export type SarSearchLifecycle = {
  phase: SarSearchLifecyclePhase;
  generation: number;
  requestId: string | null;
};

export type SarSearchTerminalOutcome = "cancelled" | "completed" | "failed";

export function initialSarSearchLifecycle(): SarSearchLifecycle {
  return { phase: "idle", generation: 0, requestId: null };
}

export function beginSarSearch(state: SarSearchLifecycle, requestId: string): SarSearchLifecycle {
  if (state.phase !== "idle" || !requestId.trim()) return state;
  return {
    phase: "running",
    generation: state.generation + 1,
    requestId,
  };
}

export function requestSarSearchCancellation(
  state: SarSearchLifecycle,
  requestId: string,
): SarSearchLifecycle {
  return state.phase === "running" && state.requestId === requestId
    ? { ...state, phase: "cancelling", generation: state.generation + 1 }
    : state;
}

export function rejectSarSearchCancellation(
  state: SarSearchLifecycle,
  requestId: string,
): SarSearchLifecycle {
  return state.phase === "cancelling" && state.requestId === requestId
    ? { ...state, phase: "running" }
    : state;
}

export function completeSarSearch(
  state: SarSearchLifecycle,
  requestId: string,
  outcome: SarSearchTerminalOutcome,
): SarSearchLifecycle {
  if (state.requestId !== requestId || (state.phase !== "running" && state.phase !== "cancelling")) {
    return state;
  }
  return { ...state, phase: outcome };
}

export function settleSarSearch(state: SarSearchLifecycle): SarSearchLifecycle {
  return state.phase === "completed" || state.phase === "failed" || state.phase === "cancelled"
    ? { phase: "idle", generation: state.generation, requestId: null }
    : state;
}

export function sarSearchAcceptsResult(
  state: SarSearchLifecycle,
  requestId: string,
): boolean {
  return state.phase === "running" && state.requestId === requestId;
}

export function sarSearchCancellationPending(state: SarSearchLifecycle): boolean {
  return state.phase === "cancelling";
}

export function sarSearchIsBusy(state: SarSearchLifecycle): boolean {
  // Cancelling is intentionally not a foreground-busy state. The old request
  // keeps its requestId until backend acknowledgement/settlement, but its
  // results are no longer eligible for UI projection.
  return state.phase === "running";
}
