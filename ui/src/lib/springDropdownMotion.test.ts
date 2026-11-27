import test from "node:test";
import assert from "node:assert/strict";
import {
  SPRING_PRESETS,
  createSpring,
  createStretchyHighlight,
  hideStretchyHighlight,
  isSpringSettled,
  readStretchyHighlightMetrics,
  settleIfClose,
  stepSpringSubdivided,
  stepStretchyHighlight,
  targetStretchyHighlight,
} from "./springDropdownMotion.ts";

test("springDropdownMotion: 4-substep integrator & stretchy highlight physics", async (t) => {
  await t.test("1. 4-substep spring solver converges cleanly and zeroes velocity at rest", () => {
    const spring = createSpring(0, SPRING_PRESETS.selectOpen.k, SPRING_PRESETS.selectOpen.c);
    spring.t = 240;

    let maxPos = 0;
    for (let frame = 0; frame < 90; frame += 1) {
      stepSpringSubdivided(spring, 1 / 60);
      if (spring.x > maxPos) {
        maxPos = spring.x;
      }
      if (settleIfClose(spring, 0.2, 0.8)) {
        break;
      }
    }

    assert.ok(maxPos > 240, `Expected slight open overshoot above 240px, got ${maxPos}`);
    assert.strictEqual(spring.x, 240);
    assert.strictEqual(spring.v, 0);
    assert.strictEqual(isSpringSettled(spring, 0.2, 0.8), true);
  });

  await t.test("2. Downward highlight travel stretches (bottom leads, top trails) then settles cleanly", () => {
    const hl = createStretchyHighlight();
    targetStretchyHighlight(hl, { top: 4, left: 4, width: 208, height: 32 }, false);
    for (let i = 0; i < 30; i += 1) {
      stepStretchyHighlight(hl, 1 / 60, false);
    }

    const beforeGlide = readStretchyHighlightMetrics(hl);
    assert.ok(Math.abs(beforeGlide.top - 4) < 0.2);
    assert.ok(Math.abs(beforeGlide.height - 32) < 0.2);
    assert.ok(beforeGlide.opacity > 0.95);

    targetStretchyHighlight(hl, { top: 124, left: 4, width: 208, height: 32 }, false);

    let peakHeight = 0;
    for (let i = 0; i < 60; i += 1) {
      const done = stepStretchyHighlight(hl, 1 / 60, false);
      peakHeight = Math.max(peakHeight, readStretchyHighlightMetrics(hl).height);
      if (done) break;
    }

    assert.ok(
      peakHeight > 42,
      `Expected downward stretchy highlight mid-flight height > 42px (resting 32px), got ${peakHeight}`
    );

    const settled = readStretchyHighlightMetrics(hl);
    assert.ok(Math.abs(settled.top - 124) <= 0.2, `Expected settled top ~124, got ${settled.top}`);
    assert.ok(Math.abs(settled.height - 32) <= 0.2, `Expected settled height ~32, got ${settled.height}`);
  });

  await t.test("3. Upward highlight travel stretches (top leads, bottom trails) then settles cleanly", () => {
    const hl = createStretchyHighlight();
    targetStretchyHighlight(hl, { top: 124, left: 4, width: 208, height: 32 }, false);
    for (let i = 0; i < 30; i += 1) {
      stepStretchyHighlight(hl, 1 / 60, false);
    }

    targetStretchyHighlight(hl, { top: 4, left: 4, width: 208, height: 32 }, false);

    let peakHeight = 0;
    for (let i = 0; i < 60; i += 1) {
      const done = stepStretchyHighlight(hl, 1 / 60, false);
      peakHeight = Math.max(peakHeight, readStretchyHighlightMetrics(hl).height);
      if (done) break;
    }

    assert.ok(
      peakHeight > 42,
      `Expected upward stretchy highlight mid-flight height > 42px, got ${peakHeight}`
    );

    const settled = readStretchyHighlightMetrics(hl);
    assert.ok(Math.abs(settled.top - 4) <= 0.2);
    assert.ok(Math.abs(settled.height - 32) <= 0.2);
  });

  await t.test("4. Initial hover snaps coordinates without flying from y=0, and hide fades opacity", () => {
    const hl = createStretchyHighlight();
    targetStretchyHighlight(hl, { top: 96, left: 6, width: 180, height: 30 }, false);

    const m0 = readStretchyHighlightMetrics(hl);
    assert.strictEqual(m0.top, 96);
    assert.strictEqual(m0.height, 30);

    for (let i = 0; i < 30; i += 1) {
      stepStretchyHighlight(hl, 1 / 60, false);
    }
    hideStretchyHighlight(hl, false);
    for (let i = 0; i < 40; i += 1) {
      if (stepStretchyHighlight(hl, 1 / 60, false)) break;
    }
    assert.strictEqual(readStretchyHighlightMetrics(hl).opacity, 0);
  });

  await t.test("5. prefers-reduced-motion bypass settles immediately with zero stretch or overshoot", () => {
    const hl = createStretchyHighlight();
    targetStretchyHighlight(hl, { top: 4, left: 4, width: 208, height: 32 }, true);
    const first = readStretchyHighlightMetrics(hl);
    assert.strictEqual(first.top, 4);
    assert.strictEqual(first.height, 32);
    assert.strictEqual(first.opacity, 1);

    targetStretchyHighlight(hl, { top: 124, left: 4, width: 208, height: 32 }, true);
    const immediate = readStretchyHighlightMetrics(hl);
    assert.strictEqual(immediate.top, 124);
    assert.strictEqual(immediate.height, 32);
    assert.strictEqual(stepStretchyHighlight(hl, 1 / 60, true), true);
  });

  await t.test("6. cardTrail preset produces tighter, non-sticky glide than menu trail while settling accurately", () => {
    const menuHl = createStretchyHighlight();
    const cardHl = createStretchyHighlight();
    const row1 = { top: 6, left: 6, width: 308, height: 64 };
    const row2 = { top: 72, left: 6, width: 308, height: 64 };

    targetStretchyHighlight(menuHl, row1, false, SPRING_PRESETS.trail);
    targetStretchyHighlight(cardHl, row1, false, SPRING_PRESETS.cardTrail);
    for (let i = 0; i < 30; i += 1) {
      stepStretchyHighlight(menuHl, 1 / 60, false);
      stepStretchyHighlight(cardHl, 1 / 60, false);
    }

    targetStretchyHighlight(menuHl, row2, false, SPRING_PRESETS.trail);
    targetStretchyHighlight(cardHl, row2, false, SPRING_PRESETS.cardTrail);

    let maxMenuHeight = 0;
    let maxCardHeight = 0;
    for (let i = 0; i < 60; i += 1) {
      const mDone = stepStretchyHighlight(menuHl, 1 / 60, false);
      const cDone = stepStretchyHighlight(cardHl, 1 / 60, false);
      maxMenuHeight = Math.max(maxMenuHeight, readStretchyHighlightMetrics(menuHl).height);
      maxCardHeight = Math.max(maxCardHeight, readStretchyHighlightMetrics(cardHl).height);
      if (mDone && cDone) break;
    }

    assert.ok(
      maxCardHeight > 64 && maxCardHeight < maxMenuHeight,
      `Expected cardTrail peak stretch (${maxCardHeight.toFixed(2)}) between 64 and menu peak (${maxMenuHeight.toFixed(2)})`
    );

    const cardSettled = readStretchyHighlightMetrics(cardHl);
    assert.ok(Math.abs(cardSettled.top - 72) <= 0.2, `Expected card settled top ~72, got ${cardSettled.top}`);
    assert.ok(Math.abs(cardSettled.height - 64) <= 0.2, `Expected card settled height ~64, got ${cardSettled.height}`);
  });

  await t.test("7. pillCollapse preset collapses monotonically from 268px to 136px with zero undershoot or rebound", () => {
    const w = createSpring(268, SPRING_PRESETS.pillCollapse.k, SPRING_PRESETS.pillCollapse.c);
    w.t = 136;

    let prevX = 268;
    let minX = 268;
    for (let frame = 0; frame < 90; frame += 1) {
      stepSpringSubdivided(w, 1 / 60);
      assert.ok(w.x <= prevX + 1e-6, `Expected strictly monotonic shrink, got ${w.x} > ${prevX}`);
      minX = Math.min(minX, w.x);
      prevX = w.x;
      if (settleIfClose(w, 0.25, 0.8)) {
        break;
      }
    }

    assert.ok(minX >= 136 - 1e-6, `Expected zero undershoot below 136px, got minX=${minX}`);
    assert.strictEqual(w.x, 136);
    assert.strictEqual(w.v, 0);
  });
});
