import type { SourceDescriptor, SourceView, RowTaskState, Bbox } from "./types.ts";
import { restoreConditionsFromBatchQuery } from "./manualConditionsLogic.ts";

export interface StartupLoadDependencies {
  listSources: () => Promise<SourceDescriptor[]>;
  getSourceViews: () => Promise<SourceView[]>;
  getBatchPage: (
    batchId: string,
    revision: number,
    page?: number,
    pageSize?: number
  ) => Promise<{ batch_id?: string; total_count?: number; scenes: any[]; rows: RowTaskState[] }>;
  getPreferredActiveResourceId?: () => Promise<string | null>;
  onRestoreActiveResourceId?: (resourceId: string) => void;
  /** #22 B slice 5: the first page read at startup (its delivery state, e.g. interrupted). */
  onBatchPage?: (page: { batch_id?: string; total_count?: number; scenes: any[]; rows: RowTaskState[] }) => void;
  getWorkingAoi: () => Promise<unknown>;
  getCredentialStatus: () => Promise<unknown>;
  getSarDownloadPolicySettings: () => Promise<any>;
  getDemTaskSettings: () => Promise<any>;
  isDesktopHost: () => boolean;
  getFixtureWorkingAoi?: () => {
    bbox?: Bbox | number[];
    geometry?: Record<string, unknown>;
    label?: string;
    display_name?: string;
  } | null;
  /** 仅非桌面演示模式：集中 fixture 提供的存储根（生产实现返回 null） */
  getFixtureStorageRoot?: () => string | null;

  setAvailableSources: (sources: SourceDescriptor[]) => void;
  setSourceViews: (views: SourceView[]) => void;
  setScenes: (scenes: any[]) => void;
  setRowStates: (rows: Map<string, RowTaskState>) => void;
  setWorkingGeometry: (geom: Record<string, unknown>) => void;
  setWorkingAoiState: (bbox: Bbox) => void;
  setAoiLabel: (label: string) => void;
  setCreds: (creds: any) => void;
  setStorageRootPath: (root: string) => void;
  setSarDownloadPolicy: (policy: any) => void;
  setDemConcurrency: (concurrency: number) => void;
  setPolarizations?: (polarizations: string[]) => void;
  setBeamModes?: (beamModes: string[]) => void;
  setLookDirection?: (lookDirection: string | null) => void;
  setBurstStrategy?: (strategy: "all" | "primary") => void;
}

export function createStartupEffect(deps: StartupLoadDependencies): () => () => void {
  let startupRan = false;

  return () => {
    if (startupRan) {
      return () => {};
    }
    startupRan = true;
    let active = true;

    // 1. C1 数据源与初始批次优先立即加载
    void (async () => {
      try {
        const [sourceList, views, preferredResourceId] = await Promise.all([
          deps.listSources(),
          deps.getSourceViews(),
          deps.getPreferredActiveResourceId
            ? deps.getPreferredActiveResourceId().catch(() => null)
            : Promise.resolve(null),
        ]);
        if (!active) return;
        deps.setAvailableSources(sourceList);
        deps.setSourceViews(views);

        const defaultResourceId = "science.sentinel-1.standard.v1";
        const trimmedPreferred =
          typeof preferredResourceId === "string" ? preferredResourceId.trim() : "";
        const isKnownStaticResource =
          /^(dem\.[a-z0-9._-]+|orbit\.sentinel-1\.poeorb\.v1|atmospheric\.gacos\.ztd\.v1|auxiliary\.xyz-tiles\.v1)$/.test(
            trimmedPreferred
          );
        const isValidPreferred =
          trimmedPreferred.length > 0 &&
          (sourceList.some((s) => s.resource_id === trimmedPreferred) ||
            views.some((v) => v.resource_id === trimmedPreferred) ||
            isKnownStaticResource);
        const targetResourceId = isValidPreferred ? trimmedPreferred : defaultResourceId;
        if (targetResourceId !== defaultResourceId) {
          deps.onRestoreActiveResourceId?.(targetResourceId);
        }

        // 加载目标数据源（默认 Sentinel-1 或上次退出前选中的数据源）的初始批次
        const targetView = views.find((v) => v.resource_id === targetResourceId);
        if (targetView?.batch_id) {
          const page = await deps.getBatchPage(targetView.batch_id, targetView.revision ?? 0);
          if (!active) return;
          deps.setScenes(page.scenes);
          const rMap = new Map<string, RowTaskState>();
          for (const r of page.rows) rMap.set(r.item_id, r);
          deps.setRowStates(rMap);
          deps.onBatchPage?.(page);
          const rawQuery =
            ((page as unknown as Record<string, unknown>)?.query as
              | Record<string, unknown>
              | null
              | undefined) ??
            ((targetView as unknown as Record<string, unknown>)?.query as
              | Record<string, unknown>
              | null
              | undefined) ??
            null;
          const restoredConditions = restoreConditionsFromBatchQuery(rawQuery);
          if (restoredConditions.polarizations.length > 0) {
            deps.setPolarizations?.(restoredConditions.polarizations);
          }
          if (restoredConditions.beamModes.length > 0) {
            deps.setBeamModes?.(restoredConditions.beamModes);
          }
          if (restoredConditions.lookDirection !== null) {
            deps.setLookDirection?.(restoredConditions.lookDirection);
          }
          if (restoredConditions.burstStrategy !== null) {
            deps.setBurstStrategy?.(restoredConditions.burstStrategy);
          }
        }
      } catch (err) {
        console.warn("Load C1 sources error:", err);
      }
    })();

    // 2. 真实原生 Tauri 桌面环境下安全加载宿主持久化状态（非原生环境不执行，杜绝假成功与悬挂）
    if (deps.isDesktopHost()) {
      void (async () => {
        try {
          const aoiRes = await deps.getWorkingAoi();
          if (active && aoiRes && typeof aoiRes === "object") {
            const anyRes = aoiRes as any;
            if (anyRes.geometry && typeof anyRes.geometry === "object") {
              deps.setWorkingGeometry(anyRes.geometry as Record<string, unknown>);
            }
            if (anyRes.bbox) {
              const raw = anyRes.bbox;
              const b: Bbox = Array.isArray(raw)
                ? { west: raw[0], south: raw[1], east: raw[2], north: raw[3] }
                : raw;
              deps.setWorkingAoiState(b);
              deps.setAoiLabel(anyRes.display_name || `AOI [${b.west.toFixed(1)}, ${b.south.toFixed(1)}]`);
            }
          }
        } catch (err) {
          console.warn("Load AOI error:", err);
        }
        try {
          const c = await deps.getCredentialStatus();
          if (active && c) deps.setCreds(c);
        } catch (err) {
          console.warn("Load creds error:", err);
        }
        try {
          const policy = await deps.getSarDownloadPolicySettings();
          if (active && policy && policy.ok) {
            deps.setSarDownloadPolicy(policy);
            if (policy.default_output_root) {
              deps.setStorageRootPath(policy.default_output_root);
            }
          }
        } catch (err) {
          console.warn("Load download policy settings error:", err);
        }
        try {
          const demSettings = await deps.getDemTaskSettings();
          if (active && demSettings && demSettings.ok && Number.isInteger(demSettings.max_concurrent_tasks)) {
            deps.setDemConcurrency(demSettings.max_concurrent_tasks);
          }
        } catch (err) {
          console.warn("Load DEM settings error:", err);
        }
      })();
    } else if (typeof deps.getFixtureWorkingAoi === "function" || typeof deps.getFixtureStorageRoot === "function") {
      const fixtureRoot = typeof deps.getFixtureStorageRoot === "function" ? deps.getFixtureStorageRoot() : null;
      if (active && typeof fixtureRoot === "string" && fixtureRoot.trim()) {
        deps.setStorageRootPath(fixtureRoot.trim());
      }
    }
    if (!deps.isDesktopHost() && typeof deps.getFixtureWorkingAoi === "function") {
      const fixtureAoi = deps.getFixtureWorkingAoi();
      if (active && fixtureAoi) {
        if (fixtureAoi.geometry && typeof fixtureAoi.geometry === "object") {
          deps.setWorkingGeometry(fixtureAoi.geometry as Record<string, unknown>);
        }
        if (fixtureAoi.bbox) {
          const raw = fixtureAoi.bbox;
          const b: Bbox = Array.isArray(raw)
            ? { west: raw[0], south: raw[1], east: raw[2], north: raw[3] }
            : raw;
          deps.setWorkingAoiState(b);
          deps.setAoiLabel(
            fixtureAoi.label || fixtureAoi.display_name || `AOI [${b.west.toFixed(1)}, ${b.south.toFixed(1)}]`
          );
        }
      }
    }

    return () => {
      active = false;
      startupRan = false;
    };
  };
}
