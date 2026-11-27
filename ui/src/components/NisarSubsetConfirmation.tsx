import React, { useEffect, useState } from "react";
import { CheckCircle2, FolderOpen, AlertTriangle } from "lucide-react";

import type {
  DesktopJsonObject,
  DesktopNisarConfirmSubsetBatchTaskV2Request,
  DesktopNisarPreparedSubsetBatchV2,
} from "@/lib/desktopShellBridge";
import { SubsetScissorsIcon } from "./desktop/SubsetScissorsIcon";
import { isSbandSubsetIntent, NisarSbandSourceNotice } from "./NisarSbandSourceNotice";

export type NisarSubsetConfirmationProps = {
  prepared: DesktopNisarPreparedSubsetBatchV2;
  outputRoot: string;
  taskRequest: DesktopNisarConfirmSubsetBatchTaskV2Request;
  onChooseDirectory: () => Promise<string | null>;
  onCancel: () => void;
  onConfirm: (request: DesktopNisarConfirmSubsetBatchTaskV2Request) => Promise<void> | void;
};

function intentValue(intent: DesktopJsonObject, key: string): unknown {
  return intent[key];
}

function safeGeometryType(value: unknown): "Polygon" | "MultiPolygon" | null {
  if (!value || typeof value !== "object" || Array.isArray(value)) return null;
  const record = value as DesktopJsonObject;
  if ("canonical_geojson" in record) {
    if (!record.canonical_geojson || typeof record.canonical_geojson !== "object" || Array.isArray(record.canonical_geojson)) return null;
  }
  const candidate = "canonical_geojson" in record
    ? record.canonical_geojson as DesktopJsonObject
    : record;
  return candidate.type === "Polygon" || candidate.type === "MultiPolygon"
    ? candidate.type
    : null;
}

function overlapLabel(intent: DesktopJsonObject): string {
  return safeGeometryType(intentValue(intent, "effective_geometry"))
    ? "已确认相交"
    : "相交状态不可用";
}

function errorMessage(value: unknown): string {
  if (value instanceof Error && value.message.trim()) return value.message;
  if (value && typeof value === "object" && "error" in value && typeof value.error === "string") {
    return value.error;
  }
  return "创建裁剪任务失败，请重试。";
}

export function NisarSubsetConfirmation({
  prepared,
  outputRoot: initialOutputRoot,
  taskRequest,
  onChooseDirectory,
  onCancel,
  onConfirm,
}: NisarSubsetConfirmationProps) {
  const [outputRoot, setOutputRoot] = useState(initialOutputRoot);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState(null as string | null);
  const firstIntent = prepared.intents[0]?.intent ?? {};
  const hasSbandIntent = prepared.intents.some((item) => isSbandSubsetIntent(item.intent));
  const frequency = intentValue(firstIntent, "selected_frequency");
  const polarizations = intentValue(firstIntent, "selected_polarizations");
  const requestedGeometryType = safeGeometryType(intentValue(firstIntent, "requested_geometry"));

  async function chooseDirectory() {
    try {
      const chosen = await onChooseDirectory();
      if (chosen) setOutputRoot(chosen);
    } catch (caught) {
      setError(errorMessage(caught));
    }
  }

  async function confirm() {
    if (busy) return;
    setBusy(true);
    setError(null);
    const request: DesktopNisarConfirmSubsetBatchTaskV2Request = {
      ...taskRequest,
      output_root: outputRoot,
    };
    try {
      await onConfirm(request);
    } catch (caught) {
      setError(errorMessage(caught));
    } finally {
      setBusy(false);
    }
  }

  useEffect(() => {
    const handleKeyDown = (event: KeyboardEvent) => {
      if (event.key === "Escape") {
        if (busy) return;
        event.preventDefault();
        onCancel();
      }
    };
    window.addEventListener("keydown", handleKeyDown);
    return () => window.removeEventListener("keydown", handleKeyDown);
  }, [busy, onCancel]);

  return (
    <div className="fixed inset-0 z-[1400] flex items-center justify-center bg-black/25 p-4">
      <section
        role="dialog"
        aria-modal="true"
        aria-label="确认 NISAR 裁剪任务"
        className="max-h-[90vh] w-full max-w-2xl overflow-y-auto rounded-modal border border-sp-line bg-sp-surface text-sp-text shadow-modal"
      >
        <header className="flex items-center gap-2.5 border-b border-sp-line bg-[#fafafa] dark:bg-[#141416] px-5 py-3.5">
          <SubsetScissorsIcon className="h-4 w-4 text-sp-text-secondary" />
          <div className="min-w-0 flex-1">
            <h2 className="text-[13px] font-semibold text-sp-text">确认 NISAR 裁剪任务</h2>
            <p className="mt-0.5 truncate font-mono text-[11px] text-sp-text-secondary">
              来源批次：{prepared.source_batch_id}
            </p>
          </div>
        </header>

        <div className="p-5">
          {hasSbandIntent && <NisarSbandSourceNotice />}

          <dl className="mt-3 divide-y divide-sp-line rounded-sp-md border border-sp-line bg-sp-surface text-xs">
            <div className="grid grid-cols-3 divide-x divide-sp-line">
              <div className="px-3 py-2.5">
                <dt className="text-[11px] text-sp-text-secondary">影像数</dt>
                <dd className="mt-0.5 font-semibold tabular-nums text-sp-text">{prepared.intents.length} 景</dd>
              </div>
              <div className="px-3 py-2.5">
                <dt className="text-[11px] text-sp-text-secondary">Frequency</dt>
                <dd className="mt-0.5 font-semibold text-sp-text">Frequency {String(frequency ?? "未知")}</dd>
              </div>
              <div className="px-3 py-2.5">
                <dt className="text-[11px] text-sp-text-secondary">Polarizations</dt>
                <dd className="mt-0.5 font-mono font-semibold text-sp-text">
                  {Array.isArray(polarizations) ? polarizations.join(", ") : "未知"}
                </dd>
              </div>
            </div>
            <div className="px-3 py-2.5">
              <dt className="text-[11px] text-sp-text-secondary">请求 AOI</dt>
              <dd className="mt-0.5 font-medium text-sp-text">
                {requestedGeometryType ?? "不可用"}，{prepared.intents.length} 景使用同一请求范围
              </dd>
            </div>
            <div className="px-3 py-2.5">
              <dt className="text-[11px] text-sp-text-secondary">预计数据量</dt>
              <dd className="mt-0.5 font-medium text-sp-text">
                将在读取产品结构后确定（{prepared.intents.length} 景未知）
              </dd>
            </div>
            <div className="flex items-center justify-between gap-3 px-3 py-2.5">
              <div className="min-w-0">
                <dt className="text-[11px] text-sp-text-secondary">输出目录</dt>
                <dd className="mt-0.5 break-all font-mono font-medium text-sp-text">{outputRoot}</dd>
              </div>
              <button
                type="button"
                onClick={() => void chooseDirectory()}
                disabled={busy}
                className="inline-flex h-7 shrink-0 items-center gap-1.5 rounded-sp-sm border border-sp-line bg-sp-surface px-2.5 text-xs font-medium text-sp-text hover:bg-sp-hover disabled:opacity-50 cursor-pointer"
              >
                <FolderOpen className="h-3.5 w-3.5 text-sp-text-secondary" />
                选择目录
              </button>
            </div>
          </dl>

          <div className="mt-4 rounded-sp-md border border-sp-line overflow-hidden">
            <h3 className="border-b border-sp-line bg-[#fafafa] dark:bg-[#141416] px-3 py-2 text-xs font-semibold text-sp-text">
              逐景准备结果
            </h3>
            <ul aria-label="逐景相交结果" className="divide-y divide-sp-line max-h-48 overflow-y-auto">
              {prepared.intents.map((item) => {
                const label = overlapLabel(item.intent);
                const ok = label === "已确认相交";
                return (
                  <li key={item.source_item_id} className="flex items-center justify-between gap-3 px-3 py-2 text-xs">
                    <span className="break-all font-mono font-medium text-sp-text">{item.source_item_id}</span>
                    <span
                      className={
                        ok
                          ? "inline-flex shrink-0 items-center gap-1 rounded-sp-sm border border-sp-line bg-sp-surface-muted px-2 py-0.5 font-medium text-sp-text"
                          : "inline-flex shrink-0 items-center gap-1 rounded-sp-sm bg-sp-warning-bg px-2 py-0.5 font-medium text-sp-warning-text"
                      }
                    >
                      {ok && <CheckCircle2 className="h-3 w-3 text-sp-success-text" />}
                      {label}
                    </span>
                  </li>
                );
              })}
            </ul>
          </div>

          {prepared.excluded_items.length > 0 && (
            <div className="mt-4 rounded-sp-md border border-sp-warning-text/30 bg-sp-warning-bg p-3 text-xs text-sp-warning-text">
              <div className="flex items-center gap-1.5 font-semibold">
                <AlertTriangle className="h-3.5 w-3.5" />
                <h3>已排除影像</h3>
              </div>
              <ul className="mt-1.5 list-disc space-y-1 pl-5">
                {prepared.excluded_items.map((item) => (
                  <li key={item.source_item_id}>
                    <span className="font-mono font-semibold">{item.source_item_id}</span>：{item.reason}
                  </li>
                ))}
              </ul>
            </div>
          )}

          <p className="mt-3.5 text-xs text-sp-text-secondary">
            输出：每景一个客户端派生、PROVISIONAL HDF5。
          </p>
          {error && (
            <p role="alert" className="mt-3 rounded-sp-sm bg-sp-danger-bg px-3 py-2 text-xs text-sp-danger-text">
              {error}
            </p>
          )}
        </div>

        <footer className="flex flex-wrap justify-end gap-2 border-t border-sp-line bg-[#fafafa] dark:bg-[#141416] px-5 py-3">
          <button
            type="button"
            onClick={onCancel}
            disabled={busy}
            className="h-8 rounded-sp-sm border border-sp-line bg-sp-surface px-3.5 text-xs font-medium text-sp-text hover:bg-sp-hover disabled:opacity-50 cursor-pointer"
          >
            取消
          </button>
          <button
            type="button"
            onClick={() => void confirm()}
            disabled={busy || outputRoot.trim().length === 0}
            className="eo-settings-btn-primary h-8 rounded-sp-sm bg-sp-accent-fill px-4 text-xs font-semibold text-[var(--sp-on-accent-fill,#fff)] hover:opacity-90 disabled:opacity-50 cursor-pointer"
          >
            {busy ? "创建中…" : "确认创建裁剪任务"}
          </button>
        </footer>
      </section>
    </div>
  );
}
