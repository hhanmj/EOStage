import React, { useEffect, useState } from "react";
import { AlertCircle, CheckCircle2 } from "lucide-react";
import { cn } from "@/lib/utils";

export type TopToastTone = "neutral" | "success" | "danger";

export interface TopToastEntry {
  id: string;
  text: string;
  tone: TopToastTone;
  createdAtMs: number;
  durationMs: number;
}

type Listener = (items: TopToastEntry[]) => void;

let currentToasts: TopToastEntry[] = [];
const listeners = new Set<Listener>();
const dismissTimers = new Map<string, ReturnType<typeof setTimeout>>();

function emit() {
  const snapshot = [...currentToasts];
  for (const fn of listeners) {
    try {
      fn(snapshot);
    } catch {
      // ignore listener errors
    }
  }
}

export function emitTopToast(input: {
  text: string;
  tone?: TopToastTone;
  durationMs?: number;
}): string {
  const id = `toast-${Date.now()}-${Math.random().toString(16).slice(2, 7)}`;
  const durationMs = input.durationMs ?? 3000;
  const entry: TopToastEntry = {
    id,
    text: input.text,
    tone: input.tone ?? "neutral",
    createdAtMs: Date.now(),
    durationMs,
  };
  // Keep at most 2 concise top-center notifications visible simultaneously.
  currentToasts = [entry, ...currentToasts].slice(0, 2);
  emit();

  if (durationMs > 0) {
    const timer = setTimeout(() => {
      dismissTimers.delete(id);
      currentToasts = currentToasts.filter((t) => t.id !== id);
      emit();
    }, durationMs);
    dismissTimers.set(id, timer);
  }
  return id;
}

export function clearAllTopToastsForTest(): void {
  for (const timer of dismissTimers.values()) {
    clearTimeout(timer);
  }
  dismissTimers.clear();
  currentToasts = [];
  emit();
}

export function subscribeTopToasts(listener: Listener): () => void {
  listeners.add(listener);
  listener([...currentToasts]);
  return () => {
    listeners.delete(listener);
  };
}

export function TopCenterToastHost() {
  const [toasts, setToasts] = useState<TopToastEntry[]>([]);

  useEffect(() => subscribeTopToasts(setToasts), []);

  if (toasts.length === 0) return null;

  return (
    <div
      data-testid="top-center-toast-host"
      aria-live="polite"
      aria-atomic="false"
      className="fixed bottom-6 left-1/2 -translate-x-1/2 z-[2600] flex flex-col items-center gap-1.5 pointer-events-none"
    >
      {toasts.map((toast) => (
        <div
          key={toast.id}
          role="status"
          data-testid="top-center-toast"
          className={cn(
            "sp-top-toast-enter inline-flex items-center gap-2 px-3 py-1.5 rounded-full border text-xs font-medium shadow-sp-overlay whitespace-nowrap",
            toast.tone === "danger"
              ? "border-sp-danger-text/30 bg-sp-danger-bg text-sp-danger-text"
              : toast.tone === "success"
                ? "border-sp-success-text/25 bg-sp-surface text-sp-text"
                : "border-sp-line bg-sp-surface text-sp-text"
          )}
        >
          {toast.tone === "danger" ? (
            <AlertCircle className="h-3.5 w-3.5 text-sp-danger-text shrink-0" />
          ) : (
            <CheckCircle2 className="h-3.5 w-3.5 text-sp-success-text shrink-0" />
          )}
          <span>{toast.text}</span>
        </div>
      ))}
    </div>
  );
}
