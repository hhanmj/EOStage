/**
 * Zero-dependency 1D spring physics & stretchy highlight engine for EOStage floating cards,
 * expandable status pills, dropdowns, and navigation surfaces.
 *
 * Physics presets & multi-substep semi-implicit Euler integrator calibrated after
 * DesignBookmark's Spring Dropdown motion study (`stiffness`, `damping`, `stagger`),
 * with automatic idle termination (0% idle CPU) and strict `prefers-reduced-motion` bypass.
 */

export interface Spring1D {
  /** Current position. */
  x: number;
  /** Current velocity (units/sec). */
  v: number;
  /** Target position. */
  t: number;
  /** Spring stiffness (k). */
  k: number;
  /** Spring damping (c). */
  c: number;
}

export interface SpringTuning {
  k: number;
  c: number;
}

export const SPRING_PRESETS = {
  /** Trigger <-> menu / floating card expansion open (one clean, subtle overshoot). */
  selectOpen: { k: 360, c: 24 },
  /** Crisp close collapse with no lingering bounce. */
  selectClose: { k: 420, c: 30 },
  /** Pill-to-card horizontal width morph (`--dock-open-w: 340 21`). */
  dockOpenWidth: { k: 340, c: 21 },
  /** Pill-to-card vertical height morph (`--dock-open-h: 230 21`). */
  dockOpenHeight: { k: 280, c: 23 },
  /** Critically damped card-to-pill collapse (`ζ ≈ 1.01`, crisp ~320ms settle with zero undershoot or rebound). */
  pillCollapse: { k: 520, c: 46 },
  /** Left sidebar width expansion (`52px -> 248px`, smooth critical damping `ζ ≈ 0.99`, zero overshoot rebound). */
  sidebarExpand: { k: 260, c: 32 },
  /** Left sidebar width collapse (`248px -> 52px`, smooth critical damping `ζ ≈ 0.99`, zero initial jolt and zero rebound). */
  sidebarCollapse: { k: 260, c: 32 },
  /** Trigger chevron & slot roll-up settle. */
  settle: { k: 280, c: 22 },
  /** Stretchy highlight leading edge (facing direction of travel). */
  lead: { k: 620, c: 36 },
  /** Stretchy highlight trailing edge (crisp elastic stretch on compact 30px menu rows). */
  trail: { k: 290, c: 29 },
  /** Tighter trailing edge for tall 60–75px multi-line download & notification cards. */
  cardTrail: { k: 400, c: 35 },
  /** Opacity & micro-slide fade. */
  fade: { k: 420, c: 32 },
  /** Checkmark draw / selection pop. */
  pop: { k: 520, c: 16 },
  /** Per-item entrance stagger interval (ms). */
  staggerMs: 45,
} as const;

export function clamp(value: number, min: number, max: number): number {
  return Math.max(min, Math.min(max, value));
}

/**
 * Safely checks whether the runtime environment prefers reduced motion.
 * Guards jsdom or SSR environments where `window.matchMedia` is absent.
 */
export function prefersReducedMotion(): boolean {
  if (typeof window === "undefined" || typeof window.matchMedia !== "function") {
    return false;
  }
  try {
    return window.matchMedia("(prefers-reduced-motion: reduce)").matches;
  } catch {
    return false;
  }
}

/**
 * Returns `true` only when running in a real browser layout viewport (`offsetWidth > 0`),
 * `requestAnimationFrame` is available, and `prefers-reduced-motion` is not active.
 * In jsdom (`offsetWidth === 0`) or reduced-motion environments, callers stay on synchronous
 * deterministic state without asynchronous RAF timers.
 */
export function canAnimateSpringInBrowser(el: HTMLElement | null): boolean {
  if (!el || prefersReducedMotion()) return false;
  if (typeof window === "undefined" || typeof window.requestAnimationFrame !== "function") {
    return false;
  }
  return el.offsetWidth > 0 || el.offsetHeight > 0;
}

export function createSpring(
  initial: number,
  k: number = SPRING_PRESETS.settle.k,
  c: number = SPRING_PRESETS.settle.c
): Spring1D {
  return { x: initial, v: 0, t: initial, k, c };
}

export function tuneSpring(
  spring: Spring1D,
  k: number,
  c: number,
  reducedMotion: boolean = prefersReducedMotion()
): void {
  spring.k = k;
  spring.c = reducedMotion ? 2 * Math.sqrt(k) : c;
}

/**
 * Advances a 1D spring by `dtSeconds` using subdivided semi-implicit Euler steps
 * (clamped at 1/30s frame delta, 4 substeps per frame for numerical stability on stiff springs).
 */
export function stepSpringSubdivided(
  spring: Spring1D,
  dtSeconds: number,
  substeps: number = 4
): void {
  const clampedDt = clamp(dtSeconds, 0, 1 / 30);
  if (clampedDt <= 0) return;
  const steps = Math.max(1, Math.floor(substeps));
  const subDt = clampedDt / steps;
  for (let i = 0; i < steps; i += 1) {
    const accel = -spring.k * (spring.x - spring.t) - spring.c * spring.v;
    spring.v += accel * subDt;
    spring.x += spring.v * subDt;
  }
}

export function isSpringSettled(
  spring: Spring1D,
  posEps: number = 0.15,
  velEps: number = 0.6
): boolean {
  return Math.abs(spring.x - spring.t) <= posEps && Math.abs(spring.v) <= velEps;
}

export function settleIfClose(
  spring: Spring1D,
  posEps: number = 0.15,
  velEps: number = 0.6
): boolean {
  if (!isSpringSettled(spring, posEps, velEps)) {
    return false;
  }
  spring.x = spring.t;
  spring.v = 0;
  return true;
}

export function snapSpring(spring: Spring1D, value: number): void {
  spring.x = value;
  spring.t = value;
  spring.v = 0;
}

export interface HighlightRect {
  top: number;
  left: number;
  width: number;
  height: number;
}

export interface StretchyHighlightState {
  topEdge: Spring1D;
  bottomEdge: Spring1D;
  leftEdge: Spring1D;
  widthSpring: Spring1D;
  opacity: Spring1D;
  active: boolean;
}

export interface StretchyHighlightMetrics {
  top: number;
  left: number;
  width: number;
  height: number;
  opacity: number;
}

export function createStretchyHighlight(): StretchyHighlightState {
  return {
    topEdge: createSpring(0, SPRING_PRESETS.lead.k, SPRING_PRESETS.lead.c),
    bottomEdge: createSpring(0, SPRING_PRESETS.trail.k, SPRING_PRESETS.trail.c),
    leftEdge: createSpring(0, SPRING_PRESETS.lead.k, SPRING_PRESETS.lead.c),
    widthSpring: createSpring(0, SPRING_PRESETS.lead.k, SPRING_PRESETS.lead.c),
    opacity: createSpring(0, SPRING_PRESETS.fade.k, SPRING_PRESETS.fade.c),
    active: false,
  };
}

/**
 * Retargets the stretchy highlight pill to a menu row.
 * When appearing from hidden, snaps geometry in place before fading in.
 * When moving between rows, gives the leading edge faster stiffness (`lead`)
 * and the trailing edge configurable trail stiffness (`trail` or `cardTrail`) so the pill stretches in motion.
 */
export function targetStretchyHighlight(
  state: StretchyHighlightState,
  rect: HighlightRect,
  reducedMotion: boolean = prefersReducedMotion(),
  trailTuning: SpringTuning = SPRING_PRESETS.trail
): void {
  const targetTop = rect.top;
  const targetBottom = rect.top + Math.max(0, rect.height);
  const coldStart = reducedMotion || !state.active || state.opacity.x < 0.05;

  if (coldStart) {
    snapSpring(state.topEdge, targetTop);
    snapSpring(state.bottomEdge, targetBottom);
    snapSpring(state.leftEdge, rect.left);
    snapSpring(state.widthSpring, rect.width);
    if (reducedMotion) {
      snapSpring(state.opacity, 1);
    } else {
      tuneSpring(state.opacity, SPRING_PRESETS.fade.k, SPRING_PRESETS.fade.c, false);
      state.opacity.t = 1;
    }
    state.active = true;
    return;
  }

  const movingDown = targetTop >= state.topEdge.t;
  if (movingDown) {
    tuneSpring(state.bottomEdge, SPRING_PRESETS.lead.k, SPRING_PRESETS.lead.c, false);
    tuneSpring(state.topEdge, trailTuning.k, trailTuning.c, false);
  } else {
    tuneSpring(state.topEdge, SPRING_PRESETS.lead.k, SPRING_PRESETS.lead.c, false);
    tuneSpring(state.bottomEdge, trailTuning.k, trailTuning.c, false);
  }
  tuneSpring(state.leftEdge, SPRING_PRESETS.lead.k, SPRING_PRESETS.lead.c, false);
  tuneSpring(state.widthSpring, SPRING_PRESETS.lead.k, SPRING_PRESETS.lead.c, false);
  tuneSpring(state.opacity, SPRING_PRESETS.fade.k, SPRING_PRESETS.fade.c, false);

  state.topEdge.t = targetTop;
  state.bottomEdge.t = targetBottom;
  state.leftEdge.t = rect.left;
  state.widthSpring.t = rect.width;
  state.opacity.t = 1;
  state.active = true;
}

export function hideStretchyHighlight(
  state: StretchyHighlightState,
  reducedMotion: boolean = prefersReducedMotion()
): void {
  state.active = false;
  if (reducedMotion) {
    snapSpring(state.opacity, 0);
    return;
  }
  tuneSpring(state.opacity, SPRING_PRESETS.fade.k, SPRING_PRESETS.fade.c, false);
  state.opacity.t = 0;
}

/**
 * Advances all stretchy highlight springs by one frame.
 * Returns `true` if the animation has settled and `rAF` can stop.
 */
export function stepStretchyHighlight(
  state: StretchyHighlightState,
  dtSeconds: number,
  reducedMotion: boolean = prefersReducedMotion()
): boolean {
  if (reducedMotion) {
    snapSpring(state.topEdge, state.topEdge.t);
    snapSpring(state.bottomEdge, state.bottomEdge.t);
    snapSpring(state.leftEdge, state.leftEdge.t);
    snapSpring(state.widthSpring, state.widthSpring.t);
    snapSpring(state.opacity, state.opacity.t);
    return true;
  }

  stepSpringSubdivided(state.topEdge, dtSeconds);
  stepSpringSubdivided(state.bottomEdge, dtSeconds);
  stepSpringSubdivided(state.leftEdge, dtSeconds);
  stepSpringSubdivided(state.widthSpring, dtSeconds);
  stepSpringSubdivided(state.opacity, dtSeconds);

  const s1 = settleIfClose(state.topEdge, 0.15, 0.8);
  const s2 = settleIfClose(state.bottomEdge, 0.15, 0.8);
  const s3 = settleIfClose(state.leftEdge, 0.15, 0.8);
  const s4 = settleIfClose(state.widthSpring, 0.15, 0.8);
  const s5 = settleIfClose(state.opacity, 0.008, 0.05);
  return s1 && s2 && s3 && s4 && s5;
}

export function readStretchyHighlightMetrics(
  state: StretchyHighlightState
): StretchyHighlightMetrics {
  const top = Math.min(state.topEdge.x, state.bottomEdge.x);
  const bottom = Math.max(state.topEdge.x, state.bottomEdge.x);
  return {
    top,
    left: state.leftEdge.x,
    width: Math.max(0, state.widthSpring.x),
    height: Math.max(0, bottom - top),
    opacity: clamp(state.opacity.x, 0, 1),
  };
}

/**
 * Computes an element's resting (un-transformed) coordinates inside `container`.
 * Walking `offsetParent` avoids transient `translate3d`/`scale3d` distortion from
 * staggered item entrance or card scale animations.
 */
export function measureRelativeLayoutRect(
  container: HTMLElement,
  element: HTMLElement,
  inset: { x?: number; y?: number } = {}
): HighlightRect {
  const insetX = inset.x ?? 0;
  const insetY = inset.y ?? 0;

  if (element.offsetParent === container && element.offsetHeight > 0) {
    return {
      top: element.offsetTop + insetY,
      left: element.offsetLeft + insetX,
      width: Math.max(0, element.offsetWidth - insetX * 2),
      height: Math.max(0, element.offsetHeight - insetY * 2),
    };
  }

  let top = 0;
  let left = 0;
  let current: HTMLElement | null = element;
  let reachedContainer = false;

  while (current && current !== container) {
    top += current.offsetTop;
    left += current.offsetLeft;
    const nextParent = current.offsetParent as HTMLElement | null;
    if (!nextParent || !container.contains(nextParent)) {
      break;
    }
    current = nextParent;
    if (current === container) {
      reachedContainer = true;
    }
  }

  if (reachedContainer && element.offsetHeight > 0) {
    return {
      top: top + insetY,
      left: left + insetX,
      width: Math.max(0, element.offsetWidth - insetX * 2),
      height: Math.max(0, element.offsetHeight - insetY * 2),
    };
  }

  const cRect = container.getBoundingClientRect();
  const eRect = element.getBoundingClientRect();
  return {
    top: eRect.top - cRect.top + container.scrollTop + insetY,
    left: eRect.left - cRect.left + container.scrollLeft + insetX,
    width: Math.max(0, eRect.width - insetX * 2),
    height: Math.max(0, eRect.height - insetY * 2),
  };
}
