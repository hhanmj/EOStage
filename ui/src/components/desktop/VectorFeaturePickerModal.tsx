/**
 * VectorFeaturePickerModal.tsx
 * 矢量文件多要素勾选与一键合并弹窗。
 * 对接 Rust `crates/insar-gis/src/vector.rs` (previewAoiFileBytes) 解析出的 AoiPreviewOk，
 * 支持拖拽或上传 .shp / .kml / .kmz / .geojson / .json 多要素文件时按需勾选并合并为工作区 AOI。
 */

import React, { useEffect, useMemo, useState } from "react";
import { CheckSquare, Layers, Search, Square, X } from "lucide-react";
import { ModalOverlay } from "@/components/ui/ModalOverlay";
import { SpSelect } from "@/components/ui/SpSelect";
import { cn } from "@/lib/utils";
import type { AoiFeaturePreview, AoiPreviewOk } from "@/lib/bridge";
import { vectorAoiDisplayName } from "@/shell/api/agentComposerLogic";

export interface VectorFeaturePickerModalProps {
  open: boolean;
  preview: AoiPreviewOk | null;
  busy?: boolean;
  onCancel: () => void;
  onConfirm: (params: {
    featureIds: string[];
    displayField: string;
    displayName: string;
  }) => Promise<void> | void;
}

export function resolveFeatureId(f: AoiFeaturePreview, fallbackIdx: number): string {
  return f.id || (f.properties?.id as string) || String(f.index ?? fallbackIdx);
}

export function resolveFeatureDisplayTitle(
  f: AoiFeaturePreview,
  displayField: string,
  fallbackIdx: number
): string {
  if (displayField && f.properties && f.properties[displayField] !== undefined && f.properties[displayField] !== null) {
    const val = String(f.properties[displayField]).trim();
    if (val) return val;
  }
  if (f.name && f.name.trim()) return f.name.trim();
  return `要素 #${(f.index ?? fallbackIdx) + 1}`;
}

export function VectorFeaturePickerModal({
  open,
  preview,
  busy = false,
  onCancel,
  onConfirm,
}: VectorFeaturePickerModalProps) {
  const [selectedIds, setSelectedIds] = useState<Set<string>>(new Set());
  const [displayField, setDisplayField] = useState<string>("");
  const [searchQuery, setSearchQuery] = useState<string>("");

  useEffect(() => {
    if (!open || !preview) return;
    const allIds = (preview.features || []).map((f, idx) => resolveFeatureId(f, idx));
    setSelectedIds(new Set(allIds));
    setDisplayField(preview.display_field || preview.fields?.[0] || "");
    setSearchQuery("");
  }, [open, preview]);

  const features = preview?.features ?? [];
  const fields = preview?.fields ?? [];

  const indexedFeatures = useMemo(
    () =>
      features.map((feature, originalIdx) => ({
        feature,
        id: resolveFeatureId(feature, originalIdx),
        originalIdx,
      })),
    [features]
  );

  const filteredFeatures = useMemo(() => {
    const q = searchQuery.trim().toLowerCase();
    if (!q) return indexedFeatures;
    return indexedFeatures.filter(({ feature, originalIdx }) => {
      const title = resolveFeatureDisplayTitle(feature, displayField, originalIdx).toLowerCase();
      if (title.includes(q)) return true;
      const propText = Object.values(feature.properties || {})
        .map((v) => String(v ?? ""))
        .join(" ")
        .toLowerCase();
      return propText.includes(q);
    });
  }, [indexedFeatures, displayField, searchQuery]);

  if (!open || !preview) return null;

  const toggleFeature = (id: string) => {
    setSelectedIds((prev) => {
      const next = new Set(prev);
      if (next.has(id)) {
        next.delete(id);
      } else {
        next.add(id);
      }
      return next;
    });
  };

  const handleSelectAllFiltered = () => {
    setSelectedIds((prev) => {
      const next = new Set(prev);
      for (const entry of filteredFeatures) {
        next.add(entry.id);
      }
      return next;
    });
  };

  const handleInvertSelection = () => {
    setSelectedIds((prev) => {
      const next = new Set<string>();
      for (const entry of indexedFeatures) {
        if (!prev.has(entry.id)) next.add(entry.id);
      }
      return next;
    });
  };

  const totalAreaKm2 = indexedFeatures.reduce((sum, { feature, id }) => {
    if (!selectedIds.has(id)) return sum;
    return sum + (typeof feature.area_km2 === "number" && Number.isFinite(feature.area_km2) ? feature.area_km2 : 0);
  }, 0);

  const handleConfirm = () => {
    if (selectedIds.size === 0 || busy) return;
    const baseName = vectorAoiDisplayName(preview.file_name) || "矢量范围";
    const chosenTitles = indexedFeatures
      .filter((entry) => selectedIds.has(entry.id))
      .slice(0, 2)
      .map((entry) => resolveFeatureDisplayTitle(entry.feature, displayField, entry.originalIdx));
    const displayName =
      selectedIds.size === features.length
        ? `${baseName} (${selectedIds.size}个要素)`
        : selectedIds.size === 1
          ? `${baseName} · ${chosenTitles[0]}`
          : `${baseName} · ${chosenTitles.join("+")} 等${selectedIds.size}个要素`;
    void onConfirm({
      featureIds: Array.from(selectedIds),
      displayField,
      displayName,
    });
  };

  return (
    <ModalOverlay
      role="dialog"
      aria-modal={true}
      aria-label="选择要导入的矢量要素"
      data-testid="vector-feature-picker-modal"
      className="z-[2200]"
      closeOnBackdropClick={false}
    >
      <div className="w-[560px] max-w-full max-h-[85vh] rounded-card border border-sp-line bg-sp-surface shadow-modal flex flex-col overflow-hidden text-xs text-sp-text">
        {/* 头部 */}
        <div className="flex items-center justify-between gap-2 px-4 py-3 border-b border-sp-line bg-sp-card">
          <div className="flex items-center gap-2 min-w-0">
            <Layers className="h-4 w-4 text-sp-accent shrink-0" />
            <div className="min-w-0">
              <div className="text-sm font-semibold text-sp-text truncate">
                检测到多要素矢量文件 · 请勾选要添加的范围
              </div>
              <div className="text-xs text-sp-text-muted truncate">
                {preview.file_name} · 共 {preview.total_features || features.length} 个面要素（Rust 原生引擎解析）
              </div>
            </div>
          </div>
          <button
            type="button"
            disabled={busy}
            aria-label="关闭矢量要素选择弹窗"
            onClick={onCancel}
            className="p-1 rounded-sp-sm text-sp-text-muted hover:text-sp-text hover:bg-sp-hover cursor-pointer"
          >
            <X className="h-4 w-4" />
          </button>
        </div>

        {/* 工具栏：搜索 + 名称字段选择 + 全选/反选 */}
        <div className="flex flex-wrap items-center justify-between gap-2 px-4 py-2.5 border-b border-sp-line bg-sp-surface-muted">
          <div className="relative flex-1 min-w-[160px]">
            <Search className="pointer-events-none absolute left-2.5 top-1/2 -translate-y-1/2 h-3.5 w-3.5 text-sp-text-muted" />
            <input
              type="text"
              value={searchQuery}
              onChange={(e) => setSearchQuery(e.target.value)}
              placeholder="按要素名称或属性过滤..."
              aria-label="过滤矢量要素"
              className="h-8 w-full rounded-sp-sm border border-sp-line bg-sp-surface pl-8 pr-2 text-xs text-sp-text outline-none focus:ring-1 focus:ring-sp-accent"
            />
          </div>

          {fields.length > 0 && (
            <div className="flex items-center gap-1.5">
              <span className="text-sp-text-muted">名称字段:</span>
              <SpSelect
                aria-label="要素名称字段"
                value={displayField}
                onChange={setDisplayField}
                size="sm"
                className="min-w-[120px]"
                options={fields.map((field) => ({ value: field, label: field }))}
              />
            </div>
          )}

          <div className="flex items-center gap-1">
            <button
              type="button"
              data-testid="vector-picker-select-all"
              onClick={handleSelectAllFiltered}
              className="h-8 rounded-sp-sm border border-sp-line bg-sp-surface px-2.5 text-xs text-sp-text-secondary hover:text-sp-text hover:bg-sp-hover cursor-pointer"
            >
              全选
            </button>
            <button
              type="button"
              data-testid="vector-picker-invert"
              onClick={handleInvertSelection}
              className="h-8 rounded-sp-sm border border-sp-line bg-sp-surface px-2.5 text-xs text-sp-text-secondary hover:text-sp-text hover:bg-sp-hover cursor-pointer"
            >
              反选
            </button>
          </div>
        </div>

        {/* 要素列表 */}
        <div className="flex-1 min-h-0 max-h-[340px] overflow-y-auto p-3 space-y-1.5">
          {filteredFeatures.map(({ feature: feat, id, originalIdx }) => {
            const checked = selectedIds.has(id);
            const title = resolveFeatureDisplayTitle(feat, displayField, originalIdx);
            const areaText =
              typeof feat.area_km2 === "number" && feat.area_km2 > 0
                ? `${feat.area_km2.toLocaleString("zh-CN", { maximumFractionDigits: feat.area_km2 < 100 ? 1 : 0 })} km²`
                : null;

            return (
              <div
                key={id}
                data-testid={`vector-feature-row-${id}`}
                onClick={() => toggleFeature(id)}
                className={cn(
                  "flex items-center justify-between gap-3 rounded-sp-sm border px-3 py-2 cursor-pointer select-none transition-colors",
                  checked
                    ? "border-sp-accent/40 bg-sp-selected"
                    : "border-sp-line bg-sp-card hover:bg-sp-hover"
                )}
              >
                <div className="flex items-center gap-2.5 min-w-0 flex-1">
                  {checked ? (
                    <CheckSquare className="h-4 w-4 text-sp-accent shrink-0" />
                  ) : (
                    <Square className="h-4 w-4 text-sp-text-muted shrink-0" />
                  )}
                  <div className="min-w-0 flex-1">
                    <div className="font-semibold text-sp-text truncate">{title}</div>
                    {feat.properties && Object.keys(feat.properties).length > 0 && (
                      <div className="text-xs text-sp-text-muted truncate">
                        {Object.entries(feat.properties)
                          .slice(0, 3)
                          .map(([k, v]) => `${k}: ${String(v ?? "")}`)
                          .join(" · ")}
                      </div>
                    )}
                  </div>
                </div>
                {areaText && (
                  <span className="shrink-0 font-mono text-xs text-sp-text-secondary tabular-nums">
                    {areaText}
                  </span>
                )}
              </div>
            );
          })}
        </div>

        {/* 底部确认栏 */}
        <div className="flex items-center justify-between gap-3 px-4 py-3 border-t border-sp-line bg-sp-card">
          <div className="text-xs text-sp-text-secondary tabular-nums">
            已勾选 <strong className="font-semibold text-sp-text">{selectedIds.size}</strong> /{" "}
            {features.length} 个要素
            {totalAreaKm2 > 0 && (
              <span>
                {" "}
                · 合计约{" "}
                <strong className="font-semibold text-sp-text">
                  {Math.round(totalAreaKm2).toLocaleString("zh-CN")} km²
                </strong>
              </span>
            )}
          </div>
          <div className="flex items-center gap-2">
            <button
              type="button"
              disabled={busy}
              onClick={onCancel}
              className="h-8 rounded-sp-sm border border-sp-line bg-sp-surface px-3 text-xs text-sp-text-secondary hover:text-sp-text hover:bg-sp-hover cursor-pointer"
            >
              取消
            </button>
            <button
              type="button"
              data-testid="vector-picker-confirm"
              disabled={selectedIds.size === 0 || busy}
              onClick={handleConfirm}
              className="h-8 rounded-sp-sm bg-sp-accent-fill px-3.5 text-xs font-semibold text-white hover:opacity-90 disabled:opacity-50 cursor-pointer"
            >
              {busy ? "正在合并导入..." : `合并并添加所选 (${selectedIds.size})`}
            </button>
          </div>
        </div>
      </div>
    </ModalOverlay>
  );
}
