/**
 * WorkStepsSettingsTab.tsx
 * 设置 → 「工作步骤显示」独立入口面板（对齐 DSH TranscriptViewRow / id: "transcript-view"）
 *
 * 专门管理 Agent 思考链与工作步骤的信息显示完整程度：
 * 1. 工作步骤显示 (stepDetailMode): 简洁 (concise) ⇄ 详细 (detailed)
 * 2. 工作步骤收起时机 (collapseTiming): 回答结束后 (completion) ⇄ 下次有新消息时 (next-input)
 * 3. 收起后显示的回复 (foldedReplies): 全部回复 (all) ⇄ 仅最终回答 (final)
 */

import React, { useEffect, useState } from "react";
import { Layers } from "lucide-react";
import { cn } from "@/lib/utils";
import { useI18n } from "@/i18n";
import {
  getWorkStepsPrefs,
  setWorkStepsPrefs,
  subscribeWorkStepsPrefs,
  type AgentStepDetailMode,
  type WorkStepsCollapseTiming,
  type WorkStepsFoldedReplies,
} from "@/lib/performanceAndUsagePrefs";

export function WorkStepsSettingsTab() {
  const { t } = useI18n();
  const [prefs, setPrefs] = useState(() => getWorkStepsPrefs());

  useEffect(() => {
    return subscribeWorkStepsPrefs((next) => {
      setPrefs(next);
    });
  }, []);

  const handleStepMode = (stepDetailMode: AgentStepDetailMode) => {
    setWorkStepsPrefs({ stepDetailMode });
  };

  const handleCollapseTiming = (collapseTiming: WorkStepsCollapseTiming) => {
    setWorkStepsPrefs({ collapseTiming });
  };

  const handleFoldedReplies = (foldedReplies: WorkStepsFoldedReplies) => {
    setWorkStepsPrefs({ foldedReplies });
  };

  return (
    <div className="eo-settings-section" data-testid="work-steps-settings-tab">
      <h3 className="text-[14px] font-medium text-sp-text">
        {t("settings.transcript.title")}
      </h3>

      {/* Concentric SettingsCard */}
      <div className="eo-settings-card">
        <div className="eo-settings-card-content">
          {/* 行 1：工作步骤显示（简洁 / 详细） */}
          <div className="eo-settings-card-row">
            <div className="flex flex-wrap items-center justify-between gap-4">
              <div className="flex min-w-0 flex-col gap-0.5">
                <h4 className="text-[13px] font-normal text-sp-text">
                  {t("settings.transcript.modeRowLabel")}
                </h4>
              </div>

              <div
                className="eo-segmented-track shrink-0"
                role="group"
                aria-label={t("settings.transcript.title")}
              >
                <button
                  type="button"
                  data-testid="work-step-mode-concise"
                  aria-pressed={prefs.stepDetailMode === "concise"}
                  data-active={prefs.stepDetailMode === "concise"}
                  onClick={() => handleStepMode("concise")}
                  className="eo-segmented-pill h-[26px] px-3 text-[12px] cursor-pointer"
                >
                  <span>{t("settings.transcript.compact")}</span>
                </button>
                <button
                  type="button"
                  data-testid="work-step-mode-detailed"
                  aria-pressed={prefs.stepDetailMode === "detailed"}
                  data-active={prefs.stepDetailMode === "detailed"}
                  onClick={() => handleStepMode("detailed")}
                  className="eo-segmented-pill h-[26px] px-3 text-[12px] cursor-pointer"
                >
                  <span>{t("settings.transcript.detailed")}</span>
                </button>
              </div>
            </div>
          </div>

          {/* 行 2：工作步骤收起时机 */}
          <div className="eo-settings-card-row">
            <div className="flex flex-wrap items-center justify-between gap-4">
              <div className="flex min-w-0 flex-col gap-0.5">
                <h4 className="text-[13px] font-normal text-sp-text">
                  {t("settings.transcript.collapseTitle")}
                </h4>
                <div className="text-[12.5px] text-sp-text-secondary">
                  {t("settings.transcript.collapseDescription")}
                </div>
              </div>

              <div
                className="eo-segmented-track shrink-0"
                role="group"
                aria-label={t("settings.transcript.collapseTitle")}
              >
                <button
                  type="button"
                  data-testid="work-step-collapse-completion"
                  aria-pressed={prefs.collapseTiming === "completion"}
                  data-active={prefs.collapseTiming === "completion"}
                  onClick={() => handleCollapseTiming("completion")}
                  className="eo-segmented-pill h-[26px] px-3 text-[12px] cursor-pointer"
                >
                  <span>{t("settings.transcript.collapseCompletion")}</span>
                </button>
                <button
                  type="button"
                  data-testid="work-step-collapse-next-input"
                  aria-pressed={prefs.collapseTiming === "next-input"}
                  data-active={prefs.collapseTiming === "next-input"}
                  onClick={() => handleCollapseTiming("next-input")}
                  className="eo-segmented-pill h-[26px] px-3 text-[12px] cursor-pointer"
                >
                  <span>{t("settings.transcript.collapseNextInput")}</span>
                </button>
              </div>
            </div>
          </div>

          {/* 行 3：收起后显示的回复 */}
          <div className="eo-settings-card-row">
            <div className="flex flex-wrap items-center justify-between gap-4">
              <div className="flex min-w-0 flex-col gap-0.5">
                <h4 className="text-[13px] font-normal text-sp-text">
                  {t("settings.transcript.foldedRepliesTitle")}
                </h4>
                <div className="text-[12.5px] text-sp-text-secondary">
                  {t("settings.transcript.foldedRepliesDescription")}
                </div>
              </div>

              <div
                className="eo-segmented-track shrink-0"
                role="group"
                aria-label={t("settings.transcript.foldedRepliesTitle")}
              >
                <button
                  type="button"
                  data-testid="work-step-replies-all"
                  aria-pressed={prefs.foldedReplies === "all"}
                  data-active={prefs.foldedReplies === "all"}
                  onClick={() => handleFoldedReplies("all")}
                  className="eo-segmented-pill h-[26px] px-3 text-[12px] cursor-pointer"
                >
                  <span>{t("settings.transcript.foldedRepliesAll")}</span>
                </button>
                <button
                  type="button"
                  data-testid="work-step-replies-final"
                  aria-pressed={prefs.foldedReplies === "final"}
                  data-active={prefs.foldedReplies === "final"}
                  onClick={() => handleFoldedReplies("final")}
                  className="eo-segmented-pill h-[26px] px-3 text-[12px] cursor-pointer"
                >
                  <span>{t("settings.transcript.foldedRepliesFinal")}</span>
                </button>
              </div>
            </div>
          </div>
        </div>

        {/* Concentric SettingsCardBeard */}
        <div className="eo-settings-card-beard">
          {prefs.stepDetailMode === "concise" ? (
            <div className="flex items-center gap-2">
              <Layers className="h-3.5 w-3.5 text-sp-text-muted shrink-0" />
              <span>
                <strong className="font-medium text-sp-text">
                  {t("settings.transcript.compactSummaryTitle")}
                </strong>
                {t("settings.transcript.compactSummaryBody")}
              </span>
            </div>
          ) : (
            <div className="flex items-center gap-2">
              <Layers className="h-3.5 w-3.5 text-sp-text-muted shrink-0" />
              <span>
                <strong className="font-medium text-sp-text">
                  {t("settings.transcript.detailedSummaryTitle")}
                </strong>
                {t("settings.transcript.detailedSummaryBody")}
              </span>
            </div>
          )}
        </div>
      </div>
    </div>
  );
}
