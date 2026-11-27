import React, { useCallback, useEffect, useLayoutEffect, useRef, useState } from "react";
import { ChevronDown } from "lucide-react";
import { cn } from "@/lib/utils";
import {
  SPRING_PRESETS,
  clamp,
  createSpring,
  createStretchyHighlight,
  hideStretchyHighlight,
  measureRelativeLayoutRect,
  prefersReducedMotion,
  readStretchyHighlightMetrics,
  settleIfClose,
  snapSpring,
  stepSpringSubdivided,
  stepStretchyHighlight,
  targetStretchyHighlight,
  tuneSpring,
  type Spring1D,
} from "@/lib/springDropdownMotion";

function canRunBrowserSprings(element: HTMLElement | null): boolean {
  if (prefersReducedMotion()) return false;
  if (typeof window === "undefined" || typeof window.requestAnimationFrame !== "function") {
    return false;
  }
  if (!element) return false;
  return true;
}

export interface SpringDropdownSurfaceProps
  extends Omit<React.HTMLAttributes<HTMLDivElement>, "children"> {
  open: boolean;
  children: React.ReactNode;
  /** Optional ref bound to the outer popup card (e.g. click-outside containment). */
  panelRef?: React.RefObject<HTMLDivElement | null>;
  /** Extra classes applied to the inner content box (put padding like `p-1` here). */
  innerClassName?: string;
  /** Stagger delay between items in ms (default 45ms). */
  staggerMs?: number;
  /** Subtle scale when collapsed (default 0.96). */
  closedScale?: number;
  /** Notifies parent while surface remains mounted (open or mid-close spring). */
  onPresentChange?: (present: boolean) => void;
}

/**
 * Shared DesignBookmark-style Spring Dropdown surface.
 * - Height expands & collapses on a real 4-substep spring (`selectOpen: 360 24` / `selectClose: 420 30`).
 * - Inner `[data-spring-item]` elements reveal with staggered `translate3d` + `opacity` (zero blur).
 * - Terminates `rAF` cleanly on settle (`0%` idle CPU) and unmounts synchronously in jsdom or `prefers-reduced-motion`.
 */
export function SpringDropdownSurface({
  open,
  children,
  panelRef,
  className,
  innerClassName,
  staggerMs = SPRING_PRESETS.staggerMs,
  closedScale = 0.96,
  onPresentChange,
  style,
  ...rest
}: SpringDropdownSurfaceProps) {
  const [mounted, setMounted] = useState(open);
  if (open && !mounted) {
    setMounted(true);
  }

  useEffect(() => {
    onPresentChange?.(mounted);
  }, [mounted, onPresentChange]);

  useEffect(() => {
    return () => {
      onPresentChange?.(false);
    };
  }, [onPresentChange]);

  const localOuterRef = useRef<HTMLDivElement | null>(null);
  const innerRef = useRef<HTMLDivElement | null>(null);
  const rafRef = useRef<number | null>(null);
  const phaseRef = useRef<"idle" | "opening" | "open" | "closing">("idle");
  const targetHeightRef = useRef<number>(0);

  const heightSpringRef = useRef<Spring1D>(
    createSpring(0, SPRING_PRESETS.selectOpen.k, SPRING_PRESETS.selectOpen.c)
  );
  const opacitySpringRef = useRef<Spring1D>(
    createSpring(0, SPRING_PRESETS.fade.k, SPRING_PRESETS.fade.c)
  );
  const scaleSpringRef = useRef<Spring1D>(
    createSpring(closedScale, SPRING_PRESETS.selectOpen.k, SPRING_PRESETS.selectOpen.c)
  );
  const itemSpringsRef = useRef<Spring1D[]>([]);
  const elapsedRef = useRef<number>(0);

  const assignOuterRef = useCallback(
    (node: HTMLDivElement | null) => {
      localOuterRef.current = node;
      if (panelRef) {
        panelRef.current = node;
      }
    },
    [panelRef]
  );

  const stopRaf = useCallback(() => {
    if (rafRef.current !== null && typeof window !== "undefined") {
      window.cancelAnimationFrame(rafRef.current);
      rafRef.current = null;
    }
  }, []);

  const clearInlineStyles = useCallback(() => {
    const outer = localOuterRef.current;
    const inner = innerRef.current;
    if (outer) {
      outer.style.height = "";
      outer.style.overflow = "";
      outer.style.opacity = "";
      outer.style.transform = "";
      outer.style.pointerEvents = "";
    }
    if (inner) {
      const items = inner.querySelectorAll<HTMLElement>("[data-spring-item]");
      items.forEach((el) => {
        el.style.opacity = "";
        el.style.transform = "";
      });
    }
  }, []);

  useLayoutEffect(() => {
    if (!mounted) {
      stopRaf();
      phaseRef.current = "idle";
      return;
    }

    const outer = localOuterRef.current;
    const inner = innerRef.current;
    const measureNaturalHeight = (): number => {
      if (!outer || !inner) return 0;
      const borderY = outer.offsetHeight - outer.clientHeight;
      return inner.offsetHeight > 0 ? inner.offsetHeight + borderY : 0;
    };

    if (!open) {
      const currentNatural = measureNaturalHeight();
      const canAnimateClose =
        canRunBrowserSprings(outer) &&
        (heightSpringRef.current.x > 1 || currentNatural > 0);

      if (!canAnimateClose) {
        stopRaf();
        phaseRef.current = "idle";
        setMounted(false);
        return;
      }

      if (phaseRef.current === "open" || heightSpringRef.current.x <= 0) {
        snapSpring(heightSpringRef.current, currentNatural);
        snapSpring(opacitySpringRef.current, 1);
        snapSpring(scaleSpringRef.current, 1);
      }

      phaseRef.current = "closing";
      tuneSpring(
        heightSpringRef.current,
        SPRING_PRESETS.selectClose.k,
        SPRING_PRESETS.selectClose.c,
        false
      );
      tuneSpring(
        opacitySpringRef.current,
        SPRING_PRESETS.fade.k,
        SPRING_PRESETS.fade.c,
        false
      );
      tuneSpring(
        scaleSpringRef.current,
        SPRING_PRESETS.selectClose.k,
        SPRING_PRESETS.selectClose.c,
        false
      );
      heightSpringRef.current.t = 0;
      opacitySpringRef.current.t = 0;
      scaleSpringRef.current.t = closedScale;

      if (outer) {
        outer.style.pointerEvents = "none";
        outer.style.overflow = "hidden";
      }

      stopRaf();
      let last = performance.now();
      const tickClose = (now: number) => {
        const dt = Math.min((now - last) / 1000, 1 / 30);
        last = now;

        stepSpringSubdivided(heightSpringRef.current, dt);
        stepSpringSubdivided(opacitySpringRef.current, dt);
        stepSpringSubdivided(scaleSpringRef.current, dt);

        const h = Math.max(0, heightSpringRef.current.x);
        const op = clamp(opacitySpringRef.current.x, 0, 1);
        const sc = clamp(scaleSpringRef.current.x, 0.85, 1.04);

        if (outer) {
          outer.style.height = `${h.toFixed(2)}px`;
          outer.style.opacity = `${op.toFixed(3)}`;
          outer.style.transform = `translate3d(0, ${((1 - op) * -3).toFixed(2)}px, 0) scale3d(${sc.toFixed(4)}, ${sc.toFixed(4)}, 1)`;
        }

        const done =
          (heightSpringRef.current.x <= 1.2 && opacitySpringRef.current.x <= 0.03) ||
          (settleIfClose(heightSpringRef.current, 0.5, 2) &&
            settleIfClose(opacitySpringRef.current, 0.02, 0.1));

        if (done) {
          stopRaf();
          phaseRef.current = "idle";
          setMounted(false);
          return;
        }
        rafRef.current = window.requestAnimationFrame(tickClose);
      };
      rafRef.current = window.requestAnimationFrame(tickClose);
      return;
    }

    // open === true
    const naturalH = measureNaturalHeight();
    if (!canRunBrowserSprings(outer) || naturalH <= 0) {
      stopRaf();
      clearInlineStyles();
      phaseRef.current = "open";
      return;
    }

    targetHeightRef.current = naturalH;
    const itemElements = inner
      ? Array.from(inner.querySelectorAll<HTMLElement>("[data-spring-item]"))
      : [];

    if (phaseRef.current === "idle") {
      snapSpring(heightSpringRef.current, 0);
      snapSpring(opacitySpringRef.current, 0);
      snapSpring(scaleSpringRef.current, closedScale);
      elapsedRef.current = 0;
      itemSpringsRef.current = itemElements.map(() =>
        createSpring(0, SPRING_PRESETS.fade.k, SPRING_PRESETS.fade.c)
      );
      if (outer) {
        outer.style.height = "0px";
        outer.style.opacity = "0";
        outer.style.overflow = "hidden";
        outer.style.pointerEvents = "auto";
        outer.style.transform = `translate3d(0, -4px, 0) scale3d(${closedScale}, ${closedScale}, 1)`;
      }
      itemElements.forEach((el) => {
        el.style.opacity = "0";
        el.style.transform = "translate3d(0, 7px, 0)";
      });
    } else if (phaseRef.current === "closing") {
      // Interrupted mid-close: immediately restore interactivity and reverse smoothly
      elapsedRef.current = 0.06;
      if (itemSpringsRef.current.length !== itemElements.length) {
        itemSpringsRef.current = itemElements.map(() =>
          createSpring(clamp(opacitySpringRef.current.x, 0, 1), SPRING_PRESETS.fade.k, SPRING_PRESETS.fade.c)
        );
      }
      if (outer) {
        outer.style.pointerEvents = "auto";
        outer.style.overflow = "hidden";
      }
    } else {
      // Already open or opening: preserve stagger if height simply updated
      if (phaseRef.current === "open" && itemSpringsRef.current.length === itemElements.length) {
        return;
      }
    }

    phaseRef.current = "opening";
    tuneSpring(heightSpringRef.current, SPRING_PRESETS.selectOpen.k, SPRING_PRESETS.selectOpen.c, false);
    tuneSpring(opacitySpringRef.current, SPRING_PRESETS.fade.k, SPRING_PRESETS.fade.c, false);
    tuneSpring(scaleSpringRef.current, SPRING_PRESETS.selectOpen.k, SPRING_PRESETS.selectOpen.c, false);
    heightSpringRef.current.t = naturalH;
    opacitySpringRef.current.t = 1;
    scaleSpringRef.current.t = 1;

    stopRaf();
    let last = performance.now();
    const tickOpen = (now: number) => {
      const dt = Math.min((now - last) / 1000, 1 / 30);
      last = now;
      elapsedRef.current += dt;

      // Refresh target height in case inner scroll content resized dynamically during opening
      const liveNatural = measureNaturalHeight();
      if (liveNatural > 0) {
        targetHeightRef.current = liveNatural;
        heightSpringRef.current.t = liveNatural;
      }

      stepSpringSubdivided(heightSpringRef.current, dt);
      stepSpringSubdivided(opacitySpringRef.current, dt);
      stepSpringSubdivided(scaleSpringRef.current, dt);

      // Keep height overshoot gently bounded so bordered cards cushion without leaving gaps
      const maxCushion = targetHeightRef.current + 6;
      const h = clamp(heightSpringRef.current.x, 0, maxCushion);
      const op = clamp(opacitySpringRef.current.x, 0, 1);
      const sc = clamp(scaleSpringRef.current.x, 0.9, 1.03);
      const dropY = (1 - clamp(h / Math.max(1, targetHeightRef.current), 0, 1.04)) * -4;

      if (outer) {
        outer.style.height = `${h.toFixed(2)}px`;
        outer.style.opacity = `${op.toFixed(3)}`;
        outer.style.transform = `translate3d(0, ${dropY.toFixed(2)}px, 0) scale3d(${sc.toFixed(4)}, ${sc.toFixed(4)}, 1)`;
      }

      const items = innerRef.current
        ? Array.from(innerRef.current.querySelectorAll<HTMLElement>("[data-spring-item]"))
        : [];
      while (itemSpringsRef.current.length < items.length) {
        itemSpringsRef.current.push(createSpring(0, SPRING_PRESETS.fade.k, SPRING_PRESETS.fade.c));
      }

      let allItemsSettled = true;
      for (let i = 0; i < items.length; i += 1) {
        const sp = itemSpringsRef.current[i];
        const delaySec = (Math.min(i, 6) * staggerMs) / 1000;
        if (elapsedRef.current >= delaySec) {
          sp.t = 1;
          stepSpringSubdivided(sp, dt);
        }
        const p = clamp(sp.x, 0, 1.04);
        const itemEl = items[i];
        itemEl.style.opacity = `${clamp(p, 0, 1).toFixed(3)}`;
        const ty = (1 - p) * 7;
        itemEl.style.transform = Math.abs(ty) < 0.05 ? "" : `translate3d(0, ${ty.toFixed(2)}px, 0)`;
        if (elapsedRef.current < delaySec || !settleIfClose(sp, 0.008, 0.05)) {
          allItemsSettled = false;
        }
      }

      const cardSettled =
        settleIfClose(heightSpringRef.current, 0.35, 1.5) &&
        settleIfClose(opacitySpringRef.current, 0.008, 0.05) &&
        settleIfClose(scaleSpringRef.current, 0.003, 0.03);

      if (cardSettled && allItemsSettled) {
        stopRaf();
        clearInlineStyles();
        phaseRef.current = "open";
        return;
      }

      rafRef.current = window.requestAnimationFrame(tickOpen);
    };

    rafRef.current = window.requestAnimationFrame(tickOpen);
  }, [clearInlineStyles, closedScale, mounted, open, staggerMs, stopRaf]);

  useEffect(() => {
    return () => {
      stopRaf();
    };
  }, [stopRaf]);

  if (!mounted) {
    return null;
  }

  return (
    <div
      ref={assignOuterRef}
      data-state={open ? "open" : "closing"}
      aria-hidden={!open ? true : undefined}
      style={style}
      className={cn("sp-spring-dropdown-surface", className)}
      {...rest}
    >
      <div ref={innerRef} className={innerClassName}>
        {children}
      </div>
    </div>
  );
}

/**
 * Stretchy hover/focus highlight pill driven by `lead` and `trail`/`cardTrail` springs.
 */
export function useStretchyMenuHighlight(
  options: { x?: number; y?: number; variant?: "menu" | "card" } = {}
) {
  const containerRef = useRef<HTMLDivElement | null>(null);
  const highlightRef = useRef<HTMLDivElement | null>(null);
  const stateRef = useRef(createStretchyHighlight());
  const rafRef = useRef<number | null>(null);
  const settleTimerRef = useRef<number | null>(null);
  const insetX = options.x ?? 0;
  const insetY = options.y ?? 0;
  const trailTuning =
    options.variant === "card" ? SPRING_PRESETS.cardTrail : SPRING_PRESETS.trail;

  const stopRaf = useCallback(() => {
    if (rafRef.current !== null && typeof window !== "undefined") {
      window.cancelAnimationFrame(rafRef.current);
      rafRef.current = null;
    }
    if (settleTimerRef.current !== null && typeof window !== "undefined") {
      window.clearTimeout(settleTimerRef.current);
      settleTimerRef.current = null;
    }
  }, []);

  const applyMetrics = useCallback(() => {
    const hl = highlightRef.current;
    if (!hl) return;
    const m = readStretchyHighlightMetrics(stateRef.current);
    hl.style.transform = `translate3d(${m.left.toFixed(2)}px, ${m.top.toFixed(2)}px, 0)`;
    hl.style.width = `${m.width.toFixed(2)}px`;
    hl.style.height = `${m.height.toFixed(2)}px`;
    hl.style.opacity = `${m.opacity.toFixed(3)}`;
  }, []);

  const startLoop = useCallback(() => {
    if (settleTimerRef.current !== null && typeof window !== "undefined") {
      window.clearTimeout(settleTimerRef.current);
      settleTimerRef.current = null;
    }
    const hl = highlightRef.current;
    if (!canRunBrowserSprings(hl)) {
      stopRaf();
      stepStretchyHighlight(stateRef.current, 1, true);
      applyMetrics();
      return;
    }
    if (typeof window !== "undefined") {
      settleTimerRef.current = window.setTimeout(() => {
        settleTimerRef.current = null;
        if (rafRef.current !== null) {
          window.cancelAnimationFrame(rafRef.current);
          rafRef.current = null;
        }
        stepStretchyHighlight(stateRef.current, 1, true);
        applyMetrics();
      }, 220);
    }
    if (rafRef.current !== null) return;

    let last = performance.now();
    const tick = (now: number) => {
      const dt = Math.min((now - last) / 1000, 1 / 30);
      last = now;
      const settled = stepStretchyHighlight(stateRef.current, dt, false);
      applyMetrics();
      if (settled) {
        rafRef.current = null;
        if (settleTimerRef.current !== null) {
          window.clearTimeout(settleTimerRef.current);
          settleTimerRef.current = null;
        }
        return;
      }
      rafRef.current = window.requestAnimationFrame(tick);
    };
    rafRef.current = window.requestAnimationFrame(tick);
  }, [applyMetrics, stopRaf]);

  const activateElement = useCallback(
    (el: HTMLElement | null) => {
      const container = containerRef.current;
      if (!container || !el) return;
      const rect = measureRelativeLayoutRect(container, el, { x: insetX, y: insetY });
      const reduced = !canRunBrowserSprings(highlightRef.current) || el.offsetHeight <= 0;
      targetStretchyHighlight(stateRef.current, rect, reduced, trailTuning);
      applyMetrics();
      if (!reduced) {
        startLoop();
      }
    },
    [applyMetrics, insetX, insetY, startLoop, trailTuning]
  );

  const clearHighlight = useCallback(() => {
    const reduced = !canRunBrowserSprings(highlightRef.current);
    hideStretchyHighlight(stateRef.current, reduced);
    applyMetrics();
    if (!reduced) {
      startLoop();
    }
  }, [applyMetrics, startLoop]);

  const resetHighlight = useCallback(() => {
    stopRaf();
    stateRef.current = createStretchyHighlight();
    applyMetrics();
  }, [applyMetrics, stopRaf]);

  useEffect(() => {
    return () => {
      stopRaf();
    };
  }, [stopRaf]);

  return {
    containerRef,
    highlightRef,
    activateElement,
    clearHighlight,
    resetHighlight,
  };
}

/**
 * Animated SVG checkmark (`stroke-dashoffset: 22 -> 0` on `pop` spring) matching DesignBookmark `.tick`.
 */
export function SpringCheckmark({
  visible,
  menuOpen,
  className,
}: {
  visible: boolean;
  menuOpen: boolean;
  className?: string;
}) {
  const svgRef = useRef<SVGSVGElement | null>(null);
  const pathRef = useRef<SVGPathElement | null>(null);
  const offsetSpringRef = useRef<Spring1D>(
    createSpring(visible ? 0 : 22, SPRING_PRESETS.pop.k, SPRING_PRESETS.pop.c)
  );
  const rafRef = useRef<number | null>(null);

  useLayoutEffect(() => {
    if (rafRef.current !== null && typeof window !== "undefined") {
      window.cancelAnimationFrame(rafRef.current);
      rafRef.current = null;
    }

    const pathEl = pathRef.current;
    const svgEl = svgRef.current;
    if (!pathEl) return;

    if (!visible) {
      snapSpring(offsetSpringRef.current, 22);
      pathEl.style.strokeDashoffset = "22";
      return;
    }

    const hostHeight = svgEl?.closest("button")?.offsetHeight ?? 0;
    if (!menuOpen || prefersReducedMotion() || hostHeight <= 0 || typeof window.requestAnimationFrame !== "function") {
      snapSpring(offsetSpringRef.current, 0);
      pathEl.style.strokeDashoffset = "0";
      return;
    }

    snapSpring(offsetSpringRef.current, 22);
    tuneSpring(offsetSpringRef.current, SPRING_PRESETS.pop.k, SPRING_PRESETS.pop.c, false);
    offsetSpringRef.current.t = 0;
    pathEl.style.strokeDashoffset = "22";

    let elapsed = 0;
    let last = performance.now();
    const tick = (now: number) => {
      const dt = Math.min((now - last) / 1000, 1 / 30);
      last = now;
      elapsed += dt;
      // Delay stroke draw slightly so it paints right as the option row lands
      if (elapsed >= 0.08) {
        stepSpringSubdivided(offsetSpringRef.current, dt);
        const offset = clamp(offsetSpringRef.current.x, 0, 22);
        pathEl.style.strokeDashoffset = `${offset.toFixed(2)}`;
        if (settleIfClose(offsetSpringRef.current, 0.12, 0.5)) {
          pathEl.style.strokeDashoffset = "0";
          rafRef.current = null;
          return;
        }
      }
      rafRef.current = window.requestAnimationFrame(tick);
    };
    rafRef.current = window.requestAnimationFrame(tick);

    return () => {
      if (rafRef.current !== null && typeof window !== "undefined") {
        window.cancelAnimationFrame(rafRef.current);
        rafRef.current = null;
      }
    };
  }, [visible, menuOpen]);

  if (!visible) return null;

  return (
    <svg
      ref={svgRef}
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth={2.2}
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden="true"
      className={cn("h-3.5 w-3.5 text-[#18181b] dark:text-white shrink-0", className)}
    >
      <path
        ref={pathRef}
        d="M5 12.5l4.5 4.5L19 7.5"
        strokeDasharray={22}
        strokeDashoffset={0}
      />
    </svg>
  );
}

/**
 * Spring-rotated ChevronDown (`0deg <-> 180deg`, `settle: 280 22`).
 */
export function SpringChevron({
  open,
  className,
}: {
  open: boolean;
  className?: string;
}) {
  const iconRef = useRef<SVGSVGElement | null>(null);
  const angleSpringRef = useRef<Spring1D>(
    createSpring(open ? 180 : 0, SPRING_PRESETS.settle.k, SPRING_PRESETS.settle.c)
  );
  const rafRef = useRef<number | null>(null);

  useLayoutEffect(() => {
    if (rafRef.current !== null && typeof window !== "undefined") {
      window.cancelAnimationFrame(rafRef.current);
      rafRef.current = null;
    }

    const el = iconRef.current;
    if (!el) return;
    const targetDeg = open ? 180 : 0;
    const hostHeight = el.closest("button")?.offsetHeight ?? 0;

    if (prefersReducedMotion() || hostHeight <= 0 || typeof window.requestAnimationFrame !== "function") {
      snapSpring(angleSpringRef.current, targetDeg);
      el.style.transform = `rotate(${targetDeg}deg)`;
      return;
    }

    tuneSpring(angleSpringRef.current, SPRING_PRESETS.settle.k, SPRING_PRESETS.settle.c, false);
    angleSpringRef.current.t = targetDeg;

    let last = performance.now();
    const tick = (now: number) => {
      const dt = Math.min((now - last) / 1000, 1 / 30);
      last = now;
      stepSpringSubdivided(angleSpringRef.current, dt);
      el.style.transform = `rotate(${angleSpringRef.current.x.toFixed(2)}deg)`;
      if (settleIfClose(angleSpringRef.current, 0.2, 0.8)) {
        el.style.transform = `rotate(${targetDeg}deg)`;
        rafRef.current = null;
        return;
      }
      rafRef.current = window.requestAnimationFrame(tick);
    };
    rafRef.current = window.requestAnimationFrame(tick);

    return () => {
      if (rafRef.current !== null && typeof window !== "undefined") {
        window.cancelAnimationFrame(rafRef.current);
        rafRef.current = null;
      }
    };
  }, [open]);

  return (
    <ChevronDown
      ref={iconRef}
      aria-hidden="true"
      className={cn("h-3 w-3 shrink-0 will-change-transform", className)}
    />
  );
}

interface SlotItemSnapshot {
  key: string;
  Icon: React.ComponentType<{ className?: string }>;
  label: string;
  width: number;
}

/**
 * DesignBookmark vertical slot roll-up inside the workspace switcher trigger.
 * Only renders a temporary `aria-hidden="true"` outgoing ghost while rolling in a real browser
 * (`offsetHeight > 0` and `!prefersReducedMotion()`); at rest and in jsdom renders one clean span.
 */
export function SpringSlotTriggerValue({
  routeKey,
  icon: Icon,
  label,
}: {
  routeKey: string;
  icon: React.ComponentType<{ className?: string }>;
  label: string;
}) {
  const wrapRef = useRef<HTMLSpanElement | null>(null);
  const incomingRef = useRef<HTMLSpanElement | null>(null);
  const outgoingRef = useRef<HTMLSpanElement | null>(null);
  const lastSnapshotRef = useRef<SlotItemSnapshot>({
    key: routeKey,
    Icon,
    label,
    width: 0,
  });
  const [outgoingGhost, setOutgoingGhost] = useState<SlotItemSnapshot | null>(null);
  const rafRef = useRef<number | null>(null);

  useLayoutEffect(() => {
    const wrap = wrapRef.current;
    const incoming = incomingRef.current;
    const prev = lastSnapshotRef.current;

    if (prev.key !== routeKey) {
      const canAnimateSlot =
        Boolean(wrap && incoming) &&
        (wrap?.offsetHeight ?? 0) > 0 &&
        prev.width > 0 &&
        !prefersReducedMotion() &&
        typeof window !== "undefined" &&
        typeof window.requestAnimationFrame === "function";

      if (canAnimateSlot) {
        setOutgoingGhost(prev);
      } else {
        setOutgoingGhost(null);
      }

      lastSnapshotRef.current = {
        key: routeKey,
        Icon,
        label,
        width: incoming ? incoming.offsetWidth : prev.width,
      };
      return;
    }

    if (incoming && incoming.offsetWidth > 0 && !outgoingGhost) {
      lastSnapshotRef.current.width = incoming.offsetWidth;
    }
  }, [Icon, label, outgoingGhost, routeKey]);

  useLayoutEffect(() => {
    if (!outgoingGhost) return;

    const wrap = wrapRef.current;
    const incoming = incomingRef.current;
    const outgoing = outgoingRef.current;
    if (!wrap || !incoming || !outgoing || prefersReducedMotion()) {
      setOutgoingGhost(null);
      return;
    }

    const fromW = outgoingGhost.width;
    wrap.style.width = "";
    const toW = incoming.offsetWidth;
    const roll = createSpring(0, SPRING_PRESETS.settle.k, SPRING_PRESETS.settle.c);
    roll.t = 1;
    const expanding = toW > fromW;
    const widthSpring = createSpring(
      fromW,
      expanding ? SPRING_PRESETS.lead.k : SPRING_PRESETS.settle.k,
      expanding ? SPRING_PRESETS.lead.c : SPRING_PRESETS.settle.c
    );
    widthSpring.t = toW;

    wrap.style.width = `${fromW.toFixed(2)}px`;
    outgoing.style.transform = "translate3d(0, 0px, 0)";
    outgoing.style.opacity = "1";
    incoming.style.transform = "translate3d(0, 18px, 0)";
    incoming.style.opacity = "0";

    let last = performance.now();
    const tick = (now: number) => {
      const dt = Math.min((now - last) / 1000, 1 / 30);
      last = now;
      stepSpringSubdivided(roll, dt);
      stepSpringSubdivided(widthSpring, dt);
      const p = roll.x;

      if (fromW > 0 && toW > 0) {
        const currentW = Math.max(20, widthSpring.x);
        wrap.style.width = `${currentW.toFixed(2)}px`;
      }
      outgoing.style.transform = `translate3d(0, ${(-p * 18).toFixed(2)}px, 0)`;
      outgoing.style.opacity = `${clamp(1 - p * 1.25, 0, 1).toFixed(3)}`;
      incoming.style.transform = `translate3d(0, ${((1 - p) * 18).toFixed(2)}px, 0)`;
      incoming.style.opacity = `${clamp(p * 1.25, 0, 1).toFixed(3)}`;

      const rollDone = settleIfClose(roll, 0.01, 0.12);
      const widthDone = settleIfClose(widthSpring, 0.25, 1.0);
      if (rollDone && widthDone) {
        wrap.style.width = "";
        incoming.style.transform = "";
        incoming.style.opacity = "";
        lastSnapshotRef.current.width = toW;
        rafRef.current = null;
        setOutgoingGhost(null);
        return;
      }
      rafRef.current = window.requestAnimationFrame(tick);
    };

    rafRef.current = window.requestAnimationFrame(tick);

    return () => {
      if (rafRef.current !== null && typeof window !== "undefined") {
        window.cancelAnimationFrame(rafRef.current);
        rafRef.current = null;
      }
      if (wrap) wrap.style.width = "";
      if (incoming) {
        incoming.style.transform = "";
        incoming.style.opacity = "";
      }
    };
  }, [outgoingGhost]);

  const OutgoingIcon = outgoingGhost?.Icon;

  return (
    <span
      ref={wrapRef}
      data-testid="titlebar-workspace-slot"
      className="relative inline-flex h-[18px] items-center overflow-hidden align-middle"
    >
      {outgoingGhost && OutgoingIcon && (
        <span
          ref={outgoingRef}
          aria-hidden="true"
          className="pointer-events-none absolute left-0 top-0 inline-flex h-full items-center gap-1.5 whitespace-nowrap will-change-transform"
        >
          <OutgoingIcon className="h-3.5 w-3.5 shrink-0 text-[#52525b] dark:text-[#a1a1aa]" />
          <span>{outgoingGhost.label}</span>
        </span>
      )}
      <span
        ref={incomingRef}
        className="inline-flex h-full shrink-0 items-center gap-1.5 whitespace-nowrap will-change-transform"
      >
        <Icon className="h-3.5 w-3.5 shrink-0 text-[#52525b] dark:text-[#a1a1aa]" />
        <span>{label}</span>
      </span>
    </span>
  );
}
