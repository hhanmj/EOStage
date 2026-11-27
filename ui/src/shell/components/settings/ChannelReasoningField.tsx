import React, { useId } from "react";
import { Info } from "lucide-react";
import { useI18n } from "@/i18n";
import { cn } from "@/lib/utils";
import { HoverHintLabel } from "@/components/desktop/HoverHintLabel.tsx";
import type { AgentReasoningMode } from "../../api/agentBridgeTypes.ts";

export interface ChannelReasoningFieldProps {
  value: AgentReasoningMode;
  onChange: (value: AgentReasoningMode) => void;
  disabled?: boolean;
  /** AgentChannelView.reasoning_switch_unsupported：主机已记下该渠道拒绝关闭思考 */
  switchUnsupported?: boolean;
}

export function ChannelReasoningField({
  value,
  onChange,
  disabled = false,
  switchUnsupported = false,
}: ChannelReasoningFieldProps) {
  const { t } = useI18n();
  const labelId = useId();
  const isOff = value === "off";
  const isDefault = value === "default";

  return (
    <div className="eo-settings-row-split">
      <div className="flex min-w-0 flex-col gap-1">
        <div className="flex items-center gap-2">
          <label id={labelId} className="text-[13px] font-normal text-sp-text">
            {t("settings.channels.reasoning")}
          </label>
          <HoverHintLabel hint={t("settings.channels.reasoningHint")} focusable>
            <Info
              className="h-3.5 w-3.5 text-sp-text-muted"
              aria-label={t("settings.channels.reasoningHintLabel")}
            />
          </HoverHintLabel>
          {switchUnsupported && (
            <HoverHintLabel hint={t("settings.channels.reasoningUnsupportedHint")} focusable>
              <Info
                className="h-3.5 w-3.5 text-sp-text-secondary"
                aria-label={t("settings.channels.reasoningUnsupportedLabel")}
              />
            </HoverHintLabel>
          )}
        </div>
        <span className="text-xs text-sp-text-secondary">
          控制支持推理的模型是否输出思维链过程
        </span>
      </div>
      <div
        role="radiogroup"
        aria-labelledby={labelId}
        className={cn(
          "eo-segmented-track shrink-0",
          disabled && "opacity-50 cursor-not-allowed"
        )}
      >
        <button
          type="button"
          role="radio"
          aria-checked={isOff}
          data-active={isOff}
          disabled={disabled}
          onClick={() => {
            if (!disabled) onChange("off");
          }}
          className={cn(
            "eo-segmented-pill px-3 text-[13px]",
            isOff ? "eo-segmented-pill-active" : "text-sp-text-secondary hover:text-sp-text"
          )}
        >
          {t("settings.channels.off")}
        </button>
        <button
          type="button"
          role="radio"
          aria-checked={isDefault}
          data-active={isDefault}
          disabled={disabled}
          onClick={() => {
            if (!disabled) onChange("default");
          }}
          className={cn(
            "eo-segmented-pill px-3 text-[13px]",
            isDefault ? "eo-segmented-pill-active" : "text-sp-text-secondary hover:text-sp-text"
          )}
        >
          {t("settings.channels.default")}
        </button>
      </div>
    </div>
  );
}
