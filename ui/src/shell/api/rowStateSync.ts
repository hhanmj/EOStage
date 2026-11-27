import type { RowTaskState } from "./types.ts";

function isRowStateEqual(a: RowTaskState, b: RowTaskState): boolean {
  if (
    a.item_id !== b.item_id ||
    a.task_id !== b.task_id ||
    a.task_status !== b.task_status ||
    a.item_status !== b.item_status
  ) {
    return false;
  }
  const aActions = a.actions || [];
  const bActions = b.actions || [];
  if (aActions.length !== bActions.length) return false;
  for (let i = 0; i < aActions.length; i++) {
    if (aActions[i] !== bActions[i]) return false;
  }
  return true;
}

/**
 * C1 / D5-1: 纯函数合并行级任务状态
 * 遍历后端返回的 rows，与 prev Map 比对；若有实质变动则返回新的 Map 与 changed=true，否则 changed=false。
 */
export function mergeRowStates(
  prev: Map<string, RowTaskState>,
  rows: RowTaskState[]
): { next: Map<string, RowTaskState>; changed: boolean } {
  if (!rows || rows.length === 0) {
    return { next: prev, changed: false };
  }

  let changed = false;
  for (const row of rows) {
    const existing = prev.get(row.item_id);
    if (!existing || !isRowStateEqual(existing, row)) {
      changed = true;
      break;
    }
  }

  if (!changed) {
    return { next: prev, changed: false };
  }

  const next = new Map(prev);
  for (const row of rows) {
    next.set(row.item_id, row);
  }
  return { next, changed: true };
}
