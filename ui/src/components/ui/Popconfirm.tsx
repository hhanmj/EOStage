import React, { useEffect, useLayoutEffect, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { cn } from "@/lib/utils";

export interface PopconfirmProps {
  open: boolean;
  anchorEl?: HTMLElement | null;
  title: React.ReactNode;
  description?: React.ReactNode;
  error?: React.ReactNode;
  extra?: React.ReactNode;
  confirmLabel: string;
  cancelLabel?: string;
  tone?: "danger" | "primary";
  busy?: boolean;
  onConfirm: () => void;
  onCancel: () => void;
  align?: "end" | "start";
  role?: "dialog" | "alertdialog";
  ariaLabel?: string;
  testId?: string;
}

const POPOVER_WIDTH = 276;
const VIEWPORT_MARGIN = 12;
const ANCHOR_GAP = 6;

export function Popconfirm({
  open,
  anchorEl,
  title,
  description,
  error,
  extra,
  confirmLabel,
  cancelLabel = "取消",
  tone = "danger",
  busy = false,
  onConfirm,
  onCancel,
  align = "end",
  role = "alertdialog",
  ariaLabel,
  testId = "sp-popconfirm",
}: PopconfirmProps) {
  const bubbleRef = useRef<HTMLDivElement | null>(null);
  const cancelBtnRef = useRef<HTMLButtonElement | null>(null);
  const returnFocusRef = useRef<HTMLElement | null>(null);
  const onCancelRef = useRef(onCancel);
  const busyRef = useRef(busy);
  onCancelRef.current = onCancel;
  busyRef.current = busy;
  const [coords, setCoords] = useState<{ top: number; left: number } | null>(null);

  const updatePosition = () => {
    if (!open || typeof window === "undefined") return;
    if (!anchorEl || !anchorEl.isConnected) {
      setCoords(null);
      return;
    }
    const rect = anchorEl.getBoundingClientRect();
    const bubbleHeight = bubbleRef.current?.offsetHeight || 132;
    const bubbleWidth = bubbleRef.current?.offsetWidth || POPOVER_WIDTH;
    const vw = window.innerWidth || 1280;
    const vh = window.innerHeight || 800;

    if (rect.width === 0 && rect.height === 0 && rect.top === 0 && rect.left === 0) {
      setCoords({ top: 16, left: 16 });
      return;
    }

    let left = align === "end" ? rect.right - bubbleWidth : rect.left;
    left = Math.max(VIEWPORT_MARGIN, Math.min(left, vw - bubbleWidth - VIEWPORT_MARGIN));

    let top = rect.bottom + ANCHOR_GAP;
    if (top + bubbleHeight > vh - VIEWPORT_MARGIN && rect.top - ANCHOR_GAP - bubbleHeight >= VIEWPORT_MARGIN) {
      top = rect.top - ANCHOR_GAP - bubbleHeight;
    } else {
      top = Math.max(VIEWPORT_MARGIN, Math.min(top, vh - bubbleHeight - VIEWPORT_MARGIN));
    }

    setCoords({ top, left });
  };

  useLayoutEffect(() => {
    if (!open) return;
    updatePosition();
  }, [open, anchorEl, align]);

  useEffect(() => {
    if (!open || typeof document === "undefined") return;
    returnFocusRef.current =
      anchorEl ?? (document.activeElement instanceof HTMLElement ? document.activeElement : null);
    cancelBtnRef.current?.focus();

    const handlePointerDown = (e: MouseEvent) => {
      if (busyRef.current) return;
      const target = e.target as Node | null;
      if (!target) return;
      if (bubbleRef.current?.contains(target)) return;
      if (anchorEl?.contains(target)) return;
      onCancelRef.current();
    };

    const handleKeyDown = (e: KeyboardEvent) => {
      if (e.key === "Escape" && !busyRef.current) {
        e.preventDefault();
        e.stopPropagation();
        onCancelRef.current();
      }
    };

    const handleReposition = () => updatePosition();

    document.addEventListener("mousedown", handlePointerDown);
    document.addEventListener("keydown", handleKeyDown, true);
    window.addEventListener("resize", handleReposition);
    window.addEventListener("scroll", handleReposition, true);

    return () => {
      document.removeEventListener("mousedown", handlePointerDown);
      document.removeEventListener("keydown", handleKeyDown, true);
      window.removeEventListener("resize", handleReposition);
      window.removeEventListener("scroll", handleReposition, true);
      const prev = returnFocusRef.current;
      if (prev && prev.isConnected) {
        prev.focus();
      }
    };
  }, [open, anchorEl]);

  if (!open || typeof document === "undefined") return null;

  const resolvedAriaLabel = ariaLabel ?? (typeof title === "string" ? title : confirmLabel);

  const content = (
    <div
      ref={bubbleRef}
      role={role}
      aria-modal="false"
      aria-label={resolvedAriaLabel}
      data-testid={testId}
      style={
        coords
          ? { position: "fixed", top: `${coords.top}px`, left: `${coords.left}px`, width: `${POPOVER_WIDTH}px` }
          : { position: "fixed", top: "50%", left: "50%", transform: "translate(-50%, -50%)", width: `${POPOVER_WIDTH}px` }
      }
      onClick={(e) => e.stopPropagation()}
      className="z-[1600] rounded-sp-card border border-sp-line bg-sp-surface p-3.5 text-xs text-sp-text shadow-sp-overlay dark:bg-sp-card space-y-2.5"
    >
      <div className="font-semibold text-sp-text leading-snug">{title}</div>
      {description && (
        <div className="text-xs text-sp-text-secondary leading-relaxed break-words">
          {description}
        </div>
      )}
      {extra}
      {error && (
        <div role="alert" className="rounded-sp-sm bg-sp-danger-bg p-2 text-xs text-sp-danger-text">
          {error}
        </div>
      )}
      <div className="flex items-center justify-end gap-2 pt-1">
        <button
          ref={cancelBtnRef}
          type="button"
          data-testid="popconfirm-cancel-btn"
          disabled={busy}
          onClick={onCancel}
          className="inline-flex h-7 items-center rounded-sp-sm border border-sp-line bg-sp-surface px-2.5 text-xs font-medium text-sp-text hover:bg-sp-hover transition-colors duration-fast ease-sp cursor-pointer disabled:opacity-50 focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-sp-accent dark:bg-sp-card"
        >
          {cancelLabel}
        </button>
        <button
          type="button"
          data-testid="popconfirm-confirm-btn"
          disabled={busy}
          onClick={onConfirm}
          className={cn(
            "inline-flex h-7 items-center rounded-sp-sm px-2.5 text-xs font-semibold text-white transition-opacity duration-fast ease-sp cursor-pointer disabled:opacity-50 focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-sp-accent",
            tone === "danger"
              ? "bg-sp-danger-strong hover:opacity-90"
              : "bg-sp-accent-fill hover:opacity-90"
          )}
        >
          {confirmLabel}
        </button>
      </div>
    </div>
  );

  return createPortal(content, document.body);
}
