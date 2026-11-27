/**
 * ReasoningIntensitySlider.tsx
 * EOStage-style horizontal stepped Reasoning Intensity Slider ("思考强度 <label> [Ctrl+T]").
 * Supports click on any step, pointer drag across the track, keyboard navigation,
 * and clean EOStage light + dark theme styling.
 */

import React, { useCallback, useRef } from "react";
import { t as translateCurrent, useI18n } from "@/i18n";
import { cn } from "@/lib/utils";
import { ctrlShortcut } from "@/lib/platformShortcuts";
import type { AgentReasoningLevel } from "@/lib/generated/v3-contracts";

function formatReasoningLevelWith(
  level: string | null | undefined,
  translateFn: typeof translateCurrent,
): string {
  const key = (level || "").trim().toLowerCase();
  if (!key) return translateFn("agent.modelPicker.reasoningDefault");
  if (key === "off" || key === "none" || key === "disabled") {
    return translateFn("agent.modelPicker.reasoningOff");
  }
  if (key === "minimal" || key === "min") {
    return translateFn("agent.modelPicker.reasoningMinimal");
  }
  if (key === "low") return translateFn("agent.modelPicker.reasoningLow");
  if (key === "medium" || key === "med" || key === "balanced") {
    return translateFn("agent.modelPicker.reasoningMedium");
  }
  if (key === "high") return translateFn("agent.modelPicker.reasoningHigh");
  if (key === "xhigh" || key === "max" || key === "very_high") {
    return translateFn("agent.modelPicker.reasoningXHigh");
  }
  if (/^\d+$/.test(key)) return `${key} tok`;
  return level?.trim() || translateFn("agent.modelPicker.reasoningDefault");
}

export function formatReasoningLevelZh(level: string | null | undefined): string {
  return formatReasoningLevelWith(level, translateCurrent);
}

export interface ReasoningIntensitySliderProps {
  modelDisplayName: string;
  levels: readonly AgentReasoningLevel[];
  activeLevel: string | null | undefined;
  onSelectLevel: (level: string) => void;
  onCycle?: () => void;
  disabled?: boolean;
}

export function ReasoningIntensitySlider({
  modelDisplayName,
  levels,
  activeLevel,
  onSelectLevel,
  onCycle,
  disabled = false,
}: ReasoningIntensitySliderProps) {
  const { t } = useI18n();
  const trackRef = useRef<HTMLDivElement | null>(null);
  const draggingRef = useRef<boolean>(false);
  const didDragMoveRef = useRef<boolean>(false);
  const lastEmittedLevelRef = useRef<string | null>(null);

  const hasLevels = levels.length > 0;
  const effectiveDisabled = disabled || !hasLevels;

  const resolvedIndex = (() => {
    if (!hasLevels) return 0;
    const idx = levels.findIndex((item) => item.level === activeLevel);
    if (idx >= 0) return idx;
    const mediumIdx = levels.findIndex((item) => item.level.toLowerCase() === "medium");
    if (mediumIdx >= 0) return mediumIdx;
    return 0;
  })();

  const currentLevelObj = hasLevels ? levels[resolvedIndex] : null;
  const currentLabelZh = hasLevels
    ? formatReasoningLevelWith(activeLevel ?? currentLevelObj?.level, t)
    : t("agent.modelPicker.reasoningDefault");

  const stepCount = Math.max(levels.length, 1);
  // Position stops cleanly from 9% to 91% (or centered when single stop) so the white circular thumb sits inside the capsule pill cleanly.
  const stopPercentForIndex = (idx: number): number => {
    if (stepCount <= 1) return 50;
    const inset = 9;
    return inset + (idx / (stepCount - 1)) * (100 - inset * 2);
  };

  const activePercent = hasLevels ? stopPercentForIndex(resolvedIndex) : 0;

  const resolveIndexFromClientX = useCallback(
    (clientX: number): number | null => {
      const el = trackRef.current;
      if (!el || levels.length <= 1) return levels.length === 1 ? 0 : null;
      const rect = el.getBoundingClientRect();
      if (rect.width <= 0) return null;
      const ratio = Math.min(1, Math.max(0, (clientX - rect.left) / rect.width));
      return Math.round(ratio * (levels.length - 1));
    },
    [levels.length]
  );

  const handleTrackPointerDown = (e: React.PointerEvent<HTMLDivElement>) => {
    if (effectiveDisabled || e.button !== 0) return;
    e.preventDefault();
    draggingRef.current = true;
    didDragMoveRef.current = false;
    lastEmittedLevelRef.current = activeLevel ?? currentLevelObj?.level ?? null;
    try {
      e.currentTarget.setPointerCapture(e.pointerId);
    } catch {
      // JSDOM may not support setPointerCapture
    }
    const nextIdx = resolveIndexFromClientX(e.clientX);
    if (nextIdx !== null) {
      const nextLevel = levels[nextIdx]?.level;
      if (nextLevel && nextLevel !== lastEmittedLevelRef.current) {
        lastEmittedLevelRef.current = nextLevel;
        onSelectLevel(nextLevel);
      }
    }
  };

  const handleTrackPointerMove = (e: React.PointerEvent<HTMLDivElement>) => {
    if (!draggingRef.current || effectiveDisabled) return;
    const nextIdx = resolveIndexFromClientX(e.clientX);
    if (nextIdx === null) return;
    didDragMoveRef.current = true;
    const nextLevel = levels[nextIdx]?.level;
    if (nextLevel && nextLevel !== lastEmittedLevelRef.current) {
      lastEmittedLevelRef.current = nextLevel;
      onSelectLevel(nextLevel);
    }
  };

  const handleTrackPointerUp = (e: React.PointerEvent<HTMLDivElement>) => {
    if (!draggingRef.current) return;
    draggingRef.current = false;
    try {
      e.currentTarget.releasePointerCapture(e.pointerId);
    } catch {
      // ignore
    }
  };

  return (
    <div
      data-testid="reasoning-intensity-slider"
      data-disabled={effectiveDisabled ? "true" : "false"}
      className="border-t border-sp-divider px-3 py-2.5 select-none"
    >
      <div className="mb-2 flex items-center justify-between gap-2">
        <div className="flex items-center gap-1.5 min-w-0">
          <span className="text-xs text-sp-text-muted whitespace-nowrap">
            {t("agent.modelPicker.reasoningTitle")}
          </span>
          <span
            data-testid="reasoning-intensity-label"
            className="text-xs font-semibold text-sp-text truncate"
          >
            {hasLevels
              ? currentLabelZh
              : t("agent.modelPicker.reasoningDefaultUnconfigured")}
          </span>
        </div>
        <button
          type="button"
          data-testid="reasoning-intensity-shortcut-btn"
          disabled={effectiveDisabled}
          onClick={() => {
            if (!effectiveDisabled) onCycle?.();
          }}
          title={
            effectiveDisabled
              ? t("agent.modelPicker.reasoningUnconfiguredTitle")
              : t("agent.modelPicker.reasoningCycleTitle", {
                  shortcut: ctrlShortcut("T"),
                })
          }
          className={cn(
            "inline-flex items-center rounded-[5px] border border-sp-line bg-sp-surface-muted px-1.5 py-0.5 font-mono text-xs leading-none text-sp-text-muted transition-colors",
            !effectiveDisabled
              ? "hover:bg-sp-hover hover:text-sp-text cursor-pointer"
              : "opacity-50 cursor-not-allowed"
          )}
        >
          {ctrlShortcut("T")}
        </button>
      </div>

      {/* Horizontal Capsule Track matching EOStage Slider */}
      <div
        ref={trackRef}
        data-testid="reasoning-intensity-track"
        onPointerDown={handleTrackPointerDown}
        onPointerMove={handleTrackPointerMove}
        onPointerUp={handleTrackPointerUp}
        onPointerCancel={handleTrackPointerUp}
        className={cn(
          "relative flex h-6 w-full items-center rounded-full border border-sp-line/70 bg-sp-surface-muted dark:bg-[#2a2a30] transition-colors",
          effectiveDisabled ? "opacity-45 cursor-not-allowed" : "cursor-pointer"
        )}
      >
        {/* Active Emerald Fill Bar */}
        {hasLevels && (
          <div
            aria-hidden="true"
            data-testid="reasoning-intensity-fill"
            className={cn(
              "pointer-events-none absolute inset-y-0 left-0 rounded-full transition-[width] duration-fast ease-sp",
              currentLevelObj?.level.toLowerCase() === "off"
                ? "bg-sp-text-muted/35"
                : "bg-emerald-500 dark:bg-emerald-500/90"
            )}
            style={{
              width:
                currentLevelObj?.level.toLowerCase() === "off" && resolvedIndex === 0
                  ? "24px"
                  : `calc(${activePercent}% + 10px)`,
              maxWidth: "100%",
            }}
          />
        )}

        {/* Interactive Step Stops (also exposed as menuitemradio for keyboard & accessibility compatibility) */}
        {hasLevels ? (
          levels.map((level, idx) => {
            const isSelected = resolvedIndex === idx;
            const leftPct = stopPercentForIndex(idx);
            const isFilled = idx <= resolvedIndex && currentLevelObj?.level.toLowerCase() !== "off";
            return (
              <div
                key={level.level}
                role="menuitemradio"
                aria-checked={isSelected}
                aria-label={`${modelDisplayName} ${level.level}`}
                data-testid={`reasoning-level-step-${level.level}`}
                tabIndex={0}
                title={
                  level.value
                    ? `${formatReasoningLevelWith(level.level, t)} (${level.value})`
                    : formatReasoningLevelWith(level.level, t)
                }
                onClick={(e) => {
                  e.stopPropagation();
                  if (didDragMoveRef.current) {
                    didDragMoveRef.current = false;
                    return;
                  }
                  onSelectLevel(level.level);
                }}
                onKeyDown={(e) => {
                  if (e.key === "Enter" || e.key === " ") {
                    e.preventDefault();
                    e.stopPropagation();
                    onSelectLevel(level.level);
                  } else if (e.key === "ArrowRight") {
                    e.preventDefault();
                    e.stopPropagation();
                    const next = levels[Math.min(levels.length - 1, idx + 1)];
                    if (next) onSelectLevel(next.level);
                  } else if (e.key === "ArrowLeft") {
                    e.preventDefault();
                    e.stopPropagation();
                    const prev = levels[Math.max(0, idx - 1)];
                    if (prev) onSelectLevel(prev.level);
                  }
                }}
                style={{ left: `${leftPct}%` }}
                className="absolute top-1/2 z-10 flex h-6 w-6 -translate-x-1/2 -translate-y-1/2 items-center justify-center rounded-full outline-none focus-visible:ring-2 focus-visible:ring-sp-accent"
              >
                {isSelected ? (
                  <span
                    aria-hidden="true"
                    data-testid="reasoning-intensity-thumb"
                    className="h-[22px] w-[22px] rounded-full border border-black/15 bg-white shadow-[0_1px_4px_rgba(0,0,0,0.28)] dark:border-white/25"
                  />
                ) : (
                  <span
                    aria-hidden="true"
                    className={cn(
                      "h-1.5 w-1.5 rounded-full transition-colors",
                      isFilled
                        ? "bg-white/75"
                        : "bg-sp-text-muted/50 dark:bg-white/35"
                    )}
                  />
                )}
              </div>
            );
          })
        ) : (
          <div className="flex w-full items-center justify-between px-3">
            {[0, 1, 2, 3].map((dot) => (
              <span
                key={dot}
                aria-hidden="true"
                className="h-1.5 w-1.5 rounded-full bg-sp-text-muted/35"
              />
            ))}
          </div>
        )}
      </div>
    </div>
  );
}
