import { invoke as tauriInvoke } from "@tauri-apps/api/core";
import { isTauriDesktopHost } from "./desktopShellBridge.ts";

export type PerFileSegmentsOption = 1 | 2 | 4 | 8;
export type TileChunkConcurrencyOption = 8 | 16 | 32 | 64;
export type SsdStagingMode = "off" | "auto" | "always";
export const MAX_DOWNLOAD_SPEED_KIB_S_LIMIT = 1_048_576;
export const SPEED_LIMIT_PRESET_KIB_S = [0, 2048, 5120, 10240, 20480] as const;

export function normalizeMaxDownloadSpeedKibS(value: unknown): number {
  const numeric = typeof value === "number" ? value : Number(value);
  if (!Number.isFinite(numeric) || numeric <= 0) {
    return 0;
  }
  return Math.min(MAX_DOWNLOAD_SPEED_KIB_S_LIMIT, Math.round(numeric));
}

export function speedKibSToMbInput(kib: number): string {
  const normalized = normalizeMaxDownloadSpeedKibS(kib);
  if (normalized === 0) return "";
  const mb = normalized / 1024;
  const digits = mb >= 1 ? 2 : mb >= 0.01 ? 3 : 4;
  return String(Number(mb.toFixed(digits)));
}

export function resolveSsdStagingMode(
  prefs: Pick<TransferEnginePrefs, "ssd_staging_enabled" | "ssd_staging_auto_slow_disk">,
): SsdStagingMode {
  if (prefs.ssd_staging_enabled) return "always";
  if (prefs.ssd_staging_auto_slow_disk) return "auto";
  return "off";
}

export function ssdStagingPatchForMode(
  mode: SsdStagingMode,
): Pick<TransferEnginePrefs, "ssd_staging_enabled" | "ssd_staging_auto_slow_disk"> {
  if (mode === "always") {
    return { ssd_staging_enabled: true, ssd_staging_auto_slow_disk: false };
  }
  if (mode === "auto") {
    return { ssd_staging_enabled: false, ssd_staging_auto_slow_disk: true };
  }
  return { ssd_staging_enabled: false, ssd_staging_auto_slow_disk: false };
}

export interface TransferEnginePrefs {
  max_concurrent_tasks: number;
  smart_concurrency: boolean;
  per_file_segments: PerFileSegmentsOption;
  tile_chunk_concurrency: TileChunkConcurrencyOption;
  stall_reconnect_enabled: boolean;
  ssd_staging_enabled: boolean;
  ssd_staging_auto_slow_disk: boolean;
  ssd_staging_dir: string | null;
  max_download_speed_kib_s: number;
}

export const TRANSFER_ENGINE_PREFS_STORAGE_KEY = "eostage.transferEnginePrefs.v1";
export const TRANSFER_ENGINE_PREFS_EVENT = "eostage:transfer-engine-prefs-changed";

export const DEFAULT_TRANSFER_ENGINE_PREFS: TransferEnginePrefs = {
  max_concurrent_tasks: 2,
  smart_concurrency: true,
  per_file_segments: 4,
  tile_chunk_concurrency: 16,
  stall_reconnect_enabled: true,
  ssd_staging_enabled: false,
  ssd_staging_auto_slow_disk: true,
  ssd_staging_dir: null,
  max_download_speed_kib_s: 0,
};

let memoryPrefs: TransferEnginePrefs = { ...DEFAULT_TRANSFER_ENGINE_PREFS };
const listeners = new Set<(prefs: TransferEnginePrefs) => void>();

function safeLocalStorage(): Storage | null {
  try {
    if (typeof window !== "undefined" && window.localStorage) {
      return window.localStorage;
    }
  } catch {
    // Ignore restricted storage contexts.
  }
  return null;
}

export function normalizeMaxConcurrentTasks(value: unknown): number {
  const numeric = typeof value === "number" ? value : Number(value);
  if (!Number.isFinite(numeric) || numeric <= 0) {
    return DEFAULT_TRANSFER_ENGINE_PREFS.max_concurrent_tasks;
  }
  return Math.max(1, Math.min(8, Math.round(numeric)));
}

export function normalizeTileChunkConcurrency(value: unknown): TileChunkConcurrencyOption {
  const numeric = typeof value === "number" ? value : Number(value);
  if (numeric === 8 || numeric === 16 || numeric === 32 || numeric === 64) {
    return numeric;
  }
  if (!Number.isFinite(numeric) || numeric <= 0) {
    return DEFAULT_TRANSFER_ENGINE_PREFS.tile_chunk_concurrency;
  }
  if (numeric <= 11) return 8;
  if (numeric <= 23) return 16;
  if (numeric <= 47) return 32;
  return 64;
}

export function normalizePerFileSegments(value: unknown): PerFileSegmentsOption {
  const numeric = typeof value === "number" ? value : Number(value);
  if (numeric === 1 || numeric === 2 || numeric === 4 || numeric === 8) {
    return numeric;
  }
  if (!Number.isFinite(numeric)) {
    return DEFAULT_TRANSFER_ENGINE_PREFS.per_file_segments;
  }
  if (numeric <= 1) return 1;
  if (numeric <= 2) return 2;
  if (numeric <= 5) return 4;
  return 8;
}

export function normalizeTransferEnginePrefs(
  raw: Partial<TransferEnginePrefs> | null | undefined,
): TransferEnginePrefs {
  const dirRaw = typeof raw?.ssd_staging_dir === "string" ? raw.ssd_staging_dir.trim() : "";
  return {
    max_concurrent_tasks: normalizeMaxConcurrentTasks(raw?.max_concurrent_tasks),
    smart_concurrency:
      typeof raw?.smart_concurrency === "boolean"
        ? raw.smart_concurrency
        : DEFAULT_TRANSFER_ENGINE_PREFS.smart_concurrency,
    per_file_segments: normalizePerFileSegments(raw?.per_file_segments),
    tile_chunk_concurrency: normalizeTileChunkConcurrency(raw?.tile_chunk_concurrency),
    stall_reconnect_enabled:
      typeof raw?.stall_reconnect_enabled === "boolean"
        ? raw.stall_reconnect_enabled
        : DEFAULT_TRANSFER_ENGINE_PREFS.stall_reconnect_enabled,
    ssd_staging_enabled:
      typeof raw?.ssd_staging_enabled === "boolean"
        ? raw.ssd_staging_enabled
        : DEFAULT_TRANSFER_ENGINE_PREFS.ssd_staging_enabled,
    ssd_staging_auto_slow_disk:
      typeof raw?.ssd_staging_auto_slow_disk === "boolean"
        ? raw.ssd_staging_auto_slow_disk
        : DEFAULT_TRANSFER_ENGINE_PREFS.ssd_staging_auto_slow_disk,
    ssd_staging_dir: dirRaw.length > 0 ? dirRaw : null,
    max_download_speed_kib_s: normalizeMaxDownloadSpeedKibS(raw?.max_download_speed_kib_s),
  };
}

function readStoredPrefs(): TransferEnginePrefs {
  const storage = safeLocalStorage();
  if (!storage) return { ...memoryPrefs };
  try {
    const raw = storage.getItem(TRANSFER_ENGINE_PREFS_STORAGE_KEY);
    if (!raw) return { ...DEFAULT_TRANSFER_ENGINE_PREFS };
    const parsed = JSON.parse(raw) as Partial<TransferEnginePrefs>;
    return normalizeTransferEnginePrefs(parsed);
  } catch {
    return { ...DEFAULT_TRANSFER_ENGINE_PREFS };
  }
}

function writeStoredPrefs(prefs: TransferEnginePrefs): void {
  memoryPrefs = prefs;
  const storage = safeLocalStorage();
  if (storage) {
    try {
      storage.setItem(TRANSFER_ENGINE_PREFS_STORAGE_KEY, JSON.stringify(prefs));
    } catch {
      // Ignore quota / privacy errors.
    }
  }
  for (const listener of listeners) {
    listener(prefs);
  }
  if (typeof window !== "undefined") {
    try {
      window.dispatchEvent(
        new CustomEvent<TransferEnginePrefs>(TRANSFER_ENGINE_PREFS_EVENT, { detail: prefs }),
      );
    } catch {
      // Ignore environments without CustomEvent.
    }
  }
}

export function getTransferEnginePrefs(): TransferEnginePrefs {
  memoryPrefs = readStoredPrefs();
  return memoryPrefs;
}

export function setTransferEnginePrefs(
  patch: Partial<TransferEnginePrefs>,
): TransferEnginePrefs {
  const current = getTransferEnginePrefs();
  const next = normalizeTransferEnginePrefs({ ...current, ...patch });
  writeStoredPrefs(next);
  return next;
}

export function subscribeTransferEnginePrefs(
  listener: (prefs: TransferEnginePrefs) => void,
): () => void {
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
  };
}

export async function loadTransferEnginePrefsFromDesktop(): Promise<TransferEnginePrefs> {
  const local = getTransferEnginePrefs();
  if (!isTauriDesktopHost()) {
    return local;
  }
  const remote = await tauriInvoke<Partial<TransferEnginePrefs>>("transfer_engine_get_prefs");
  const normalized = normalizeTransferEnginePrefs(remote);
  writeStoredPrefs(normalized);
  return normalized;
}

export async function saveTransferEnginePrefsToDesktop(
  patch: Partial<TransferEnginePrefs>,
): Promise<TransferEnginePrefs> {
  const next = setTransferEnginePrefs(patch);
  if (!isTauriDesktopHost()) {
    return next;
  }
  const saved = await tauriInvoke<Partial<TransferEnginePrefs>>("transfer_engine_save_prefs", {
    prefs: next,
  });
  const normalized = normalizeTransferEnginePrefs(saved);
  writeStoredPrefs(normalized);
  return normalized;
}
