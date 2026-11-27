const STORAGE_KEY = "eostage.subsetSizeEstimates.v1";
const MAX_ENTRIES = 200;

interface SubsetEstimateRecord {
  estimatedBytes: number;
  perItemBytes?: Record<string, number>;
  recordedAt: number;
}

interface SubsetEstimateStore {
  byTaskId: Record<string, SubsetEstimateRecord>;
  bySceneId: Record<string, { estimatedBytes: number; recordedAt: number }>;
}

const memoryStore: SubsetEstimateStore = {
  byTaskId: {},
  bySceneId: {},
};

let hydrated = false;

function ensureHydrated(): void {
  if (hydrated) return;
  hydrated = true;
  if (typeof window === "undefined") return;
  try {
    const raw = window.localStorage.getItem(STORAGE_KEY);
    if (!raw) return;
    const parsed = JSON.parse(raw) as Partial<SubsetEstimateStore>;
    if (parsed && typeof parsed === "object") {
      if (parsed.byTaskId && typeof parsed.byTaskId === "object") {
        memoryStore.byTaskId = { ...parsed.byTaskId };
      }
      if (parsed.bySceneId && typeof parsed.bySceneId === "object") {
        memoryStore.bySceneId = { ...parsed.bySceneId };
      }
    }
  } catch {
    // ignore storage read failures
  }
}

function pruneAndPersist(): void {
  const trimMap = <T extends { recordedAt: number }>(rec: Record<string, T>): Record<string, T> => {
    const entries = Object.entries(rec);
    if (entries.length <= MAX_ENTRIES) return rec;
    entries.sort((a, b) => (b[1]?.recordedAt ?? 0) - (a[1]?.recordedAt ?? 0));
    return Object.fromEntries(entries.slice(0, MAX_ENTRIES));
  };
  memoryStore.byTaskId = trimMap(memoryStore.byTaskId);
  memoryStore.bySceneId = trimMap(memoryStore.bySceneId);

  if (typeof window === "undefined") return;
  try {
    window.localStorage.setItem(STORAGE_KEY, JSON.stringify(memoryStore));
  } catch {
    // ignore storage write failures
  }
}

export function clearSubsetSizeEstimatesForTest(): void {
  hydrated = true;
  memoryStore.byTaskId = {};
  memoryStore.bySceneId = {};
  if (typeof window !== "undefined") {
    try {
      window.localStorage.removeItem(STORAGE_KEY);
    } catch {
      // ignore
    }
  }
}

export function recordSubsetSizeEstimate(input: {
  taskId?: string | null;
  totalEstimatedBytes?: number | null;
  sceneEstimates?: Record<string, number> | null;
}): void {
  ensureHydrated();
  const now = Date.now();
  const cleanPerItem: Record<string, number> = {};
  let sumFromItems = 0;

  if (input.sceneEstimates) {
    for (const [rawId, rawBytes] of Object.entries(input.sceneEstimates)) {
      const id = rawId.trim();
      const bytes = Number(rawBytes);
      if (!id || !Number.isFinite(bytes) || bytes <= 0) continue;
      const rounded = Math.round(bytes);
      cleanPerItem[id] = rounded;
      memoryStore.bySceneId[id] = { estimatedBytes: rounded, recordedAt: now };
      sumFromItems += rounded;
    }
  }

  const rawTotal = Number(input.totalEstimatedBytes);
  const totalBytes =
    Number.isFinite(rawTotal) && rawTotal > 0 ? Math.round(rawTotal) : sumFromItems;

  const taskId = (input.taskId || "").trim();
  if (taskId && totalBytes > 0) {
    memoryStore.byTaskId[taskId] = {
      estimatedBytes: totalBytes,
      perItemBytes: Object.keys(cleanPerItem).length > 0 ? cleanPerItem : undefined,
      recordedAt: now,
    };
  }

  pruneAndPersist();
}

export function lookupSubsetSizeEstimate(
  taskId?: string | null,
  sceneIds?: readonly string[] | null,
): number | null {
  ensureHydrated();
  const cleanTaskId = (taskId || "").trim();
  if (cleanTaskId) {
    const rec = memoryStore.byTaskId[cleanTaskId];
    if (rec && Number.isFinite(rec.estimatedBytes) && rec.estimatedBytes > 0) {
      return rec.estimatedBytes;
    }
  }
  if (Array.isArray(sceneIds) && sceneIds.length > 0) {
    let sum = 0;
    let hits = 0;
    for (const rawId of sceneIds) {
      const id = (rawId || "").trim();
      if (!id) continue;
      const itemRec = memoryStore.bySceneId[id];
      if (itemRec && Number.isFinite(itemRec.estimatedBytes) && itemRec.estimatedBytes > 0) {
        sum += itemRec.estimatedBytes;
        hits += 1;
      }
    }
    if (hits > 0 && sum > 0) {
      return sum;
    }
  }
  return null;
}

export function lookupSceneItemSubsetEstimate(
  taskId?: string | null,
  sceneId?: string | null,
): number | null {
  ensureHydrated();
  const cleanSceneId = (sceneId || "").trim();
  const cleanTaskId = (taskId || "").trim();
  if (cleanTaskId && cleanSceneId) {
    const rec = memoryStore.byTaskId[cleanTaskId];
    const perItem = rec?.perItemBytes?.[cleanSceneId];
    if (typeof perItem === "number" && Number.isFinite(perItem) && perItem > 0) {
      return perItem;
    }
  }
  if (cleanSceneId) {
    const rec = memoryStore.bySceneId[cleanSceneId];
    if (rec && Number.isFinite(rec.estimatedBytes) && rec.estimatedBytes > 0) {
      return rec.estimatedBytes;
    }
  }
  return null;
}
