import { useState } from "react";
import { CheckCircle2, FileUp, Loader2, MapPinned, X } from "lucide-react";

import type { AoiPreviewOk } from "@/lib/bridge";
import {
  combineNisarSubsetBoundaryFeatures,
} from "@/lib/nisarSubsetWorkspace";
import {
  validateGacosRequestDraft,
  type GacosAoiDraft,
  type GacosOutputFormat,
  type GacosRequestDraft,
  type GacosRequestMode,
  type GacosSentinelBatchReference,
} from "@/lib/gacosRequestDraft";
import { Button } from "@/components/ui/button";
import { Textarea } from "@/components/ui/textarea";
import { cn } from "@/lib/utils";

export type GacosRequestInputsProps = {
  value: GacosRequestDraft;
  onChange: (draft: GacosRequestDraft) => void;
  busy: boolean;
  latestSentinelBatch: GacosSentinelBatchReference | null;
  onImportCurrentAoi: () => Promise<{ name: string; geometry: Record<string, unknown> }>;
  onImportAoiFile: () => Promise<AoiPreviewOk | null>;
};

const HOURS = Array.from({ length: 24 }, (_, value) => value);
const MINUTES = Array.from({ length: 60 }, (_, value) => value);

function displayTimePart(value: string, maximum: number): string {
  const text = value.trim();
  if (!text) return "未选择";
  const number = Number(text);
  return Number.isInteger(number) && number >= 0 && number <= maximum
    ? String(number).padStart(2, "0")
    : "未选择";
}

function isObject(value: unknown): value is Record<string, unknown> {
  return value !== null && typeof value === "object" && !Array.isArray(value);
}

function operationError(value: unknown): string {
  return value instanceof Error && value.message.trim()
    ? value.message
    : "导入失败，请重试。";
}

function featureLabel(feature: AoiPreviewOk["features"][number]): string {
  return feature.name.trim() || `要素 ${feature.index}`;
}

function clonedDraft(value: GacosRequestDraft): GacosRequestDraft {
  return structuredClone(value);
}

/**
 * Controlled GACOS request inputs. Only successful user actions call
 * `onChange`; async file selection is kept as a short-lived local preview.
 */
export function GacosRequestInputs({
  value,
  onChange,
  busy,
  latestSentinelBatch,
  onImportCurrentAoi,
  onImportAoiFile,
}: GacosRequestInputsProps) {
  const [importBusy, setImportBusy] = useState<"current" | "file" | null>(null);
  const [aoiPreview, setAoiPreview] = useState<AoiPreviewOk | null>(null);
  const [selectedFeatureIds, setSelectedFeatureIds] = useState<Set<string>>(() => new Set());
  const [operationErrorText, setOperationErrorText] = useState<string | null>(null);
  const validation = validateGacosRequestDraft(value);
  const sourceBatch = value.sourceBatch;
  const selectedFeatureCount = selectedFeatureIds.size;

  function updateDraft(patch: Partial<GacosRequestDraft>): void {
    const next = clonedDraft(value);
    Object.assign(next, structuredClone(patch));
    setOperationErrorText(null);
    onChange(next);
  }

  function selectMode(mode: GacosRequestMode): void {
    if (mode === value.mode) return;
    // A manually authored request must not silently retain an online source;
    // switching back online requires an explicit fresh freeze click.
    updateDraft({ mode, ...(mode === "manual_dates" ? { sourceBatch: null } : {}) });
  }

  function selectOutputFormat(outputFormat: GacosOutputFormat): void {
    if (outputFormat === value.outputFormat) return;
    updateDraft({ outputFormat });
  }

  async function importCurrentAoi(): Promise<void> {
    if (busy || importBusy) return;
    setImportBusy("current");
    setOperationErrorText(null);
    try {
      const result = await onImportCurrentAoi();
      if (!result || typeof result.name !== "string" || !result.name.trim() || !isObject(result.geometry)) {
        setOperationErrorText("当前 AOI 未返回有效名称和真实几何。");
        return;
      }
      const aoi: GacosAoiDraft = {
        name: result.name.trim(),
        geometry: structuredClone(result.geometry),
      };
      updateDraft({ aoi });
    } catch (error) {
      setOperationErrorText(operationError(error));
    } finally {
      setImportBusy(null);
    }
  }

  async function importAoiFile(): Promise<void> {
    if (busy || importBusy) return;
    setImportBusy("file");
    setOperationErrorText(null);
    try {
      const result = await onImportAoiFile();
      if (!result) return;
      setAoiPreview(structuredClone(result));
      setSelectedFeatureIds(new Set());
    } catch (error) {
      setOperationErrorText(operationError(error));
    } finally {
      setImportBusy(null);
    }
  }

  function toggleFeature(id: string): void {
    setSelectedFeatureIds((current) => {
      const next = new Set(current);
      if (next.has(id)) next.delete(id);
      else next.add(id);
      return next;
    });
    setOperationErrorText(null);
  }

  function applyAoiPreview(): void {
    if (!aoiPreview) return;
    const combined = combineNisarSubsetBoundaryFeatures(aoiPreview, selectedFeatureIds);
    if (!combined.ok) {
      setOperationErrorText(combined.error);
      return;
    }
    const name = aoiPreview.file_name.trim() || aoiPreview.path.trim() || "本地 AOI";
    updateDraft({
      aoi: {
        name,
        geometry: structuredClone(combined.geometry),
      },
    });
    setAoiPreview(null);
    setSelectedFeatureIds(new Set());
  }

  const errorMessages = [...new Set([
    ...validation.errors,
    ...(operationErrorText ? [operationErrorText] : []),
  ])];
  const latestBatchLabel = latestSentinelBatch
    ? `导入最新 Sentinel-1 结果（全部 ${latestSentinelBatch.totalCount} 景）`
    : "导入最新 Sentinel-1 结果（暂无完整检索结果）";

  return (
    <section
      aria-label="GACOS 请求输入"
      className="@container/gacos space-y-3 text-xs"
    >
      <header className="border-b border-border pb-2">
        <h2 className="text-sm font-semibold">来源与范围</h2>
        <p className="mt-1 leading-5 text-muted-foreground">
          选择日期来源与 AOI；每批最多 20 个日期，超出自动分批。
        </p>
      </header>

      <fieldset className="space-y-1.5">
        <legend className="font-medium">日期来源</legend>
        <div className="grid gap-1.5 @min-[30rem]/gacos:grid-cols-2" role="radiogroup" aria-label="GACOS 日期来源">
          <label className={cn(
            "flex items-start gap-2 rounded-md border border-border bg-background/40 p-2",
            value.mode === "online_batch" && "border-primary/50 bg-primary/5",
          )}>
            <input
              type="radio"
              name="gacos-request-mode"
              value="online_batch"
              aria-label="在线 Sentinel-1 批次"
              checked={value.mode === "online_batch"}
              disabled={busy || importBusy !== null}
              onChange={() => selectMode("online_batch")}
            />
            <span>
              <span className="block font-medium">在线 Sentinel-1 批次</span>
              <span className="block text-[11px] text-muted-foreground">按影像携带的 UTC 日期和时间分组。</span>
            </span>
          </label>
          <label className={cn(
            "flex items-start gap-2 rounded-md border border-border bg-background/40 p-2",
            value.mode === "manual_dates" && "border-primary/50 bg-primary/5",
          )}>
            <input
              type="radio"
              name="gacos-request-mode"
              value="manual_dates"
              aria-label="手填日期"
              checked={value.mode === "manual_dates"}
              disabled={busy || importBusy !== null}
              onChange={() => selectMode("manual_dates")}
            />
            <span>
              <span className="block font-medium">手填日期</span>
              <span className="block text-[11px] text-muted-foreground">可独立创建请求，不需要 Sentinel-1 场景。</span>
            </span>
          </label>
        </div>
      </fieldset>

      {value.mode === "online_batch" ? (
        <div className="space-y-2 rounded-md border border-border bg-background/35 p-2.5">
          <Button
            type="button"
            size="sm"
            variant="outline"
            className="w-full justify-start"
            aria-label={latestBatchLabel}
            disabled={busy || importBusy !== null || latestSentinelBatch === null}
            onClick={() => {
              if (!latestSentinelBatch) return;
              // This is the sole online capture point. Later prop changes do
              // not flow into value.sourceBatch.
              updateDraft({
                mode: "online_batch",
                sourceBatch: structuredClone(latestSentinelBatch),
              });
            }}
          >
            {busy ? <Loader2 className="h-4 w-4 animate-spin" /> : <MapPinned className="h-4 w-4" />}
            {latestBatchLabel}
          </Button>
          {sourceBatch ? (
            <div className="rounded-md border border-success/35 bg-success/5 p-2 text-[11px]">
              <div className="flex items-start gap-1.5 font-medium text-success">
                <CheckCircle2 className="mt-0.5 h-3.5 w-3.5 shrink-0" />
                已冻结 Sentinel-1 来源
              </div>
              <div className="mt-1 break-words text-foreground">{sourceBatch.label}</div>
              <div className="mt-1 font-mono text-muted-foreground">
                {sourceBatch.batchId} · {sourceBatch.totalCount} 景
              </div>
            </div>
          ) : (
            <p className="text-[11px] text-warning">请点击上方按钮冻结最新完整检索结果。</p>
          )}
        </div>
      ) : (
        <label className="block">
          <span className="mb-1 block font-medium">GACOS 日期（每行 8 位 YYYYMMDD）</span>
          <Textarea
            aria-label="GACOS 日期（每行 YYYYMMDD）"
            value={value.datesText}
            disabled={busy || importBusy !== null}
            onChange={(event) => updateDraft({ datesText: event.currentTarget.value })}
            placeholder={"20260901\n20260902"}
            rows={4}
            className="min-h-24 rounded-md bg-background/60 px-2 py-1.5 text-xs"
          />
          <span className="mt-1 block text-[11px] text-muted-foreground">重复日期会保留一次；日期必须是真实日历日期，不限制总数。</span>
        </label>
      )}

      <div className="grid gap-2 @min-[30rem]/gacos:grid-cols-2">
        <label className="grid gap-1">
          <span className="font-medium">UTC 小时{value.mode === "online_batch" ? "（纯日期补时，可选）" : "（必选）"}</span>
          <select
            aria-label="UTC 小时"
            value={value.hour}
            disabled={busy || importBusy !== null}
            onChange={(event) => updateDraft({ hour: event.currentTarget.value })}
            className="h-9 rounded-md border border-input bg-background/60 px-2 text-xs focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
          >
            <option value="">请选择 UTC 小时</option>
            {HOURS.map((hour) => (
              <option key={hour} value={String(hour)}>{String(hour).padStart(2, "0")}</option>
            ))}
          </select>
        </label>
        <label className="grid gap-1">
          <span className="font-medium">UTC 分钟{value.mode === "online_batch" ? "（纯日期补时，可选）" : "（必选）"}</span>
          <select
            aria-label="UTC 分钟"
            value={value.minute}
            disabled={busy || importBusy !== null}
            onChange={(event) => updateDraft({ minute: event.currentTarget.value })}
            className="h-9 rounded-md border border-input bg-background/60 px-2 text-xs focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
          >
            <option value="">请选择 UTC 分钟</option>
            {MINUTES.map((minute) => (
              <option key={minute} value={String(minute)}>{String(minute).padStart(2, "0")}</option>
            ))}
          </select>
        </label>
      </div>
      {value.mode === "online_batch" && (
        <p className="text-[11px] text-muted-foreground">在线影像优先使用影像的采集时间（UTC）；上面时分只给缺少时分的纯日期影像补值。</p>
      )}

      <div className="grid gap-3 @min-[36rem]/gacos:grid-cols-[minmax(0,1.4fr)_minmax(0,1fr)]">
        <div className="space-y-2 rounded-md border border-border bg-background/35 p-2.5">
          <div className="font-medium">请求 AOI</div>
          <div className="flex flex-wrap gap-1.5">
            <Button
              type="button"
              size="sm"
              variant="outline"
              disabled={busy || importBusy !== null}
              onClick={() => void importCurrentAoi()}
            >
              {importBusy === "current" ? <Loader2 className="h-4 w-4 animate-spin" /> : <MapPinned className="h-4 w-4" />}
              导入当前 AOI
            </Button>
            <Button
              type="button"
              size="sm"
              variant="outline"
              disabled={busy || importBusy !== null}
              onClick={() => void importAoiFile()}
            >
              {importBusy === "file" ? <Loader2 className="h-4 w-4 animate-spin" /> : <FileUp className="h-4 w-4" />}
              导入本地 AOI
            </Button>
          </div>
          {value.aoi ? (
            <div className="break-words text-[11px]">
              <span className="font-medium">已冻结：</span>{value.aoi.name}
            </div>
          ) : (
            <p className="text-[11px] text-warning">请选择一个请求范围。</p>
          )}
        </div>

        <fieldset className="space-y-2 rounded-md border border-border bg-background/35 p-2.5">
          <legend className="font-medium">输出格式</legend>
          <div className="grid gap-1.5" role="radiogroup" aria-label="GACOS 输出格式">
            <label className={cn("flex items-center gap-2 rounded-md border border-border px-2 py-1.5", value.outputFormat === "geotiff" && "border-primary/50 bg-primary/5")}>
              <input
                type="radio"
                name="gacos-output-format"
                value="geotiff"
                aria-label="GeoTIFF"
                checked={value.outputFormat === "geotiff"}
                disabled={busy || importBusy !== null}
                onChange={() => selectOutputFormat("geotiff")}
              />
              GeoTIFF
            </label>
            <label className={cn("flex items-center gap-2 rounded-md border border-border px-2 py-1.5", value.outputFormat === "binary" && "border-primary/50 bg-primary/5")}>
              <input
                type="radio"
                name="gacos-output-format"
                value="binary"
                aria-label="Binary grid"
                checked={value.outputFormat === "binary"}
                disabled={busy || importBusy !== null}
                onChange={() => selectOutputFormat("binary")}
              />
              Binary grid
            </label>
          </div>
        </fieldset>
      </div>

      <details aria-label="提交前预览" className="rounded-md border border-border bg-muted/25 p-2.5">
        <summary className="cursor-pointer rounded-sm font-medium focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring">
          查看输入摘要
          <span className="ml-2 font-normal text-muted-foreground">{value.outputFormat === "binary" ? "Binary grid" : "GeoTIFF"} · 每批最多 20 个日期</span>
        </summary>
        <dl className="mt-1.5 grid gap-x-3 gap-y-1 text-[11px] @min-[30rem]/gacos:grid-cols-2">
          <div><dt className="text-muted-foreground">来源</dt><dd className="break-words">{value.mode === "online_batch" ? sourceBatch?.label ?? "未冻结 Sentinel-1 批次" : "手填日期"}</dd></div>
          <div><dt className="text-muted-foreground">AOI</dt><dd className="break-words">{value.aoi?.name ?? "未选择"}</dd></div>
          <div><dt className="text-muted-foreground">日期</dt><dd className="break-words">{value.mode === "manual_dates" ? validation.dates.join("、") || "无有效日期" : "由影像 UTC 自动分组"}</dd></div>
          <div><dt className="text-muted-foreground">时间</dt><dd>{value.mode === "online_batch" && !value.hour && !value.minute ? "影像 UTC；纯日期无补时" : `${displayTimePart(value.hour, 23)}:${displayTimePart(value.minute, 59)}`}</dd></div>
          <div><dt className="text-muted-foreground">格式</dt><dd>{value.outputFormat === "binary" ? "Binary grid" : "GeoTIFF"}</dd></div>
          <div><dt className="text-muted-foreground">日期分批</dt><dd>每批最多20个日期，超过后自动分批</dd></div>
        </dl>
      </details>

      {errorMessages.length > 0 ? (
        <div role="alert" className="rounded-md border border-warning/40 bg-warning/10 p-2.5 text-[11px]">
          <div className="font-medium text-warning">提交前请修正以下输入</div>
          <ul aria-label="GACOS 输入校验" className="mt-1 list-disc space-y-0.5 pl-4">
            {errorMessages.map((message) => <li key={message}>{message}</li>)}
          </ul>
        </div>
      ) : (
        <div className="flex items-center gap-1.5 rounded-md border border-success/35 bg-success/5 p-2.5 text-[11px] text-success">
          <CheckCircle2 className="h-3.5 w-3.5" />
          输入完整，可以生成请求预览
        </div>
      )}

      {aoiPreview && (
        <div className="fixed inset-0 z-[1300] flex items-center justify-center bg-black/35 p-4 backdrop-blur-sm">
          <section
            role="dialog"
            aria-modal="true"
            aria-label="选择本地 AOI 要素"
            className="flex max-h-[80vh] w-full max-w-lg flex-col rounded-xl border border-border bg-card text-card-foreground shadow-2xl"
          >
            <header className="flex items-center justify-between gap-3 border-b border-border px-4 py-3">
              <div className="min-w-0">
                <h3 className="text-sm font-semibold">选择本地 AOI 要素</h3>
                <p className="truncate text-[11px] text-muted-foreground">
                  {aoiPreview.file_name} · {aoiPreview.total_features} 个要素
                </p>
              </div>
              <button
                type="button"
                aria-label="关闭本地 AOI 预览"
                onClick={() => { setAoiPreview(null); setSelectedFeatureIds(new Set()); }}
                className="rounded-md p-2 hover:bg-muted focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
              >
                <X className="h-4 w-4" />
              </button>
            </header>
            <div className="min-h-0 flex-1 space-y-1.5 overflow-y-auto p-3">
              {aoiPreview.features.length === 0 ? (
                <p className="text-xs text-warning">预览中没有可选择的边界要素。</p>
              ) : aoiPreview.features.map((feature) => (
                <label key={feature.id} className="flex items-start gap-2 rounded-md border border-border p-2 text-xs">
                  <input
                    type="checkbox"
                    aria-label={`${featureLabel(feature)} AOI 要素`}
                    checked={selectedFeatureIds.has(feature.id)}
                    onChange={() => toggleFeature(feature.id)}
                    className="mt-0.5"
                  />
                  <span className="min-w-0 flex-1 break-words">
                    <span className="block font-medium">{featureLabel(feature)}</span>
                    <span className="mt-0.5 block text-[11px] text-muted-foreground">显式选择后才会写入本次 GACOS 草稿</span>
                  </span>
                </label>
              ))}
              {operationErrorText && <p role="alert" className="break-words text-xs text-destructive">{operationErrorText}</p>}
            </div>
            <footer className="grid grid-cols-[auto_1fr] gap-2 border-t border-border p-3">
              <button
                type="button"
                onClick={() => { setAoiPreview(null); setSelectedFeatureIds(new Set()); }}
                className="h-9 rounded-md border border-border px-3 text-xs"
              >
                取消
              </button>
              <button
                type="button"
                disabled={selectedFeatureCount === 0}
                onClick={applyAoiPreview}
                className="h-9 rounded-md bg-primary px-3 text-xs font-semibold text-primary-foreground disabled:cursor-not-allowed disabled:opacity-50"
              >
                应用所选 AOI（{selectedFeatureCount}）
              </button>
            </footer>
          </section>
        </div>
      )}
    </section>
  );
}
