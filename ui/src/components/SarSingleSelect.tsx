import {
  useEffect,
  useRef,
  useState,
  type KeyboardEvent,
} from "react";
import { CheckCircle2, ChevronDown } from "lucide-react";

import { cn } from "@/lib/utils";

export type SarSingleSelectOption = {
  value: string;
  label: string;
};

export function SarSingleSelect({
  ariaLabel,
  value,
  options,
  allLabel,
  disabled,
  onChange,
  includeAllOption = true,
  open: controlledOpen,
  onOpenChange,
}: {
  ariaLabel: string;
  value: string | null;
  options: SarSingleSelectOption[];
  allLabel: string;
  disabled: boolean;
  onChange(value: string | null): void;
  includeAllOption?: boolean;
  open?: boolean;
  onOpenChange?(open: boolean): void;
}) {
  const [internalOpen, setInternalOpen] = useState(false);
  const rootRef = useRef<HTMLDivElement>(null);
  const triggerRef = useRef<HTMLButtonElement>(null);
  const open = controlledOpen ?? internalOpen;
  const selected = options.find((option) => option.value === value) ?? null;
  const summary = selected?.label ?? allLabel;

  const setOpen = (next: boolean) => {
    if (controlledOpen === undefined) setInternalOpen(next);
    onOpenChange?.(next);
  };

  useEffect(() => {
    if (!open) return;
    const closeOutside = (event: PointerEvent) => {
      if (event.target instanceof Node && rootRef.current?.contains(event.target)) return;
      setOpen(false);
    };
    document.addEventListener("pointerdown", closeOutside, true);
    return () => document.removeEventListener("pointerdown", closeOutside, true);
  }, [open, controlledOpen, onOpenChange]);

  useEffect(() => {
    if (!open) return;
    const optionsInMenu = rootRef.current?.querySelectorAll<HTMLButtonElement>("[role='option']");
    const selectedIndex = selected
      ? options.findIndex((option) => option.value === selected.value) + (includeAllOption ? 1 : 0)
      : 0;
    optionsInMenu?.item(Math.max(0, selectedIndex)).focus();
  }, [open, options, selected]);

  useEffect(() => {
    if (disabled && open) setOpen(false);
  }, [disabled, open]);

  const select = (next: string | null) => {
    onChange(next);
    setOpen(false);
  };

  const handleListKeyDown = (event: KeyboardEvent<HTMLDivElement>) => {
    if (event.key === "Escape") {
      event.preventDefault();
      setOpen(false);
      triggerRef.current?.focus();
      return;
    }
    if (!["ArrowDown", "ArrowUp", "Home", "End"].includes(event.key)) return;
    event.preventDefault();
    const menuOptions = Array.from(
      event.currentTarget.querySelectorAll<HTMLButtonElement>("[role='option']"),
    );
    if (menuOptions.length === 0) return;
    const currentIndex = Math.max(0, menuOptions.indexOf(document.activeElement as HTMLButtonElement));
    const nextIndex = event.key === "Home"
      ? 0
      : event.key === "End"
        ? menuOptions.length - 1
        : event.key === "ArrowDown"
          ? (currentIndex + 1) % menuOptions.length
          : (currentIndex - 1 + menuOptions.length) % menuOptions.length;
    menuOptions[nextIndex].focus();
  };

  return (
    <div ref={rootRef} className="relative" data-sar-filter-menu data-sar-single-select>
      <button
        ref={triggerRef}
        type="button"
        disabled={disabled || options.length === 0}
        onClick={() => setOpen(!open)}
        onKeyDown={(event) => {
          if (["ArrowDown", "Enter", " "].includes(event.key) && !open) {
            event.preventDefault();
            setOpen(true);
          }
        }}
        className={cn(
          "flex h-8 w-full items-center justify-between gap-2 rounded-md border border-input bg-card px-2.5 text-left text-xs text-foreground shadow-sm transition hover:bg-muted/65 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring disabled:cursor-not-allowed disabled:bg-muted disabled:text-muted-foreground",
        )}
        aria-label={`${ariaLabel}：${summary}`}
        aria-haspopup="listbox"
        aria-expanded={open}
      >
        <span className="min-w-0 truncate">{summary}</span>
        <ChevronDown className="h-3.5 w-3.5 shrink-0 text-foreground" />
      </button>
      {open && !disabled && (
        <div
          className="absolute left-0 right-0 top-[calc(100%+0.25rem)] z-[900] rounded-xl border border-border/80 bg-popover p-1.5 text-xs text-popover-foreground shadow-xl"
          role="listbox"
          aria-label={ariaLabel}
          onKeyDown={handleListKeyDown}
        >
          {includeAllOption && (
            <button
              type="button"
              role="option"
              aria-selected={value === null}
              onClick={() => select(null)}
              className={cn(
                "flex h-8 w-full items-center justify-between rounded-lg px-2.5 text-left transition hover:bg-muted",
                value === null ? "text-primary" : "text-foreground",
              )}
            >
              <span>{allLabel}</span>
              {value === null && <CheckCircle2 className="h-3.5 w-3.5" />}
            </button>
          )}
          {options.map((option) => {
            const checked = option.value === value;
            return (
              <button
                key={option.value}
                type="button"
                role="option"
                aria-selected={checked}
                onClick={() => select(option.value)}
                className={cn(
                  "flex h-8 w-full items-center justify-between rounded-lg px-2.5 text-left transition hover:bg-muted",
                  checked ? "text-primary" : "text-foreground",
                )}
              >
                <span>{option.label}</span>
                {checked && <CheckCircle2 className="h-3.5 w-3.5" />}
              </button>
            );
          })}
        </div>
      )}
    </div>
  );
}
