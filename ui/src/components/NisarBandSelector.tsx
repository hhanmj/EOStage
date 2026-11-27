import { useRef, type KeyboardEvent } from "react";
import type { NisarBand } from "@/lib/sarSourceCapabilities";
import { cn } from "@/lib/utils";
export type NisarSBandState = "available" | "integrating" | "blocked";

export type NisarBandSelectorProps = {
  value: NisarBand;
  sBandState: NisarSBandState;
  disabled?: boolean;
  onChange: (value: NisarBand) => void;
};

const OPTIONS: Array<{ value: NisarBand; label: string }> = [
  { value: "L", label: "L-band · ASF/NASA" },
  { value: "S", label: "S-band · Bhoonidhi/NRSC" },
];

export function NisarBandSelector({
  value,
  sBandState,
  disabled = false,
  onChange,
}: NisarBandSelectorProps) {
  const sBandStatus = sBandState === "available"
    ? "可用"
    : sBandState === "integrating"
      ? "正在接入"
      : "暂不可用";
  const buttonRefs = useRef<Array<HTMLButtonElement | null>>([]);

  return (
    <div className="col-span-2 space-y-1.5" data-nisar-band-selector>
      <div className="flex items-center justify-between gap-2">
        <span className="text-[11px] font-medium text-muted-foreground">雷达波段</span>
        <span className="text-[10px] text-muted-foreground">
          S-band {sBandStatus}
        </span>
      </div>
      <div
        role="radiogroup"
        aria-label="NISAR 雷达波段"
        className="grid grid-cols-2 gap-1 rounded-lg border border-border/70 bg-muted/25 p-1"
      >
        {OPTIONS.map((option, index) => {
          const selected = value === option.value;
          const optionDisabled = disabled || (option.value === "S" && sBandState !== "available");
          const handleKeyDown = (event: KeyboardEvent<HTMLButtonElement>) => {
            if (optionDisabled) return;
            const direction = event.key === "ArrowRight" || event.key === "ArrowDown" ? 1
              : event.key === "ArrowLeft" || event.key === "ArrowUp" ? -1
                : event.key === "Home" ? -OPTIONS.length
                  : event.key === "End" ? OPTIONS.length
                    : 0;
            if (!direction) return;
            event.preventDefault();
            const nextIndex = direction < -1 ? 0 : direction > 1 ? OPTIONS.length - 1 : (index + direction + OPTIONS.length) % OPTIONS.length;
            const next = OPTIONS[nextIndex];
            if (!next || (next.value === "S" && sBandState !== "available")) return;
            onChange(next.value);
            buttonRefs.current[nextIndex]?.focus();
          };
          const status = option.value === "S" && sBandState !== "available" ? ` · ${sBandStatus}` : "";
          return (
            <button
              key={option.value}
              ref={(element) => { buttonRefs.current[index] = element; }}
              type="button"
              role="radio"
              aria-checked={selected}
              aria-label={`${option.label}${status}`}
              tabIndex={selected ? 0 : -1}
              disabled={optionDisabled}
              onClick={() => onChange(option.value)}
              onKeyDown={handleKeyDown}
              className={cn(
                "min-h-9 rounded-md border px-2 text-left text-xs font-medium transition-colors",
                selected
                  ? "border-primary bg-primary text-primary-foreground shadow-sm"
                  : "border-transparent bg-card text-foreground hover:border-primary/45 hover:bg-muted/65",
                optionDisabled && "cursor-not-allowed opacity-55",
              )}
            >
              <span className="block">{option.label}</span>
              {option.value === "S" && sBandState !== "available" && (
                <span className={cn("block text-[10px]", selected ? "text-primary-foreground/80" : "text-muted-foreground")}>{sBandStatus}</span>
              )}
            </button>
          );
        })}
      </div>
      <p className="text-[10px] leading-4 text-muted-foreground">
        切换后已保留范围和日期，已清除不兼容条件、旧结果和旧选择。
      </p>
    </div>
  );
}
