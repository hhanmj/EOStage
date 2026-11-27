import React, { useState, useEffect } from "react";
import { cn } from "@/lib/utils";
import { useI18n } from "@/i18n";
import {
  getPerformanceAndUsagePrefs,
  setPerformanceOnlyPrefs,
  subscribePerformanceAndUsagePrefs,
  type PerformanceOnlyPrefs,
  type ContextHudMode,
} from "@/lib/performanceAndUsagePrefs";

/**
 * PerformanceUsageSettingsTab.tsx
 * 独立的「性能与用量」设置页（对齐 DSH performance-usage 独立分组）：
 * - 负责性能与用量信息展示详细程度（简洁 / 详细）
 * - 与「工作步骤显示」(WorkStepsSettingsTab) 完全解耦为两个独立设置入口
 */
export function PerformanceUsageSettingsTab() {
  const { t } = useI18n();
  const [prefs, setPrefs] = useState<PerformanceOnlyPrefs>(() => ({
    hudMode: getPerformanceAndUsagePrefs().hudMode,
  }));

  useEffect(() => {
    return subscribePerformanceAndUsagePrefs((next) => {
      setPrefs({ hudMode: next.hudMode });
    });
  }, []);

  const handleSelectHudMode = (hudMode: ContextHudMode) => {
    const next = setPerformanceOnlyPrefs({ hudMode });
    setPrefs(next);
  };

  return (
    <div className="eo-settings-section" data-testid="performance-usage-settings-tab">
      <h3 className="text-[14px] font-semibold text-sp-text">
        {t("settings.performance.title")}
      </h3>

      <div className="eo-settings-card">
        <div className="eo-settings-card-content">
          <div className="eo-settings-card-row">
            <div className="flex flex-wrap items-center justify-between gap-4">
              <div className="flex min-w-0 flex-1 flex-col gap-1">
                <h4 className="text-[13px] font-normal text-sp-text">
                  {t("settings.performance.description")}
                </h4>
              </div>

              <div
                role="group"
                aria-label={t("settings.performance.title")}
                className="eo-segmented-track shrink-0"
              >
                <button
                  type="button"
                  data-testid="perf-hud-mode-minimal"
                  aria-pressed={prefs.hudMode === "minimal"}
                  data-active={prefs.hudMode === "minimal"}
                  onClick={() => handleSelectHudMode("minimal")}
                  className="eo-segmented-pill h-[26px] justify-center whitespace-nowrap px-3 cursor-pointer"
                >
                  {t("settings.performance.compact")}
                </button>
                <button
                  type="button"
                  data-testid="perf-hud-mode-pro"
                  aria-pressed={prefs.hudMode === "pro"}
                  data-active={prefs.hudMode === "pro"}
                  onClick={() => handleSelectHudMode("pro")}
                  className="eo-segmented-pill h-[26px] justify-center whitespace-nowrap px-3 cursor-pointer"
                >
                  {t("settings.performance.detailed")}
                </button>
              </div>
            </div>
          </div>
        </div>

        {/* Concentric SettingsCardBeard */}
        <div className="eo-settings-card-beard">
          {prefs.hudMode === "minimal" ? (
            <span>
              <strong className="text-sp-text font-semibold">
                {t("settings.performance.compactSummaryTitle")}
              </strong>
              {t("settings.performance.compactSummaryBody")}
            </span>
          ) : (
            <span>
              <strong className="text-sp-text font-semibold">
                {t("settings.performance.detailedSummaryTitle")}
              </strong>
              {t("settings.performance.detailedSummaryBody")}
            </span>
          )}
          <p
            data-testid="perf-hud-right-click-hint"
            className="mt-2 text-sp-text-muted"
          >
            {t("settings.performance.rightClickHint")}
          </p>
        </div>
      </div>
    </div>
  );
}
