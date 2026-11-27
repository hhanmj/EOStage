import React from "react";
import { createPortal } from "react-dom";
import { cn } from "@/lib/utils";

const activeOverlayStack: symbol[] = [];
const overlayListeners = new Set<() => void>();

function notifyOverlayListeners() {
  for (const listener of overlayListeners) {
    listener();
  }
}

export interface ModalOverlayProps {
  children: React.ReactNode;
  /**
   * Whether clicking the dimmed backdrop area closes the overlay.
   * Per DESIGN.md "Modals And Overlays", editing/catalog/confirmation dialogs
   * MUST leave this `false` so accidental clicks never discard user input.
   */
  closeOnBackdropClick?: boolean;
  /** Render a transparent click-shield backdrop on the base layer instead of dark dimming. */
  transparentBackdrop?: boolean;
  onClose?: () => void;
  className?: string;
  role?: React.AriaRole;
  "aria-modal"?: boolean;
  "aria-label"?: string;
  "aria-labelledby"?: string;
  "data-testid"?: string;
}

/**
 * Shared high-contrast backdrop wrapper for modal dialogs and editing overlays.
 * Provides consistent light/dark dim masking and blocks accidental outside-click dismissal by default.
 * When multiple overlays are stacked, only the bottom-most base overlay renders the dark/blurred
 * backdrop while upper layers keep a transparent click-intercepting shield to prevent double-darkening.
 */
export function ModalOverlay({
  children,
  closeOnBackdropClick = false,
  transparentBackdrop = false,
  onClose,
  className,
  role,
  "aria-modal": ariaModal,
  "aria-label": ariaLabel,
  "aria-labelledby": ariaLabelledBy,
  "data-testid": dataTestId,
}: ModalOverlayProps) {
  const instanceIdRef = React.useRef<symbol | null>(null);
  if (!instanceIdRef.current) {
    instanceIdRef.current = Symbol("ModalOverlay");
  }
  const instanceId = instanceIdRef.current;
  const anchorRef = React.useRef<HTMLSpanElement | null>(null);
  const [portalTarget, setPortalTarget] = React.useState<Element | null>(null);
  const [, forceUpdate] = React.useReducer((count: number) => count + 1, 0);

  React.useLayoutEffect(() => {
    const found = anchorRef.current?.closest?.(".sp-modern") ?? null;
    setPortalTarget((prev) => (prev === found ? prev : found));
  }, []);

  React.useLayoutEffect(() => {
    if (!activeOverlayStack.includes(instanceId)) {
      activeOverlayStack.push(instanceId);
    }
    overlayListeners.add(forceUpdate);
    notifyOverlayListeners();
    return () => {
      const idx = activeOverlayStack.indexOf(instanceId);
      if (idx !== -1) {
        activeOverlayStack.splice(idx, 1);
      }
      overlayListeners.delete(forceUpdate);
      notifyOverlayListeners();
    };
  }, [instanceId]);

  const isBaseLayer =
    activeOverlayStack.length === 0 || activeOverlayStack[0] === instanceId;

  const overlayNode = (
    <div
      role={role}
      aria-modal={ariaModal}
      aria-label={ariaLabel}
      aria-labelledby={ariaLabelledBy}
      data-testid={dataTestId}
      data-overlay-layer={isBaseLayer ? "base" : "nested"}
      className={cn(
        "fixed inset-0 flex items-center justify-center p-4",
        portalTarget ? "z-[1250]" : "z-50",
        isBaseLayer && !transparentBackdrop
          ? "bg-black/45 dark:bg-black/65 backdrop-blur-[2px]"
          : "bg-transparent",
        className
      )}
      onClick={(e) => {
        if (e.target === e.currentTarget && closeOnBackdropClick && onClose) {
          onClose();
        }
      }}
    >
      {children}
    </div>
  );

  return (
    <>
      <span ref={anchorRef} className="hidden" aria-hidden="true" />
      {portalTarget ? createPortal(overlayNode, portalTarget) : overlayNode}
    </>
  );
}
