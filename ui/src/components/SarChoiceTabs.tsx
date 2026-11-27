import type { KeyboardEvent } from "react";

import { cn } from "@/lib/utils";

export type SarChoiceTab = {
  value: string;
  label: string;
};

type SarChoiceTabsProps = {
  label: string;
  value: string;
  choices: SarChoiceTab[];
  onChange?: (value: string) => void;
  className?: string;
};

/**
 * Displays the legal values for one SAR product field. A single legal value is
 * still rendered as the selected product truth; adding legal values later
 * turns the same model into a keyboard-selectable tab set.
 */
export function SarChoiceTabs({ label, value, choices, onChange, className }: SarChoiceTabsProps) {
  const selectedValue = choices.some((choice) => choice.value === value)
    ? value
    : choices[0]?.value ?? "";
  const canSwitch = choices.length > 1 && Boolean(onChange);

  const select = (next: string) => {
    if (canSwitch && next !== selectedValue) onChange?.(next);
  };

  const onTabKeyDown = (event: KeyboardEvent<HTMLButtonElement>, index: number) => {
    if (!canSwitch || !["ArrowRight", "ArrowLeft", "Home", "End"].includes(event.key)) return;
    event.preventDefault();
    const nextIndex = event.key === "Home"
      ? 0
      : event.key === "End"
        ? choices.length - 1
        : event.key === "ArrowRight"
          ? (index + 1) % choices.length
          : (index - 1 + choices.length) % choices.length;
    const nextChoice = choices[nextIndex];
    select(nextChoice.value);
    const tabs = event.currentTarget.parentElement?.querySelectorAll<HTMLButtonElement>("[role='tab']");
    tabs?.item(nextIndex)?.focus();
  };

  if (!choices.length) return null;

  return (
    <div className={cn("min-w-0", className)} data-sar-choice-tabs={label}>
      <span className="mb-0.5 block text-[10px] font-medium leading-4 text-muted-foreground">{label}</span>
      <div role="tablist" aria-label={label} aria-orientation="horizontal" className="flex min-w-0 gap-1">
        {choices.map((choice, index) => {
          const selected = choice.value === selectedValue;
          if (!canSwitch) {
            return (
              <span
                key={choice.value}
                role="tab"
                aria-selected={selected}
                data-sar-choice-fixed
                className="h-8 min-w-0 flex-1 rounded-md border border-primary bg-primary px-2 text-left text-xs font-medium leading-8 text-primary-foreground shadow-sm"
              >
                <span className="block truncate" title={choice.label}>{choice.label}</span>
              </span>
            );
          }
          return (
            <button
              key={choice.value}
              type="button"
              role="tab"
              aria-selected={selected}
              tabIndex={selected ? 0 : -1}
              onClick={() => select(choice.value)}
              onKeyDown={(event) => onTabKeyDown(event, index)}
              className={cn(
                "h-8 min-w-0 flex-1 rounded-md border px-2 text-left text-xs font-medium transition-colors focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring",
                selected
                  ? "border-primary bg-primary text-primary-foreground shadow-sm"
                  : "border-border/70 bg-card text-foreground hover:border-primary/45 hover:bg-muted/65",
              )}
            >
              <span className="block truncate" title={choice.label}>{choice.label}</span>
            </button>
          );
        })}
      </div>
    </div>
  );
}
