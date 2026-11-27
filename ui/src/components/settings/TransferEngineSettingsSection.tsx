import { useEffect, useState } from "react";
import { AlertCircle, Check, FolderOpen } from "lucide-react";
import { useI18n } from "@/i18n";
import { pickDirectory } from "@/lib/bridge";
import {
  isTemporarySystemDownloadPath,
  TEMPORARY_DOWNLOAD_PATH_ERROR_ZH,
} from "@/lib/storagePathValidation";
import { emitTopToast } from "@/components/ui/TopCenterToast";
import {
  getTransferEnginePrefs,
  loadTransferEnginePrefsFromDesktop,
  MAX_DOWNLOAD_SPEED_KIB_S_LIMIT,
  resolveSsdStagingMode,
  saveTransferEnginePrefsToDesktop,
  SPEED_LIMIT_PRESET_KIB_S,
  speedKibSToMbInput,
  ssdStagingPatchForMode,
  subscribeTransferEnginePrefs,
  type PerFileSegmentsOption,
  type SsdStagingMode,
  type TileChunkConcurrencyOption,
  type TransferEnginePrefs,
} from "@/lib/transferEnginePrefs";

const SEGMENT_OPTIONS: readonly PerFileSegmentsOption[] = [1, 2, 4, 8];
const TASK_CONCURRENCY_OPTIONS: readonly number[] = [1, 2, 3, 4, 6, 8];
const TILE_CHUNK_OPTIONS: readonly TileChunkConcurrencyOption[] = [8, 16, 32, 64];
const SSD_MODE_OPTIONS: readonly SsdStagingMode[] = ["off", "auto", "always"];

interface EngineCopy {
  sectionTitle: string;
  sectionDesc: string;
  tier1Title: string;
  tier1Desc: string;
  tier1Suffix: string;
  smartConcurrencyTitle: string;
  smartConcurrencyDesc: string;
  segmentsTitle: string;
  segmentsDesc: string;
  segmentsSuffix: string;
  tileChunkTitle: string;
  tileChunkDesc: string;
  tileChunkSuffix: string;
  speedLimitTitle: string;
  speedLimitDesc: string;
  speedLimitUnlimited: string;
  speedLimitPlaceholder: string;
  speedLimitUnit: string;
  speedLimitApply: string;
  speedLimitUpdated: (kib: number) => string;
  invalidSpeedLimit: string;
  stallReconnectTitle: string;
  stallReconnectDesc: string;
  ssdModeTitle: string;
  ssdModeDesc: string;
  ssdModeLabels: Record<SsdStagingMode, string>;
  stagingPathLabel: string;
  stagingPathPlaceholder: string;
  savePath: string;
  clearPath: string;
  saved: string;
  syncFailed: string;
  pickDirFailed: string;
  globalTasksUpdated: (count: number) => string;
  smartConcurrencyUpdated: (enabled: boolean) => string;
  segmentsUpdated: (count: number) => string;
  tileConcurrencyUpdated: (count: number) => string;
  stallReconnectUpdated: (enabled: boolean) => string;
  ssdModeUpdated: (mode: SsdStagingMode) => string;
  stagingPathSaved: string;
  stagingPathCleared: string;
}

function formatSpeedLabel(kib: number, locale: string): string {
  if (kib <= 0) {
    if (locale.startsWith("en")) return "Unlimited";
    if (locale.startsWith("ja")) return "無制限";
    return "不限速";
  }
  if (kib >= 1024) {
    const mb = Number((kib / 1024).toFixed(2));
    return `${mb} MB/s`;
  }
  return `${kib} KiB/s`;
}

function getEngineCopy(locale: string): EngineCopy {
  if (locale.startsWith("en")) {
    return {
      sectionTitle: "3-Tier Transfer Concurrency & SSD Staging",
      sectionDesc:
        "Unified concurrency control across simultaneous tasks, multi-segment large files, satellite tile / GEE chunk requests, bandwidth limits, and SSD staging.",
      tier1Title: "Tier 1 · Simultaneous Download Tasks",
      tier1Desc:
        "Maximum concurrent item downloads per task (SAR, optical, orbit, 1–8) and simultaneous DEM queue limit; independent tasks for different folders still run independently.",
      tier1Suffix: "tasks",
      smartConcurrencyTitle: "Smart Concurrency Scheduling",
      smartConcurrencyDesc:
        "Runs up to your Tier 1 cap when disk headroom and transfers are healthy, and automatically throttles down under low disk space or repeated stalls.",
      segmentsTitle: "Tier 2 · Per-File Multi-Segment Connections",
      segmentsDesc:
        "HTTP Range workers per large file >= 16 MiB with dynamic tail splitting and adaptive per-host concurrency throttling (ISRO Bhoonidhi capped <= 4).",
      segmentsSuffix: "conns",
      tileChunkTitle: "Tier 3 · Satellite Tile & GEE Chunk Requests",
      tileChunkDesc:
        "Parallel worker requests for XYZ satellite tile stitching (Terrain / Imagery) and GEE gridded GeoTIFF chunks.",
      tileChunkSuffix: "workers",
      speedLimitTitle: "Global File Download Speed Limit",
      speedLimitDesc:
        "Live bandwidth cap for the main file transfer engine (SAR, optical, orbit). Set to 0 or leave blank for unlimited.",
      speedLimitUnlimited: "Unlimited",
      speedLimitPlaceholder: "0 = Unlimited",
      speedLimitUnit: "MB/s",
      speedLimitApply: "Apply",
      speedLimitUpdated: (kib) =>
        kib > 0
          ? `Global download speed limit updated to ${formatSpeedLabel(kib, "en")}`
          : "Global download speed limit removed (unlimited)",
      invalidSpeedLimit: "Enter a non-negative number in MB/s (blank or 0 for unlimited)",
      stallReconnectTitle: "Low-Speed Stall Auto-Reconnect",
      stallReconnectDesc:
        "Abort and resume from exact byte offset with overlap verification when throughput stays below 32 KiB/s for 15s (up to 12 reconnects).",
      ssdModeTitle: "SSD High-Speed Staging",
      ssdModeDesc:
        "Receive and SHA-256 verify .part files on fast SSD storage before sequentially promoting to the target drive.",
      ssdModeLabels: {
        off: "Off",
        auto: "Auto on Slow Drives",
        always: "Always On",
      },
      stagingPathLabel: "SSD Staging Directory",
      stagingPathPlaceholder:
        "Leave empty for default <app_data>/ssd_staging (kept separate from disposable cache cleanup)",
      savePath: "Save",
      clearPath: "Clear",
      saved: "Saved",
      syncFailed: "Failed to sync transfer engine settings",
      pickDirFailed: "Failed to select directory",
      globalTasksUpdated: (count) => `Simultaneous task cap updated to ${count}`,
      smartConcurrencyUpdated: (enabled) =>
        enabled ? "Smart concurrency scheduling enabled" : "Smart concurrency scheduling disabled",
      segmentsUpdated: (count) => `Per-file segment connections updated to ${count}`,
      tileConcurrencyUpdated: (count) => `Tile & GEE chunk concurrency updated to ${count}`,
      stallReconnectUpdated: (enabled) =>
        enabled ? "Low-speed stall auto-reconnect enabled" : "Low-speed stall auto-reconnect disabled",
      ssdModeUpdated: (mode) =>
        mode === "always"
          ? "SSD staging set to Always On"
          : mode === "auto"
            ? "SSD staging set to Auto on Slow Drives"
            : "SSD staging disabled",
      stagingPathSaved: "SSD staging directory saved",
      stagingPathCleared: "SSD staging directory reset to default",
    };
  }
  if (locale.startsWith("ja")) {
    return {
      sectionTitle: "3階層ダウンロード並列制御と SSD ステージング",
      sectionDesc:
        "同時タスク数、単一ファイル HTTP Range 分割、衛星タイル / GEE 分割リクエスト、速度上限、SSD ステージングを統合設定します。",
      tier1Title: "第1階層 · 同時ダウンロードタスク上限",
      tier1Desc:
        "単一タスク内の同時ダウンロード項目数（SAR・光学・精密軌道、1〜8）および同時 DEM キュー上限を設定します（保存先が異なる独立タスクは並行動作）。",
      tier1Suffix: "件",
      smartConcurrencyTitle: "スマート並列制御",
      smartConcurrencyDesc:
        "通常の空き容量・通信状態では第1階層上限で動作し、ディスク不足や停滞再接続の検知時のみ上限内で自動抑制します。",
      segmentsTitle: "第2階層 · 単一ファイル Range 接続数",
      segmentsDesc:
        "16 MiB 以上のファイルに対する同時 Range ワーカー数（長尾チャンク動的分割・ホスト別同時数学習に対応、ISRO Bhoonidhi 等は 4 本以下に抑制）。",
      segmentsSuffix: "本",
      tileChunkTitle: "第3階層 · 衛星タイル / GEE 分割並列数",
      tileChunkDesc:
        "XYZ 衛星タイル結合および GEE クラウド GeoTIFF 分割取得の同時 HTTP リクエスト数。",
      tileChunkSuffix: "並列",
      speedLimitTitle: "全体ファイルダウンロード速度上限",
      speedLimitDesc:
        "メイン転送エンジン（SAR・光学・精密軌道など）の帯域幅をリアルタイムに制限します（0 または空欄で無制限）。",
      speedLimitUnlimited: "無制限",
      speedLimitPlaceholder: "0 で無制限",
      speedLimitUnit: "MB/s",
      speedLimitApply: "適用",
      speedLimitUpdated: (kib) =>
        kib > 0
          ? `全体ダウンロード速度上限を ${formatSpeedLabel(kib, "ja")} に設定しました`
          : "ダウンロード速度上限を解除しました（無制限）",
      invalidSpeedLimit: "0 以上の数値（MB/s）を入力してください（空欄または 0 で無制限）",
      stallReconnectTitle: "低速停滞時の自動再接続",
      stallReconnectDesc:
        "32 KiB/s 未満が 15 秒続いた場合、重複検証付きで正確なバイト位置から再接続します（最大 12 回）。",
      ssdModeTitle: "SSD 高速ステージング",
      ssdModeDesc:
        "高速な SSD で .part を受信・SHA-256 検証してから対象ドライブへ 16 MiB バッファで順次昇格させます。",
      ssdModeLabels: {
        off: "オフ",
        auto: "低速ディスク時は自動",
        always: "常時有効",
      },
      stagingPathLabel: "SSD ステージングディレクトリ",
      stagingPathPlaceholder:
        "空欄で規定の ssd_staging を利用（使い捨てキャッシュ削除の対象外）",
      savePath: "保存",
      clearPath: "クリア",
      saved: "保存済み",
      syncFailed: "転送エンジン設定の同期に失敗しました",
      pickDirFailed: "フォルダの選択に失敗しました",
      globalTasksUpdated: (count) => `同時ダウンロードタスク上限を ${count} 件に更新しました`,
      smartConcurrencyUpdated: (enabled) =>
        enabled ? "スマート並列制御を有効にしました" : "スマート並列制御を無効にしました",
      segmentsUpdated: (count) => `単一ファイル Range 接続数を ${count} 本に更新しました`,
      tileConcurrencyUpdated: (count) => `タイル / GEE 分割並列数を ${count} に更新しました`,
      stallReconnectUpdated: (enabled) =>
        enabled ? "低速停滞時の自動再接続を有効にしました" : "低速停滞時の自動再接続を無効にしました",
      ssdModeUpdated: (mode) =>
        mode === "always"
          ? "SSD ステージングを「常時有効」に切り替えました"
          : mode === "auto"
            ? "SSD ステージングを「低速ディスク時は自動」に切り替えました"
            : "SSD ステージングを無効にしました",
      stagingPathSaved: "SSD ステージングディレクトリを保存しました",
      stagingPathCleared: "SSD ステージングディレクトリを規定値に戻しました",
    };
  }
  return {
    sectionTitle: "全局并发与智能传输控制",
    sectionDesc: "",
    tier1Title: "同时下载任务数",
    tier1Desc:
      "控制单个 SAR、光学、精密轨道任务内的并发下载数及同时运行的 DEM 任务上限（1–8）；不同保存目录的任务互不阻塞。",
    tier1Suffix: "任务",
    smartConcurrencyTitle: "智能并发调度",
    smartConcurrencyDesc:
      "网络与磁盘空间充足时按上限全速调度；仅在磁盘余量不足或连续低速卡顿时自动降速保护。",
    segmentsTitle: "单文件分段线程数",
    segmentsDesc:
      "针对 >= 16 MiB 影像文件启用 HTTP Range 并发切片，自动切分长尾分片并学习最优并发上限（Bhoonidhi 自动限制 <= 4 连接）。",
    segmentsSuffix: "连接",
    tileChunkTitle: "瓦片与云端分块并发数",
    tileChunkDesc:
      "控制全球高清卫星瓦片下载拼接及 Google Earth Engine 云端栅格分块的并发请求数。",
    tileChunkSuffix: "并发",
    speedLimitTitle: "全局文件下载限速",
    speedLimitDesc:
      "实时限制主下载引擎（SAR、光学、精密轨道等文件传输）的总带宽上限；设为 0 或留空为不限速，修改后立即生效。",
    speedLimitUnlimited: "不限速",
    speedLimitPlaceholder: "0 或留空为不限速",
    speedLimitUnit: "MB/s",
    speedLimitApply: "应用",
    speedLimitUpdated: (kib) =>
      kib > 0
        ? `全局文件下载限速已设为 ${formatSpeedLabel(kib, "zh")}`
        : "全局文件下载限速已关闭（不限速）",
    invalidSpeedLimit: "请输入不小于 0 的 MB/s 速率值（留空或 0 表示不限速）",
    stallReconnectTitle: "低速卡顿自动重连",
    stallReconnectDesc:
      "连续 15 秒速率低于 32 KiB/s 时自动断开并从精确字节偏移带重叠校验重连（最多 12 次有效重连）。",
    ssdModeTitle: "SSD 高速暂存策略",
    ssdModeDesc:
      "先在 SSD 目录接收并通过 SHA-256 校验，再使用 16 MiB 顺序缓冲搬运至目标目录。",
    ssdModeLabels: {
      off: "关闭",
      auto: "慢速盘自动启用",
      always: "始终启用",
    },
    stagingPathLabel: "SSD 暂存目录路径",
    stagingPathPlaceholder:
      "留空使用默认目录（应用数据目录下的 ssd_staging，独立于可清理缓存），或指定本地高速 SSD 路径",
    savePath: "保存",
    clearPath: "清空",
    saved: "已保存",
    syncFailed: "同步传输引擎配置失败",
    pickDirFailed: "选择目录失败",
    globalTasksUpdated: (count) => `全局同时下载任务上限已更新为 ${count} 个`,
    smartConcurrencyUpdated: (enabled) =>
      enabled ? "智能并发调度已开启" : "智能并发调度已关闭",
    segmentsUpdated: (count) => `单文件分段线程数已更新为 ${count} 连接`,
    tileConcurrencyUpdated: (count) => `瓦片与 GEE 并发请求数已更新为 ${count} 并发`,
    stallReconnectUpdated: (enabled) =>
      enabled ? "低速卡顿自动重连已开启" : "低速卡顿自动重连已关闭",
    ssdModeUpdated: (mode) =>
      mode === "always"
        ? "SSD 暂存策略已设为始终启用"
        : mode === "auto"
          ? "SSD 暂存策略已设为慢速盘自动启用"
          : "SSD 暂存已关闭",
    stagingPathSaved: "SSD 暂存目录已更新",
    stagingPathCleared: "SSD 暂存目录已恢复默认目录",
  };
}

export interface TransferEngineSettingsSectionProps {
  onSyncDemConcurrency?: (limit: number) => void | Promise<boolean | void>;
}

export function TransferEngineSettingsSection({
  onSyncDemConcurrency,
}: TransferEngineSettingsSectionProps = {}) {
  const { locale } = useI18n();
  const copy = getEngineCopy(locale);

  const [prefs, setPrefs] = useState<TransferEnginePrefs>(() => getTransferEnginePrefs());
  const [dirInput, setDirInput] = useState<string>(() => getTransferEnginePrefs().ssd_staging_dir ?? "");
  const [speedInput, setSpeedInput] = useState<string>(() =>
    speedKibSToMbInput(getTransferEnginePrefs().max_download_speed_kib_s),
  );
  const [saving, setSaving] = useState(false);
  const [savedFlag, setSavedFlag] = useState(false);
  const [errorMessage, setErrorMessage] = useState<string | null>(null);

  useEffect(() => {
    const unsubscribe = subscribeTransferEnginePrefs((next) => {
      setPrefs(next);
      setSpeedInput(speedKibSToMbInput(next.max_download_speed_kib_s));
    });
    let cancelled = false;
    void loadTransferEnginePrefsFromDesktop()
      .then((loaded) => {
        if (cancelled) return;
        setPrefs(loaded);
        setDirInput(loaded.ssd_staging_dir ?? "");
        setSpeedInput(speedKibSToMbInput(loaded.max_download_speed_kib_s));
      })
      .catch(() => {
        // Keep localStorage preference snapshot when backend command is not yet available.
      });
    return () => {
      cancelled = true;
      unsubscribe();
    };
  }, []);

  const commitPatch = async (
    patch: Partial<TransferEnginePrefs>,
    markSaved = false,
    toastSuccessText?: string,
  ) => {
    if (
      typeof patch.ssd_staging_dir === "string" &&
      isTemporarySystemDownloadPath(patch.ssd_staging_dir)
    ) {
      setErrorMessage(TEMPORARY_DOWNLOAD_PATH_ERROR_ZH);
      emitTopToast({ text: TEMPORARY_DOWNLOAD_PATH_ERROR_ZH, tone: "danger" });
      return;
    }
    setSaving(true);
    setErrorMessage(null);
    try {
      const saved = await saveTransferEnginePrefsToDesktop(patch);
      setPrefs(saved);
      if ("ssd_staging_dir" in patch) {
        setDirInput(saved.ssd_staging_dir ?? "");
      }
      if ("max_download_speed_kib_s" in patch) {
        setSpeedInput(speedKibSToMbInput(saved.max_download_speed_kib_s));
      }
      if (markSaved) {
        setSavedFlag(true);
        window.setTimeout(() => setSavedFlag(false), 1800);
      }
      if (toastSuccessText) {
        emitTopToast({ text: toastSuccessText, tone: "success" });
      }
    } catch (err: unknown) {
      const detail = err instanceof Error ? err.message : String(err ?? "");
      const msg = detail ? `${copy.syncFailed}: ${detail}` : copy.syncFailed;
      setErrorMessage(msg);
      emitTopToast({ text: msg, tone: "danger" });
    } finally {
      setSaving(false);
    }
  };

  const commitSpeedInput = () => {
    const raw = speedInput.trim();
    if (raw === "") {
      void commitPatch({ max_download_speed_kib_s: 0 }, false, copy.speedLimitUpdated(0));
      return;
    }
    const mbVal = Number(raw);
    if (!Number.isFinite(mbVal) || mbVal < 0) {
      setErrorMessage(copy.invalidSpeedLimit);
      emitTopToast({ text: copy.invalidSpeedLimit, tone: "danger" });
      return;
    }
    if (mbVal === 0) {
      void commitPatch({ max_download_speed_kib_s: 0 }, false, copy.speedLimitUpdated(0));
      return;
    }
    const nextKib = Math.max(
      1,
      Math.min(MAX_DOWNLOAD_SPEED_KIB_S_LIMIT, Math.round(mbVal * 1024)),
    );
    void commitPatch(
      { max_download_speed_kib_s: nextKib },
      false,
      copy.speedLimitUpdated(nextKib),
    );
  };

  const commitTier1Tasks = async (count: number) => {
    setSaving(true);
    setErrorMessage(null);
    try {
      const saved = await saveTransferEnginePrefsToDesktop({ max_concurrent_tasks: count });
      setPrefs(saved);
      if (onSyncDemConcurrency) {
        const syncResult = await onSyncDemConcurrency(count);
        if (syncResult === false) {
          setErrorMessage(copy.syncFailed);
          emitTopToast({ text: copy.syncFailed, tone: "danger" });
          return;
        }
      }
      emitTopToast({ text: copy.globalTasksUpdated(count), tone: "success" });
    } catch (err: unknown) {
      const detail = err instanceof Error ? err.message : String(err ?? "");
      const msg = detail ? `${copy.syncFailed}: ${detail}` : copy.syncFailed;
      setErrorMessage(msg);
      emitTopToast({ text: msg, tone: "danger" });
    } finally {
      setSaving(false);
    }
  };

  const trimmedInput = dirInput.trim();
  const normalizedCurrentDir = (prefs.ssd_staging_dir ?? "").trim();
  const pathDirty = trimmedInput !== normalizedCurrentDir;
  const ssdMode = resolveSsdStagingMode(prefs);

  return (
    <div className="eo-settings-section" data-testid="transfer-engine-settings-section">
      <div className="eo-settings-card">
        <div className="eo-settings-card-content">
          {/* Tier 1. Global Maximum Concurrent Tasks (unified SAR / Optical / DEM / Orbit) */}
          <div className="eo-settings-card-row">
            <div className="flex flex-wrap items-center justify-between gap-4">
              <div className="flex min-w-0 flex-1 flex-col gap-0.5">
                <h4 className="text-[13px] font-normal text-sp-text">{copy.tier1Title}</h4>
                <p className="text-[12.5px] text-sp-text-secondary">{copy.tier1Desc}</p>
              </div>
              <div
                className="eo-segmented-track shrink-0"
                role="group"
                data-testid="transfer-tier1-max-tasks"
                aria-label={copy.tier1Title}
              >
                {TASK_CONCURRENCY_OPTIONS.map((count) => {
                  const active = prefs.max_concurrent_tasks === count;
                  return (
                    <button
                      key={count}
                      type="button"
                      data-testid={`transfer-tier1-tasks-${count}`}
                      aria-pressed={active}
                      data-active={active}
                      disabled={saving}
                      onClick={() => {
                        void commitTier1Tasks(count);
                      }}
                      className="eo-segmented-pill h-[26px] px-2.5 text-[12px] cursor-pointer disabled:opacity-50"
                    >
                      <span>
                        {count} {copy.tier1Suffix}
                      </span>
                    </button>
                  );
                })}
              </div>
            </div>
          </div>

          {/* 1b. Smart Concurrency */}
          <div className="eo-settings-card-row border-t border-sp-divider">
            <div className="flex items-center justify-between gap-4">
              <div className="flex min-w-0 flex-1 flex-col gap-0.5">
                <h4 className="text-[13px] font-normal text-sp-text">
                  {copy.smartConcurrencyTitle}
                </h4>
                <p className="text-[12.5px] text-sp-text-secondary">
                  {copy.smartConcurrencyDesc}
                </p>
              </div>
              <button
                type="button"
                role="switch"
                aria-checked={prefs.smart_concurrency}
                aria-label={copy.smartConcurrencyTitle}
                data-testid="transfer-smart-concurrency-switch"
                data-state={prefs.smart_concurrency ? "checked" : "unchecked"}
                disabled={saving}
                onClick={() => {
                  const next = !prefs.smart_concurrency;
                  void commitPatch(
                    { smart_concurrency: next },
                    false,
                    copy.smartConcurrencyUpdated(next),
                  );
                }}
                className="eo-settings-switch shrink-0 cursor-pointer disabled:opacity-50"
              >
                <span className="eo-settings-switch-thumb" />
              </button>
            </div>
          </div>

          {/* Tier 2. Multi-Segment Range Connections */}
          <div className="eo-settings-card-row border-t border-sp-divider">
            <div className="flex flex-wrap items-center justify-between gap-4">
              <div className="flex min-w-0 flex-1 flex-col gap-0.5">
                <h4 className="text-[13px] font-normal text-sp-text">{copy.segmentsTitle}</h4>
                <p className="text-[12.5px] text-sp-text-secondary">{copy.segmentsDesc}</p>
              </div>
              <div
                className="eo-segmented-track shrink-0"
                role="group"
                data-testid="transfer-tier2-segments"
                aria-label={copy.segmentsTitle}
              >
                {SEGMENT_OPTIONS.map((count) => {
                  const active = prefs.per_file_segments === count;
                  return (
                    <button
                      key={count}
                      type="button"
                      data-testid={`transfer-segments-${count}`}
                      aria-pressed={active}
                      data-active={active}
                      disabled={saving}
                      onClick={() =>
                        void commitPatch(
                          { per_file_segments: count },
                          false,
                          copy.segmentsUpdated(count),
                        )
                      }
                      className="eo-segmented-pill h-[26px] px-2.5 text-[12px] cursor-pointer disabled:opacity-50"
                    >
                      <span>
                        {count} {copy.segmentsSuffix}
                      </span>
                    </button>
                  );
                })}
              </div>
            </div>
          </div>

          {/* Tier 3. Satellite Tile & GEE Chunked Concurrency */}
          <div className="eo-settings-card-row border-t border-sp-divider">
            <div className="flex flex-wrap items-center justify-between gap-4">
              <div className="flex min-w-0 flex-1 flex-col gap-0.5">
                <h4 className="text-[13px] font-normal text-sp-text">{copy.tileChunkTitle}</h4>
                <p className="text-[12.5px] text-sp-text-secondary">{copy.tileChunkDesc}</p>
              </div>
              <div
                className="eo-segmented-track shrink-0"
                role="group"
                data-testid="transfer-tier3-tile-concurrency"
                aria-label={copy.tileChunkTitle}
              >
                {TILE_CHUNK_OPTIONS.map((count) => {
                  const active = prefs.tile_chunk_concurrency === count;
                  return (
                    <button
                      key={count}
                      type="button"
                      data-testid={`transfer-tile-concurrency-${count}`}
                      aria-pressed={active}
                      data-active={active}
                      disabled={saving}
                      onClick={() =>
                        void commitPatch(
                          { tile_chunk_concurrency: count },
                          false,
                          copy.tileConcurrencyUpdated(count),
                        )
                      }
                      className="eo-segmented-pill h-[26px] px-2.5 text-[12px] cursor-pointer disabled:opacity-50"
                    >
                      <span>
                        {count} {copy.tileChunkSuffix}
                      </span>
                    </button>
                  );
                })}
              </div>
            </div>
          </div>

          {/* 2b. Live-Adjustable Global File Download Speed Limit */}
          <div className="eo-settings-card-row border-t border-sp-divider" data-testid="transfer-speed-limit-row">
            <div className="flex flex-wrap items-center justify-between gap-4">
              <div className="flex min-w-0 flex-1 flex-col gap-0.5">
                <h4 className="text-[13px] font-normal text-sp-text">{copy.speedLimitTitle}</h4>
                <p className="text-[12.5px] text-sp-text-secondary">{copy.speedLimitDesc}</p>
              </div>
              <div
                className="eo-segmented-track shrink-0"
                role="group"
                data-testid="transfer-speed-limit-presets"
                aria-label={copy.speedLimitTitle}
              >
                {SPEED_LIMIT_PRESET_KIB_S.map((kib) => {
                  const active = prefs.max_download_speed_kib_s === kib;
                  return (
                    <button
                      key={kib}
                      type="button"
                      data-testid={`transfer-speed-limit-${kib}`}
                      aria-pressed={active}
                      data-active={active}
                      disabled={saving}
                      onClick={() =>
                        void commitPatch(
                          { max_download_speed_kib_s: kib },
                          false,
                          copy.speedLimitUpdated(kib),
                        )
                      }
                      className="eo-segmented-pill h-[26px] px-2.5 text-[12px] cursor-pointer disabled:opacity-50 whitespace-nowrap"
                    >
                      <span>{kib === 0 ? copy.speedLimitUnlimited : formatSpeedLabel(kib, locale)}</span>
                    </button>
                  );
                })}
              </div>
            </div>
            <div className="mt-2.5 flex flex-wrap items-center gap-2">
              <input
                type="number"
                min="0"
                max="1024"
                step="any"
                value={speedInput}
                onChange={(e) => {
                  setSpeedInput(e.target.value);
                  setErrorMessage(null);
                }}
                onKeyDown={(e) => {
                  if (e.key === "Enter") {
                    e.preventDefault();
                    commitSpeedInput();
                  }
                }}
                disabled={saving}
                placeholder={copy.speedLimitPlaceholder}
                aria-label={copy.speedLimitTitle}
                data-testid="transfer-speed-limit-input"
                className="eo-settings-input w-40 font-mono"
              />
              <span className="text-[12px] text-sp-text-secondary">{copy.speedLimitUnit}</span>
              <button
                type="button"
                data-testid="transfer-speed-limit-apply"
                disabled={saving}
                onClick={commitSpeedInput}
                className="eo-settings-btn-secondary h-8 shrink-0 cursor-pointer disabled:opacity-50 whitespace-nowrap"
              >
                <span>{copy.speedLimitApply}</span>
              </button>
            </div>
          </div>

          {/* 3. Low-Speed Stall Auto-Reconnect */}
          <div className="eo-settings-card-row border-t border-sp-divider">
            <div className="flex items-center justify-between gap-4">
              <div className="flex min-w-0 flex-1 flex-col gap-0.5">
                <h4 className="text-[13px] font-normal text-sp-text">
                  {copy.stallReconnectTitle}
                </h4>
                <p className="text-[12.5px] text-sp-text-secondary">
                  {copy.stallReconnectDesc}
                </p>
              </div>
              <button
                type="button"
                role="switch"
                aria-checked={prefs.stall_reconnect_enabled}
                aria-label={copy.stallReconnectTitle}
                data-testid="transfer-stall-reconnect-switch"
                data-state={prefs.stall_reconnect_enabled ? "checked" : "unchecked"}
                disabled={saving}
                onClick={() => {
                  const next = !prefs.stall_reconnect_enabled;
                  void commitPatch(
                    { stall_reconnect_enabled: next },
                    false,
                    copy.stallReconnectUpdated(next),
                  );
                }}
                className="eo-settings-switch shrink-0 cursor-pointer disabled:opacity-50"
              >
                <span className="eo-settings-switch-thumb" />
              </button>
            </div>
          </div>

          {/* 4. Unified 3-Mode SSD Staging Control + Staging Directory */}
          <div className="eo-settings-card-row border-t border-sp-divider">
            <div className="flex flex-wrap items-center justify-between gap-4">
              <div className="flex min-w-0 flex-1 flex-col gap-0.5">
                <h4 className="text-[13px] font-normal text-sp-text">{copy.ssdModeTitle}</h4>
                <p className="text-[12.5px] text-sp-text-secondary">{copy.ssdModeDesc}</p>
              </div>
              <div
                className="eo-segmented-track shrink-0"
                role="group"
                data-testid="transfer-ssd-mode-group"
                aria-label={copy.ssdModeTitle}
              >
                {SSD_MODE_OPTIONS.map((mode) => {
                  const active = ssdMode === mode;
                  return (
                    <button
                      key={mode}
                      type="button"
                      data-testid={`transfer-ssd-mode-${mode}`}
                      aria-pressed={active}
                      data-active={active}
                      disabled={saving}
                      onClick={() =>
                        void commitPatch(
                          ssdStagingPatchForMode(mode),
                          false,
                          copy.ssdModeUpdated(mode),
                        )
                      }
                      className="eo-segmented-pill h-[26px] px-2.5 text-[12px] cursor-pointer disabled:opacity-50 whitespace-nowrap"
                    >
                      <span>{copy.ssdModeLabels[mode]}</span>
                    </button>
                  );
                })}
              </div>
            </div>

            {ssdMode !== "off" && (
              <div className="mt-3 flex flex-col gap-2" data-testid="transfer-ssd-staging-dir-row">
                <div className="text-[12.5px] text-sp-text-secondary">{copy.stagingPathLabel}</div>
                <div className="flex items-center gap-2">
                  <input
                    type="text"
                    value={dirInput}
                    onChange={(e) => {
                      setDirInput(e.target.value);
                      setErrorMessage(null);
                      setSavedFlag(false);
                    }}
                    disabled={saving}
                    placeholder={copy.stagingPathPlaceholder}
                    data-testid="transfer-ssd-staging-dir-input"
                    className="eo-settings-input flex-1 font-mono"
                  />
                  <button
                    type="button"
                    data-testid="transfer-ssd-staging-dir-browse"
                    disabled={saving}
                    onClick={async () => {
                      try {
                        const picked = await pickDirectory(
                          locale.startsWith("en")
                            ? "Choose SSD Staging Directory"
                            : locale.startsWith("ja")
                              ? "SSD ステージングフォルダを選択"
                              : "选择 SSD 高速暂存文件夹",
                        );
                        if (picked?.ok && picked.path?.trim()) {
                          const nextPath = picked.path.trim();
                          if (isTemporarySystemDownloadPath(nextPath)) {
                            setErrorMessage(TEMPORARY_DOWNLOAD_PATH_ERROR_ZH);
                            emitTopToast({ text: TEMPORARY_DOWNLOAD_PATH_ERROR_ZH, tone: "danger" });
                            return;
                          }
                          setDirInput(nextPath);
                          setErrorMessage(null);
                          setSavedFlag(false);
                        }
                      } catch (err: unknown) {
                        const detail = err instanceof Error ? err.message : String(err ?? "");
                        const msg = detail || copy.pickDirFailed;
                        setErrorMessage(msg);
                        emitTopToast({ text: msg, tone: "danger" });
                      }
                    }}
                    className="eo-settings-btn-secondary h-8 shrink-0 inline-flex items-center gap-1 disabled:opacity-50 cursor-pointer whitespace-nowrap"
                  >
                    <FolderOpen className="h-3.5 w-3.5" />
                    <span>{locale.startsWith("en") ? "Browse" : locale.startsWith("ja") ? "選択" : "选择目录"}</span>
                  </button>
                  <button
                    type="button"
                    data-testid="transfer-ssd-staging-dir-save"
                    disabled={saving || !pathDirty}
                    onClick={() =>
                      void commitPatch(
                        { ssd_staging_dir: trimmedInput.length > 0 ? trimmedInput : null },
                        true,
                        copy.stagingPathSaved,
                      )
                    }
                    className="eo-settings-btn-primary h-8 shrink-0 disabled:opacity-50"
                  >
                    {savedFlag ? (
                      <>
                        <Check className="h-3.5 w-3.5 text-sp-status-success" />
                        <span>{copy.saved}</span>
                      </>
                    ) : (
                      <span>{copy.savePath}</span>
                    )}
                  </button>
                  {prefs.ssd_staging_dir && (
                    <button
                      type="button"
                      data-testid="transfer-ssd-staging-dir-clear"
                      disabled={saving}
                      onClick={() => {
                        setDirInput("");
                        void commitPatch(
                          { ssd_staging_dir: null },
                          true,
                          copy.stagingPathCleared,
                        );
                      }}
                      className="eo-settings-btn-secondary h-8 shrink-0 disabled:opacity-50"
                    >
                      {copy.clearPath}
                    </button>
                  )}
                </div>
              </div>
            )}
          </div>

          {errorMessage && (
            <div className="mx-3 mb-3 flex items-center gap-2 rounded-sp-sm border border-sp-danger-text/20 bg-sp-danger-bg/40 p-3 text-xs text-sp-danger-text">
              <AlertCircle className="h-4 w-4 shrink-0" />
              <span>{errorMessage}</span>
            </div>
          )}
        </div>
      </div>
    </div>
  );
}
