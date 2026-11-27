import {
  useEffect,
  useMemo,
  useRef,
  useState,
  type KeyboardEvent,
} from "react";
import { CalendarDays, ChevronLeft, ChevronRight, X } from "lucide-react";

import { cn } from "@/lib/utils";

type DateRange = { start: string; end: string };

const WEEKDAYS = ["一", "二", "三", "四", "五", "六", "日"];

function formatDate(date: Date): string {
  return [
    date.getUTCFullYear(),
    String(date.getUTCMonth() + 1).padStart(2, "0"),
    String(date.getUTCDate()).padStart(2, "0"),
  ].join("-");
}

function parseDate(value: string): Date | null {
  const match = /^(\d{4})-(\d{2})-(\d{2})$/.exec(value.trim());
  if (!match) return null;
  const date = new Date(Date.UTC(Number(match[1]), Number(match[2]) - 1, Number(match[3])));
  return formatDate(date) === value.trim() ? date : null;
}

function subtractMonths(date: Date, months: number): Date {
  const targetMonth = date.getUTCMonth() - months;
  const targetYear = date.getUTCFullYear() + Math.floor(targetMonth / 12);
  const normalizedMonth = ((targetMonth % 12) + 12) % 12;
  const lastDay = new Date(Date.UTC(targetYear, normalizedMonth + 1, 0)).getUTCDate();
  return new Date(Date.UTC(targetYear, normalizedMonth, Math.min(date.getUTCDate(), lastDay)));
}

function rangeSummary(start: string, end: string): string {
  const display = (value: string) => value.replace(/-/g, "/");
  if (!start && !end) return "全部时间";
  if (start && end) return `${display(start)} — ${display(end)}`;
  if (start) return `${display(start)} 起`;
  return `截至 ${display(end)}`;
}

export function FilterDateRange({
  start,
  end,
  onChange,
  disabled = false,
  now = new Date(),
}: {
  start: string;
  end: string;
  onChange(next: DateRange): void;
  disabled?: boolean;
  now?: Date;
}) {
  const [open, setOpen] = useState(false);
  const initialMonth = parseDate(start) ?? parseDate(end) ?? now;
  const [visibleMonth, setVisibleMonth] = useState(
    () => new Date(Date.UTC(initialMonth.getUTCFullYear(), initialMonth.getUTCMonth(), 1)),
  );
  const rootRef = useRef<HTMLDivElement>(null);
  const triggerRef = useRef<HTMLButtonElement>(null);

  useEffect(() => {
    if (!open) return;
    const closeOutside = (event: PointerEvent) => {
      if (event.target instanceof Node && rootRef.current?.contains(event.target)) return;
      setOpen(false);
    };
    document.addEventListener("pointerdown", closeOutside, true);
    return () => document.removeEventListener("pointerdown", closeOutside, true);
  }, [open]);

  useEffect(() => {
    if (disabled) setOpen(false);
  }, [disabled]);

  const days = useMemo(() => {
    const year = visibleMonth.getUTCFullYear();
    const month = visibleMonth.getUTCMonth();
    const count = new Date(Date.UTC(year, month + 1, 0)).getUTCDate();
    const mondayOffset = (new Date(Date.UTC(year, month, 1)).getUTCDay() + 6) % 7;
    return [
      ...Array.from({ length: mondayOffset }, () => null),
      ...Array.from({ length: count }, (_, index) => new Date(Date.UTC(year, month, index + 1))),
    ];
  }, [visibleMonth]);

  const applyQuickRange = (months: number | null) => {
    if (months === null) {
      onChange({ start: "", end: "" });
    } else {
      onChange({ start: formatDate(subtractMonths(now, months)), end: formatDate(now) });
    }
    setOpen(false);
  };

  const chooseDay = (date: Date) => {
    const value = formatDate(date);
    if (!start || end) {
      onChange({ start: value, end: "" });
    } else if (value < start) {
      onChange({ start: value, end: start });
    } else {
      onChange({ start, end: value });
    }
  };

  const handleKeyDown = (event: KeyboardEvent<HTMLDivElement>) => {
    if (event.key !== "Escape") return;
    event.preventDefault();
    setOpen(false);
    triggerRef.current?.focus();
  };

  return (
    <div ref={rootRef} className="relative" data-filter-date-range>
      <button
        ref={triggerRef}
        type="button"
        disabled={disabled}
        onClick={() => setOpen((current) => !current)}
        className="flex h-8 w-full items-center justify-between gap-2 rounded-lg border border-input bg-card px-2.5 text-left text-xs text-foreground shadow-sm transition hover:bg-muted/65 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring disabled:cursor-not-allowed disabled:bg-muted disabled:text-muted-foreground"
        aria-label={`日期范围：${rangeSummary(start, end)}`}
        aria-haspopup="dialog"
        aria-expanded={open}
      >
        <span className="truncate">{rangeSummary(start, end)}</span>
        <CalendarDays className="h-3.5 w-3.5 shrink-0 text-muted-foreground" />
      </button>
      {open && !disabled && (
        <div
          role="dialog"
          aria-label="选择日期范围"
          onKeyDown={handleKeyDown}
          className="absolute left-0 top-[calc(100%+0.3rem)] z-[950] w-[19rem] rounded-xl border border-border/80 bg-popover p-3 text-xs text-popover-foreground shadow-xl"
        >
          <div className="grid grid-cols-3 gap-1.5">
            <button type="button" onClick={() => applyQuickRange(null)} className="rounded-md bg-muted px-2 py-1.5 hover:bg-muted/75">全部时间</button>
            {[
              [1, "最近 1 个月"],
              [3, "最近 3 个月"],
              [12, "最近 1 年"],
              [36, "最近 3 年"],
            ].map(([months, label]) => (
              <button
                key={label}
                type="button"
                onClick={() => applyQuickRange(months as number)}
                className="rounded-md bg-muted px-2 py-1.5 hover:bg-muted/75"
              >
                {label}
              </button>
            ))}
          </div>
          <div className="mt-3 grid grid-cols-2 gap-2">
            <div>
              <label htmlFor="filter-date-range-start" className="mb-1 block text-[10px] font-medium text-foreground">
                开始
              </label>
              <div className="relative">
              <input
                id="filter-date-range-start"
                aria-label="开始日期"
                type="text"
                inputMode="numeric"
                value={start}
                placeholder="yyyy-mm-dd"
                onChange={(event) => onChange({ start: event.currentTarget.value, end })}
                className="h-9 w-full cursor-text rounded-md border border-border bg-background px-2 pr-8 text-xs shadow-sm outline-none transition placeholder:text-muted-foreground/75 hover:border-foreground/35 focus:border-primary focus:ring-2 focus:ring-ring"
              />
                {start && (
                  <button
                    type="button"
                    aria-label="清空开始日期"
                    onClick={() => onChange({ start: "", end })}
                    className="absolute right-1 top-1/2 -translate-y-1/2 rounded p-1 text-muted-foreground hover:bg-muted hover:text-foreground"
                  >
                    <X className="h-3.5 w-3.5" />
                  </button>
                )}
              </div>
            </div>
            <div>
              <label htmlFor="filter-date-range-end" className="mb-1 block text-[10px] font-medium text-foreground">
                结束
              </label>
              <div className="relative">
              <input
                id="filter-date-range-end"
                aria-label="结束日期"
                type="text"
                inputMode="numeric"
                value={end}
                placeholder="yyyy-mm-dd"
                onChange={(event) => onChange({ start, end: event.currentTarget.value })}
                className="h-9 w-full cursor-text rounded-md border border-border bg-background px-2 pr-8 text-xs shadow-sm outline-none transition placeholder:text-muted-foreground/75 hover:border-foreground/35 focus:border-primary focus:ring-2 focus:ring-ring"
              />
                {end && (
                  <button
                    type="button"
                    aria-label="清空结束日期"
                    onClick={() => onChange({ start, end: "" })}
                    className="absolute right-1 top-1/2 -translate-y-1/2 rounded p-1 text-muted-foreground hover:bg-muted hover:text-foreground"
                  >
                    <X className="h-3.5 w-3.5" />
                  </button>
                )}
              </div>
            </div>
          </div>
          <button
            type="button"
            onClick={() => onChange({ start: "", end: "" })}
            className="mt-2 rounded-md border border-border bg-background px-2.5 py-1.5 text-[11px] font-medium text-foreground shadow-sm transition hover:bg-muted"
          >
            清空日期
          </button>
          <div className="mt-3 flex items-center justify-between">
            <button
              type="button"
              aria-label="上个月"
              onClick={() => setVisibleMonth((current) => new Date(Date.UTC(current.getUTCFullYear(), current.getUTCMonth() - 1, 1)))}
              className="rounded-md p-1 hover:bg-muted"
            >
              <ChevronLeft className="h-4 w-4" />
            </button>
            <div className="font-medium">
              {visibleMonth.getUTCFullYear()} 年 {visibleMonth.getUTCMonth() + 1} 月
            </div>
            <button
              type="button"
              aria-label="下个月"
              onClick={() => setVisibleMonth((current) => new Date(Date.UTC(current.getUTCFullYear(), current.getUTCMonth() + 1, 1)))}
              className="rounded-md p-1 hover:bg-muted"
            >
              <ChevronRight className="h-4 w-4" />
            </button>
          </div>
          <div className="mt-1 grid grid-cols-7 gap-0.5 text-center">
            {WEEKDAYS.map((weekday) => <div key={weekday} className="py-1 text-[10px] text-muted-foreground">{weekday}</div>)}
            {days.map((date, index) => {
              if (!date) return <span key={`empty-${index}`} />;
              const value = formatDate(date);
              const selected = value === start || value === end;
              const inRange = Boolean(start && end && value > start && value < end);
              return (
                <button
                  key={value}
                  type="button"
                  aria-label={`选择 ${value}`}
                  onClick={() => chooseDay(date)}
                  className={cn(
                    "h-7 rounded-md transition hover:bg-muted",
                    selected && "bg-primary text-primary-foreground hover:bg-primary",
                    inRange && "bg-primary/10 text-primary",
                  )}
                >
                  {date.getUTCDate()}
                </button>
              );
            })}
          </div>
        </div>
      )}
    </div>
  );
}
