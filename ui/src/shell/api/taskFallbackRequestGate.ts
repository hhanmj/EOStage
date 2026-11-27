import { t } from "../../i18n/index.ts";
import type { TaskRetryFallbackOutcome } from "./types.ts";

export type TaskFallbackRequestStep = "first" | "second";

export interface TaskFallbackRequestGate {
  taskId: string;
  taskRevision: number;
  step: TaskFallbackRequestStep;
  generation: number;
  pending: boolean;
  uncertain: boolean;
  outcome: TaskRetryFallbackOutcome | null;
  error: string | null;
}

type GateListener = (taskId: string, gate: TaskFallbackRequestGate | null) => void;

const gates = new Map<string, TaskFallbackRequestGate>();
const listeners = new Set<GateListener>();
let nextGeneration = 0;

function cloneOutcome(outcome: TaskRetryFallbackOutcome): TaskRetryFallbackOutcome {
  return {
    ...outcome,
    task: { ...outcome.task },
    rebuilt_item_ids: [...outcome.rebuilt_item_ids],
    missing_product_ids: [...outcome.missing_product_ids],
    added_product_ids: [...outcome.added_product_ids],
  };
}

function cloneGate(gate: TaskFallbackRequestGate): TaskFallbackRequestGate {
  return {
    ...gate,
    outcome: gate.outcome ? cloneOutcome(gate.outcome) : null,
  };
}

function notify(taskId: string, gate: TaskFallbackRequestGate | null): void {
  const snapshot = gate ? cloneGate(gate) : null;
  for (const listener of listeners) listener(taskId, snapshot);
}

export function subscribeTaskFallbackRequestGate(listener: GateListener): () => void {
  listeners.add(listener);
  return () => listeners.delete(listener);
}

/**
 * A request gate is session-scoped UI coordination, not task truth. It survives
 * drawer/page unmounts so an already-dispatched request cannot be replayed just
 * because its presentation was closed. The server request is not cancellable
 * through this gate.
 */
export function getTaskFallbackRequestGate(
  taskId: string,
  taskRevision?: number,
): TaskFallbackRequestGate | null {
  const gate = gates.get(taskId);
  if (!gate) return null;
  if (gate.pending || gate.uncertain) {
    return cloneGate(gate);
  }
  if (taskRevision !== undefined && gate.taskRevision !== taskRevision) {
    return null;
  }
  return cloneGate(gate);
}

export function startTaskFallbackRequest(params: {
  taskId: string;
  taskRevision: number;
  step: TaskFallbackRequestStep;
  outcome?: TaskRetryFallbackOutcome | null;
}): TaskFallbackRequestGate | null {
  const existing = gates.get(params.taskId);
  if (existing) {
    if (existing.pending || existing.uncertain) {
      return null;
    }
    if (params.step === "second") {
      if (
        existing.taskRevision !== params.taskRevision ||
        existing.step !== "first" ||
        existing.outcome?.needs_user_confirmation !== true ||
        existing.outcome.task.task_id !== params.taskId ||
        existing.outcome.task.revision !== params.taskRevision
      ) {
        return null;
      }
      if (params.outcome) {
        if (
          params.outcome.task?.task_id !== existing.outcome.task.task_id ||
          params.outcome.task?.revision !== existing.outcome.task.revision ||
          params.outcome.refreshed_batch_id !== existing.outcome.refreshed_batch_id
        ) {
          return null;
        }
      }
    } else {
      if (existing.taskRevision === params.taskRevision) {
        return null;
      }
      gates.delete(params.taskId);
    }
  } else {
    if (params.step === "second") {
      return null;
    }
  }

  const gate: TaskFallbackRequestGate = {
    taskId: params.taskId,
    taskRevision: params.taskRevision,
    step: params.step,
    generation: ++nextGeneration,
    pending: true,
    uncertain: false,
    outcome: params.step === "second" && existing?.outcome ? cloneOutcome(existing.outcome) : null,
    error: null,
  };
  gates.set(params.taskId, gate);
  notify(params.taskId, gate);
  return cloneGate(gate);
}

function matches(
  gate: TaskFallbackRequestGate | undefined,
  identity: { taskId: string; taskRevision: number; step: TaskFallbackRequestStep; generation: number },
): gate is TaskFallbackRequestGate {
  return Boolean(
    gate &&
      gate.taskId === identity.taskId &&
      gate.taskRevision === identity.taskRevision &&
      gate.step === identity.step &&
      gate.generation === identity.generation &&
      gate.pending,
  );
}

export function settleTaskFallbackRequest(
  identity: { taskId: string; taskRevision: number; step: TaskFallbackRequestStep; generation: number },
  outcome: TaskRetryFallbackOutcome,
): TaskFallbackRequestGate | null {
  const gate = gates.get(identity.taskId);
  if (!matches(gate, identity)) return null;
  gate.pending = false;
  gate.uncertain = false;
  gate.outcome = cloneOutcome(outcome);
  gate.error = null;
  gates.set(identity.taskId, gate);
  notify(identity.taskId, gate);
  return cloneGate(gate);
}

export function markTaskFallbackRequestUncertain(
  identity: { taskId: string; taskRevision: number; step: TaskFallbackRequestStep; generation: number },
  error: string,
  opts?: { dropOutcome?: boolean },
): TaskFallbackRequestGate | null {
  const gate = gates.get(identity.taskId);
  if (!matches(gate, identity)) return null;
  gate.pending = false;
  gate.uncertain = true;
  gate.error = error;
  if (opts?.dropOutcome) {
    gate.outcome = null;
  }
  gates.set(identity.taskId, gate);
  notify(identity.taskId, gate);
  return cloneGate(gate);
}

export function clearTaskFallbackRequest(identity: {
  taskId: string;
  taskRevision: number;
  generation?: number;
}): boolean {
  const gate = gates.get(identity.taskId);
  if (
    !gate ||
    gate.taskRevision !== identity.taskRevision ||
    (identity.generation !== undefined && gate.generation !== identity.generation)
  ) {
    return false;
  }
  gates.delete(identity.taskId);
  notify(identity.taskId, null);
  return true;
}

/**
 * Reconciles the session gate against an authoritative task snapshot.
 * Reconcile must NEVER delete or release a pending or uncertain gate.
 */
export function reconcileTaskFallbackRequestSnapshot(
  taskId: string,
  task: { revision: number; status: string } | null,
): TaskFallbackRequestGate | null {
  const gate = gates.get(taskId);
  if (!gate) return null;
  if (task && gate.taskRevision === task.revision && task.status === "failed") {
    return cloneGate(gate);
  }
  if (gate.pending) {
    gate.pending = false;
    gate.uncertain = true;
    gate.error = t("shared.taskFallbackGate.stateChangedUncertainError");
    gates.set(taskId, gate);
    notify(taskId, gate);
    return cloneGate(gate);
  }
  if (gate.uncertain) {
    return cloneGate(gate);
  }
  if (task && task.status === "failed" && gate.taskRevision !== task.revision) {
    gates.delete(taskId);
    return null;
  }
  return cloneGate(gate);
}

export function reconcileTaskFallbackRequest(task: {
  task_id: string;
  revision: number;
  status: string;
}): TaskFallbackRequestGate | null;
export function reconcileTaskFallbackRequest(
  taskId: string,
  task: { revision: number; status: string } | null,
): TaskFallbackRequestGate | null;
export function reconcileTaskFallbackRequest(
  taskIdOrTask: string | { task_id: string; revision: number; status: string },
  maybeTask?: { revision: number; status: string } | null,
): TaskFallbackRequestGate | null {
  if (typeof taskIdOrTask === "string") {
    return reconcileTaskFallbackRequestSnapshot(taskIdOrTask, maybeTask ?? null);
  }
  return reconcileTaskFallbackRequestSnapshot(taskIdOrTask.task_id, taskIdOrTask);
}

export function resetTaskFallbackRequestGatesForTests(): void {
  gates.clear();
  nextGeneration = 0;
}
