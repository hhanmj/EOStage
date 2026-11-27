import { cn } from "@/lib/utils";

export function Progress({
  value = 0,
  indeterminate = false,
  className,
}: {
  value?: number | null;
  indeterminate?: boolean;
  className?: string;
}) {
  const pct = Math.min(100, Math.max(0, value ?? 0));
  return (
    <div
      className={cn("h-2 w-full overflow-hidden rounded-full bg-muted", className)}
      role="progressbar"
      aria-valuemin={0}
      aria-valuemax={100}
      aria-valuenow={indeterminate ? undefined : pct}
      aria-label={indeterminate ? "进度总量待服务返回" : undefined}
    >
      <div
        className={cn(
          "h-full rounded-full bg-primary transition-[width] duration-200 motion-reduce:transition-none",
          indeterminate && "progress-indeterminate-bar w-1/4 motion-reduce:animate-pulse",
        )}
        style={indeterminate ? undefined : { width: `${pct}%` }}
      />
    </div>
  );
}
