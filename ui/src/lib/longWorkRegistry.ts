/**
 * Lightweight subscription registry for non-TaskStore cross-page long operations
 * (such as session export, plugin bundle import, and extension registry sync/install)
 * so the single top-right ProgressNotification host can track them across tab switches.
 * Download/DEM/GEE tasks stay owned by TaskStore / CoreRestore.
 */

export type LongWorkCategory = "plugin" | "export" | "transfer" | "generic";
export type LongWorkStatus = "running" | "completed" | "failed" | "cancelled";

export interface LongWorkItem {
  id: string;
  title: string;
  category: LongWorkCategory;
  current: number;
  total: number | null;
  unitLabel?: string;
  stageLabel?: string;
  doneTitle?: string;
  doneSubtitle?: string;
  errorMessage?: string;
  startedAtMs: number;
  updatedAtMs: number;
  finishedAtMs?: number;
  status: LongWorkStatus;
  onCancel?: () => void;
  onOpen?: () => void;
}

type Listener = (items: LongWorkItem[]) => void;

const itemsMap = new Map<string, LongWorkItem>();
const listeners = new Set<Listener>();

function emit(): void {
  const snapshot = Array.from(itemsMap.values()).sort((a, b) => b.startedAtMs - a.startedAtMs);
  for (const listener of listeners) {
    try {
      listener(snapshot);
    } catch {
      // ignore subscriber failure
    }
  }
}

export function getLongWorkItems(): LongWorkItem[] {
  return Array.from(itemsMap.values()).sort((a, b) => b.startedAtMs - a.startedAtMs);
}

export function subscribeLongWork(listener: Listener): () => void {
  listeners.add(listener);
  listener(getLongWorkItems());
  return () => {
    listeners.delete(listener);
  };
}

export function startLongWork(input: {
  id?: string;
  title: string;
  category?: LongWorkCategory;
  current?: number;
  total?: number | null;
  unitLabel?: string;
  stageLabel?: string;
  startedAtMs?: number;
  onCancel?: () => void;
  onOpen?: () => void;
}): string {
  const id = input.id || `work-${Date.now()}-${Math.random().toString(16).slice(2, 7)}`;
  const now = input.startedAtMs ?? Date.now();
  itemsMap.set(id, {
    id,
    title: input.title,
    category: input.category ?? "generic",
    current: input.current ?? 0,
    total: input.total ?? null,
    unitLabel: input.unitLabel,
    stageLabel: input.stageLabel,
    startedAtMs: now,
    updatedAtMs: now,
    status: "running",
    onCancel: input.onCancel,
    onOpen: input.onOpen,
  });
  emit();
  return id;
}

export function updateLongWork(
  id: string,
  patch: Partial<Pick<LongWorkItem, "title" | "current" | "total" | "unitLabel" | "stageLabel" | "onCancel" | "onOpen">>
): void {
  const existing = itemsMap.get(id);
  if (!existing || existing.status !== "running") return;
  itemsMap.set(id, {
    ...existing,
    ...patch,
    updatedAtMs: Date.now(),
  });
  emit();
}

export function completeLongWork(
  id: string,
  options?: {
    doneTitle?: string;
    doneSubtitle?: string;
    minVisibleThresholdMs?: number;
    onOpen?: () => void;
  }
): void {
  const existing = itemsMap.get(id);
  if (!existing) return;
  const now = Date.now();
  const threshold = options?.minVisibleThresholdMs ?? 350;
  // Finished before the notification threshold: remove cleanly without popping up.
  if (now - existing.startedAtMs < threshold) {
    itemsMap.delete(id);
    emit();
    return;
  }
  itemsMap.set(id, {
    ...existing,
    current: existing.total ?? existing.current,
    status: "completed",
    doneTitle: options?.doneTitle ?? existing.title,
    doneSubtitle: options?.doneSubtitle ?? "已完成 · 点击查看",
    finishedAtMs: now,
    updatedAtMs: now,
    onOpen: options?.onOpen ?? existing.onOpen,
  });
  emit();
}

export function failLongWork(id: string, errorMessage: string): void {
  const existing = itemsMap.get(id);
  if (!existing) return;
  const now = Date.now();
  itemsMap.set(id, {
    ...existing,
    status: "failed",
    errorMessage,
    finishedAtMs: now,
    updatedAtMs: now,
  });
  emit();
}

export function dismissLongWork(id: string): void {
  if (itemsMap.delete(id)) {
    emit();
  }
}

export function clearAllLongWorkForTest(): void {
  itemsMap.clear();
  emit();
}
