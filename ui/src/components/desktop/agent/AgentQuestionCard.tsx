/**
 * AgentQuestionCard.tsx
 * Interactive clarification card for `ask_user_question` (`question_card` stream event).
 * Renders structured question prompt + clickable option chips (with optional recommended highlight).
 */

import React, { useState } from "react";
import { Check, HelpCircle } from "lucide-react";
import { cn } from "@/lib/utils";
import { useI18n } from "@/i18n";
import type { AgentQuestionOption } from "@/lib/generated/v3-contracts";

export function isPassiveDismissQuestionOption(
  value: string,
  labelZh?: string
): boolean {
  const haystack = `${value ?? ""} ${labelZh ?? ""}`.trim().toLowerCase();
  if (!haystack) return false;
  return (
    haystack.includes("暂不下载") ||
    haystack.includes("先不下载") ||
    haystack.includes("先保留检索结果") ||
    haystack.includes("保留检索结果") ||
    haystack.includes("仅保留") ||
    haystack.includes("稍后再说") ||
    /\b(skip|cancel|keep_only|no_download|dismiss)\b/.test(haystack) ||
    haystack === "取消"
  );
}

export interface AgentQuestionCardProps {
  questionZh: string;
  options: AgentQuestionOption[];
  runId?: string;
  selectedValue?: string | null;
  selectedValues?: string[];
  multiSelect?: boolean;
  variant?: "inline" | "docked";
  disabled?: boolean;
  dismissed?: boolean;
  onSelectOption?: (value: string, option: AgentQuestionOption) => void;
  onSubmitMulti?: (values: string[], options: AgentQuestionOption[]) => void;
  className?: string;
}

export function AgentQuestionCard({
  questionZh,
  options,
  selectedValue: controlledSelected,
  selectedValues: controlledMultiSelected,
  multiSelect = false,
  variant = "inline",
  disabled = false,
  dismissed = false,
  onSelectOption,
  onSubmitMulti,
  className,
}: AgentQuestionCardProps) {
  const { t } = useI18n();
  const [internalSelected, setInternalSelected] = useState<string | null>(null);
  const [internalMulti, setInternalMulti] = useState<string[]>([]);
  const effectiveSelected =
    controlledSelected !== undefined ? controlledSelected : internalSelected;
  const effectiveMulti =
    controlledMultiSelected !== undefined ? controlledMultiSelected : internalMulti;
  const hasAnySelection = multiSelect
    ? effectiveMulti.length > 0
    : Boolean(effectiveSelected && effectiveSelected.trim().length > 0) || dismissed;

  const handleSelect = (opt: AgentQuestionOption) => {
    if (disabled) return;
    if (multiSelect) {
      setInternalMulti((prev) =>
        prev.includes(opt.value)
          ? prev.filter((item) => item !== opt.value)
          : [...prev, opt.value]
      );
      return;
    }
    setInternalSelected(opt.value);
    onSelectOption?.(opt.value, opt);
  };

  const handleConfirmMulti = () => {
    if (disabled || effectiveMulti.length === 0) return;
    const chosen = options.filter((opt) => effectiveMulti.includes(opt.value));
    onSubmitMulti?.(effectiveMulti, chosen);
  };

  const isDocked = variant === "docked";

  return (
    <div
      data-testid="agent-question-card"
      data-variant={variant}
      data-dismissed={dismissed ? "true" : "false"}
      className={cn(
        "border bg-sp-card flex flex-col w-full",
        isDocked
          ? "sp-popover-spring-in rounded-t-xl rounded-b-lg border-sp-accent/35 bg-sp-surface/95 backdrop-blur-sm px-3.5 py-3 gap-2.5 shadow-overlay"
          : "rounded-xl border-sp-line shadow-card p-3.5 gap-3",
        className
      )}
    >
      <div className="flex items-start gap-2.5">
        <HelpCircle className="h-4 w-4 text-sp-accent shrink-0 mt-0.5" />
        <div className="flex-1 min-w-0 flex flex-col gap-1">
          <div className="flex items-center justify-between gap-2">
            <span className="text-xs font-semibold text-sp-accent select-none">
              {t("agent.question.badge")}
            </span>
            {dismissed ? (
              <span
                data-testid="agent-question-settled-badge"
                className="text-xs text-sp-text-muted select-none"
              >
                已保留结果
              </span>
            ) : multiSelect ? (
              <span className="text-xs text-sp-text-muted select-none">
                可多选
              </span>
            ) : isDocked ? (
              <span className="text-xs text-sp-text-muted select-none">
                点击选项确认
              </span>
            ) : null}
          </div>
          <div
            data-testid="agent-question-prompt"
            className="text-sm leading-[1.55] text-sp-text select-text break-words"
          >
            {questionZh}
          </div>
        </div>
      </div>

      {options.length > 0 && (
        <div
          data-testid="agent-question-options"
          className={cn(
            "flex flex-wrap items-center gap-2",
            isDocked ? "pl-0.5" : "pl-6"
          )}
        >
          {options.map((opt, idx) => {
            const isSelected = multiSelect
              ? effectiveMulti.includes(opt.value)
              : effectiveSelected === opt.value;
            return (
              <button
                key={`${opt.value}-${idx}`}
                type="button"
                data-testid={`agent-question-option-${idx}`}
                data-option-value={opt.value}
                aria-pressed={isSelected}
                disabled={disabled}
                onClick={() => handleSelect(opt)}
                className={cn(
                  "cds-btn-squish inline-flex items-center gap-1.5 border px-3 py-1.5 text-xs font-medium transition-colors duration-fast ease-sp cursor-pointer",
                  isDocked ? "rounded-lg" : "rounded-full",
                  isSelected
                    ? "border-sp-accent bg-sp-accent text-sp-canvas"
                    : !hasAnySelection && opt.recommended
                      ? "border-sp-accent bg-sp-surface-muted text-sp-text hover:bg-sp-hover"
                      : "border-sp-line bg-sp-surface-muted text-sp-text-secondary hover:bg-sp-hover hover:text-sp-text",
                  disabled && "opacity-55 cursor-not-allowed"
                )}
              >
                <span
                  aria-hidden="true"
                  className={cn(
                    "inline-flex h-3.5 w-3.5 shrink-0 items-center justify-center border",
                    multiSelect ? "rounded-[4px]" : "rounded-full",
                    isSelected
                      ? "border-current bg-current/15"
                      : "border-sp-line-strong"
                  )}
                >
                  {isSelected && <Check className="h-2.5 w-2.5 shrink-0" />}
                </span>
                <span>{opt.label_zh}</span>
                {!hasAnySelection && opt.recommended && !isSelected && (
                  <span className="rounded-full bg-sp-hover px-1.5 py-0.5 text-xs text-sp-accent font-semibold">
                    {t("agent.question.recommended")}
                  </span>
                )}
              </button>
            );
          })}

          {multiSelect && (
            <button
              type="button"
              data-testid="agent-question-submit-multi"
              disabled={disabled || effectiveMulti.length === 0}
              onClick={handleConfirmMulti}
              className="ml-auto inline-flex items-center gap-1 rounded-lg bg-sp-accent-fill px-3 py-1.5 text-xs font-medium text-white hover:opacity-95 disabled:opacity-45 cursor-pointer disabled:cursor-not-allowed"
            >
              确认提交（{effectiveMulti.length}）
            </button>
          )}
        </div>
      )}
    </div>
  );
}
