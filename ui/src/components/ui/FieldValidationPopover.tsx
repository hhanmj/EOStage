import React, { useEffect, useLayoutEffect, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { cn } from "@/lib/utils";

export interface FieldValidationPopoverProps {
  id: string;
  message: string | null | undefined;
  anchorRef: React.RefObject<HTMLElement | null>;
  onDismiss?: () => void;
  testId?: string;
}

const VIEWPORT_MARGIN = 12;
const ANCHOR_GAP = 8;
const MAX_POPOVER_WIDTH = 340;

/**
 * Monochrome field validation callout anchored below (or above if space requires)
 * an invalid input, adhering strictly to DESIGN.md carbon/zinc tokens.
 */
export function FieldValidationPopover({
  id,
  message,
  anchorRef,
  onDismiss,
  testId,
}: FieldValidationPopoverProps) {
  const open = Boolean(message && message.trim().length > 0);
  const bubbleRef = useRef<HTMLDivElement | null>(null);
  const onDismissRef = useRef(onDismiss);
  onDismissRef.current = onDismiss;

  const [coords, setCoords] = useState<{
    top: number;
    left: number;
    caretLeft: number;
    placement: "bottom" | "top";
  } | null>(null);

  const updatePosition = () => {
    if (!open || typeof window === "undefined") return;
    const anchorEl = anchorRef.current;
    if (!anchorEl || !anchorEl.isConnected) {
      setCoords(null);
      return;
    }
    const rect = anchorEl.getBoundingClientRect();
    const vw = window.innerWidth || 1280;
    const vh = window.innerHeight || 800;
    const bubbleWidth = bubbleRef.current?.offsetWidth || Math.min(280, vw - VIEWPORT_MARGIN * 2);
    const bubbleHeight = bubbleRef.current?.offsetHeight || 44;

    if (rect.width === 0 && rect.height === 0 && rect.top === 0 && rect.left === 0) {
      setCoords({ top: 16, left: 16, caretLeft: 18, placement: "bottom" });
      return;
    }

    const left = Math.max(
      VIEWPORT_MARGIN,
      Math.min(rect.left, vw - bubbleWidth - VIEWPORT_MARGIN)
    );
    const caretLeft = Math.max(
      14,
      Math.min(rect.left + 16 - left, Math.max(14, bubbleWidth - 22))
    );

    const belowTop = rect.bottom + ANCHOR_GAP;
    const aboveTop = rect.top - ANCHOR_GAP - bubbleHeight;
    if (belowTop + bubbleHeight > vh - VIEWPORT_MARGIN && aboveTop >= VIEWPORT_MARGIN) {
      setCoords({ top: aboveTop, left, caretLeft, placement: "top" });
    } else {
      setCoords({
        top: Math.max(VIEWPORT_MARGIN, Math.min(belowTop, vh - bubbleHeight - VIEWPORT_MARGIN)),
        left,
        caretLeft,
        placement: "bottom",
      });
    }
  };

  useLayoutEffect(() => {
    if (!open) return;
    updatePosition();
  }, [open, message]);

  useEffect(() => {
    if (!open || typeof document === "undefined") return;

    const handlePointerDown = (event: PointerEvent) => {
      const target = event.target as Node | null;
      if (!target) return;
      if (bubbleRef.current?.contains(target)) return;
      if (anchorRef.current?.contains(target)) return;
      onDismissRef.current?.();
    };

    const handleKeyDown = (event: KeyboardEvent) => {
      if (event.key === "Escape") {
        event.preventDefault();
        event.stopPropagation();
        onDismissRef.current?.();
      }
    };

    const handleReposition = () => updatePosition();

    document.addEventListener("pointerdown", handlePointerDown);
    document.addEventListener("keydown", handleKeyDown, true);
    window.addEventListener("resize", handleReposition);
    window.addEventListener("scroll", handleReposition, true);
    return () => {
      document.removeEventListener("pointerdown", handlePointerDown);
      document.removeEventListener("keydown", handleKeyDown, true);
      window.removeEventListener("resize", handleReposition);
      window.removeEventListener("scroll", handleReposition, true);
    };
  }, [open]);

  if (!open || typeof document === "undefined") return null;

  const placement = coords?.placement ?? "bottom";
  const caretLeft = coords?.caretLeft ?? 18;

  const node = (
    <div
      ref={bubbleRef}
      id={id}
      role="alert"
      aria-live="assertive"
      data-testid={testId ?? id}
      data-placement={placement}
      style={
        coords
          ? {
              position: "fixed",
              top: `${coords.top}px`,
              left: `${coords.left}px`,
              maxWidth: `min(${MAX_POPOVER_WIDTH}px, calc(100vw - ${VIEWPORT_MARGIN * 2}px))`,
            }
          : {
              position: "fixed",
              top: "16px",
              left: "16px",
              maxWidth: `min(${MAX_POPOVER_WIDTH}px, calc(100vw - ${VIEWPORT_MARGIN * 2}px))`,
            }
      }
      onClick={(e) => e.stopPropagation()}
      className={cn(
        "sp-modern sp-popover-blur-in z-[3000] flex items-start gap-2.5 rounded-sp-card border border-sp-line bg-sp-surface px-3 py-2 text-[13px] leading-snug text-sp-text shadow-sp-overlay dark:bg-sp-card",
        placement === "bottom" ? "origin-top-left" : "origin-bottom-left"
      )}
    >
      <span
        aria-hidden="true"
        style={{ left: `${caretLeft}px` }}
        className={cn(
          "pointer-events-none absolute h-2.5 w-2.5 rotate-45 border-sp-line bg-sp-surface dark:bg-sp-card",
          placement === "bottom"
            ? "-top-[5.5px] border-l border-t"
            : "-bottom-[5.5px] border-r border-b"
        )}
      />
      <span
        aria-hidden="true"
        className="mt-0.5 inline-flex h-[18px] w-[18px] shrink-0 select-none items-center justify-center rounded-[5px] bg-sp-accent-fill text-[11px] font-bold leading-none text-sp-accent-ink"
      >
        !
      </span>
      <span className="min-w-0 break-words">{message}</span>
    </div>
  );

  return createPortal(node, document.body);
}
