import {
  STAGE1_DECK_LAYERS,
  STAGE1_DECK_SLOT_STEPS,
  STAGE1_HERO_SLOT_INDEX,
} from "./onboardingAtlasData";

export type Stage1Rng = () => number;

export interface Stage1TileFrame {
  deckIdx: number;
  placeSlotIdx: number;
  layerIdx: number;
}

export interface Stage1CanvasCell {
  prev: Stage1TileFrame | null;
  curr: Stage1TileFrame;
  fadeStartMs: number;
  stepsDoneInPlace: number;
  layerOrder: number[];
  introDelayMs: number;
  introDurationMs: number;
}

export interface PullbackCameraState {
  scale: number;
  tileSize: number;
  gap: number;
  originX: number;
  originY: number;
  vignetteAlpha: number;
  edgeMaskAlpha: number;
  neighborAlpha: number;
  heroAlpha: number;
  done: boolean;
}

/** `2.0×` speed relative to v10 baseline (`240ms` tick / `720ms` dissolve -> `120ms` / `360ms`). */
export const STAGE1_SPEED_MULTIPLIER = 2.0;
export const STAGE1_BASE_TICK_MS = 240;
export const STAGE1_BASE_DISSOLVE_MS = 720;

export const STAGE1_INTRO_TOTAL_MS = 1800;
export const STAGE1_TICK_MS = Math.round(STAGE1_BASE_TICK_MS / STAGE1_SPEED_MULTIPLIER);
export const STAGE1_DISSOLVE_MS = Math.round(STAGE1_BASE_DISSOLVE_MS / STAGE1_SPEED_MULTIPLIER);

function clamp01(v: number): number {
  return v <= 0 ? 0 : v >= 1 ? 1 : v;
}

/** Uniform exponential zoom velocity across 0..0.88 with a gentle C1 feather over 0.88..1.0. */
export function uniformZoomCurve(t: number): number {
  const p = clamp01(t);
  if (p <= 0) return 0;
  if (p >= 1) return 1;
  const linEnd = 0.88;
  const tail = 1 - linEnd;
  const slope = 1 / (linEnd + 0.5 * tail); // 1 / 0.94
  if (p <= linEnd) {
    return slope * p;
  }
  const u = (p - linEnd) / tail;
  return clamp01(slope * (linEnd + tail * (u - 0.5 * u * u)));
}

export function easeOutCubic(t: number): number {
  const p = clamp01(t);
  return 1 - Math.pow(1 - p, 3);
}

export function easeOutQuart(t: number): number {
  const p = clamp01(t);
  return 1 - Math.pow(1 - p, 4);
}

export function easeInOutSine(t: number): number {
  const p = clamp01(t);
  return 0.5 - 0.5 * Math.cos(Math.PI * p);
}

/** Per-tile Planet-style organic pop-in curve (fade + subtle 0.84 -> 1.0 settle). */
export function computeTileEmergence(
  elapsedMs: number,
  introDelayMs: number,
  introDurationMs: number
): { alpha: number; scale: number } {
  if (introDurationMs <= 0 || elapsedMs >= introDelayMs + introDurationMs) {
    return { alpha: 1, scale: 1 };
  }
  const p = clamp01((elapsedMs - introDelayMs) / introDurationMs);
  if (p <= 0) {
    return { alpha: 0, scale: 0.84 };
  }
  return {
    alpha: easeOutCubic(p),
    scale: 0.84 + 0.16 * easeOutQuart(p),
  };
}

export function shuffleWithRng<T>(items: readonly T[], rng: Stage1Rng = Math.random): T[] {
  const out = items.slice();
  for (let i = out.length - 1; i > 0; i--) {
    const j = Math.min(i, Math.max(0, Math.floor(rng() * (i + 1))));
    const tmp = out[i];
    out[i] = out[j];
    out[j] = tmp;
  }
  return out;
}

export function getStage1SlotMaxSteps(deckIdx: number, placeSlotIdx: number): number {
  const ch = STAGE1_DECK_SLOT_STEPS[deckIdx]?.[placeSlotIdx];
  const parsed = ch ? Number(ch) : 4;
  return Number.isFinite(parsed) && parsed >= 1 ? parsed : 4;
}

export function createShuffledLayerOrder(
  maxSteps: number,
  rng: Stage1Rng = Math.random,
  fixedFirstLayer?: number
): number[] {
  const safeMax = Math.max(1, maxSteps);
  const allLayers = Array.from({ length: safeMax }, (_, i) => i);
  if (fixedFirstLayer !== undefined && fixedFirstLayer >= 0 && fixedFirstLayer < safeMax) {
    const rest = shuffleWithRng(
      allLayers.filter((l) => l !== fixedFirstLayer),
      rng
    );
    return [fixedFirstLayer, ...rest];
  }
  return shuffleWithRng(allLayers, rng);
}

export function getOrthogonalNeighborLayers(
  cells: ReadonlyArray<Stage1CanvasCell | undefined>,
  cellIdx: number
): Set<number> {
  const col = cellIdx % 9;
  const row = Math.floor(cellIdx / 9);
  const out = new Set<number>();
  const coords: Array<[number, number]> = [
    [col - 1, row],
    [col + 1, row],
    [col, row - 1],
    [col, row + 1],
  ];
  for (const [nc, nr] of coords) {
    if (nc >= 0 && nc < 9 && nr >= 0 && nr < 4) {
      const nCell = cells[nr * 9 + nc];
      if (nCell) {
        out.add(nCell.curr.layerIdx);
      }
    }
  }
  return out;
}

export function createInitialStage1Cells(rng: Stage1Rng = Math.random): Stage1CanvasCell[] {
  const nonHeroSlots = Array.from({ length: 36 }, (_, i) => i).filter(
    (i) => i !== STAGE1_HERO_SLOT_INDEX
  );
  const shuffledPlaces = shuffleWithRng(nonHeroSlots, rng);
  const heroCol = STAGE1_HERO_SLOT_INDEX % 9;
  const heroRow = Math.floor(STAGE1_HERO_SLOT_INDEX / 9);
  let nonHeroCursor = 0;

  const cells: Array<Stage1CanvasCell | undefined> = Array(36).fill(undefined);
  const heroMaxSteps = getStage1SlotMaxSteps(0, STAGE1_HERO_SLOT_INDEX);
  const heroOrder = createShuffledLayerOrder(heroMaxSteps, rng, 0);
  cells[STAGE1_HERO_SLOT_INDEX] = {
    prev: null,
    curr: {
      deckIdx: 0,
      placeSlotIdx: STAGE1_HERO_SLOT_INDEX,
      layerIdx: heroOrder[0] ?? 0,
    },
    fadeStartMs: -STAGE1_DISSOLVE_MS,
    stepsDoneInPlace: 1,
    layerOrder: heroOrder,
    introDelayMs: 0,
    introDurationMs: 0,
  };

  const layerCounts = [1, 0, 0, 0];
  for (let idx = 0; idx < 36; idx++) {
    if (idx === STAGE1_HERO_SLOT_INDEX) continue;

    const placeSlotIdx = shuffledPlaces[nonHeroCursor++];
    const maxSteps = getStage1SlotMaxSteps(0, placeSlotIdx);
    const neighborLayers = getOrthogonalNeighborLayers(cells, idx);

    // Pick initial layer that differs from orthogonal neighbors and balances modality frequencies across 36 slots
    const layerOrder = createShuffledLayerOrder(maxSteps, rng);
    const rankedLayers = [...layerOrder].sort((a, b) => {
      const aClash = neighborLayers.has(a) ? 1 : 0;
      const bClash = neighborLayers.has(b) ? 1 : 0;
      if (aClash !== bClash) return aClash - bClash;
      return (layerCounts[a] ?? 0) - (layerCounts[b] ?? 0);
    });
    const layerIdx = rankedLayers[0] ?? 0;
    const stepsDoneInPlace = Math.min(maxSteps, 1 + Math.max(0, Math.floor(rng() * maxSteps)));
    const targetSlot = stepsDoneInPlace - 1;
    const pos = layerOrder.indexOf(layerIdx);
    if (pos >= 0 && pos !== targetSlot && targetSlot < layerOrder.length) {
      layerOrder[pos] = layerOrder[targetSlot];
      layerOrder[targetSlot] = layerIdx;
    }
    layerCounts[layerIdx] = (layerCounts[layerIdx] ?? 0) + 1;

    // Early scattered emergence so surrounding tiles enter promptly during uniform zoom-out and start flashing live
    const col = idx % 9;
    const row = Math.floor(idx / 9);
    const normDist = clamp01(Math.hypot(col - heroCol, (row - heroRow) * 1.4) / 5.5);
    const introDelayMs = Math.round(80 + normDist * 460 + rng() * 410);
    const introDurationMs = Math.round(340 + rng() * 120);

    cells[idx] = {
      prev: null,
      curr: {
        deckIdx: 0,
        placeSlotIdx,
        layerIdx,
      },
      fadeStartMs: -STAGE1_DISSOLVE_MS,
      stepsDoneInPlace,
      layerOrder,
      introDelayMs,
      introDurationMs,
    };
  }

  return cells as Stage1CanvasCell[];
}

export function createShuffledSlotOrder(
  rng: Stage1Rng = Math.random,
  avoidFirstIdx?: number
): number[] {
  const order = shuffleWithRng(
    Array.from({ length: 36 }, (_, i) => i),
    rng
  );
  if (avoidFirstIdx !== undefined && avoidFirstIdx >= 0 && avoidFirstIdx < 36) {
    const acol = avoidFirstIdx % 9;
    const arow = Math.floor(avoidFirstIdx / 9);
    const col0 = order[0] % 9;
    const row0 = Math.floor(order[0] / 9);
    if (Math.abs(col0 - acol) + Math.abs(row0 - arow) <= 1) {
      for (let k = 1; k < order.length; k++) {
        const ck = order[k] % 9;
        const rk = Math.floor(order[k] / 9);
        if (Math.abs(ck - acol) + Math.abs(rk - arow) > 1) {
          const tmp = order[0];
          order[0] = order[k];
          order[k] = tmp;
          break;
        }
      }
    }
  }
  return order;
}

export function createShuffledDeckOrders(
  rng: Stage1Rng = Math.random,
  deckCount: number = STAGE1_DECK_LAYERS.length
): number[][] {
  const base = Array.from({ length: 36 }, (_, i) => i);
  const count = Math.max(1, deckCount);
  return Array.from({ length: count }, () => shuffleWithRng(base, rng));
}

export function pickNextStage1Region(
  cur: Stage1TileFrame,
  loadedDecks: ReadonlySet<number>,
  shownKeys: Set<string>,
  deckOrders: number[][],
  deckCursors: number[],
  deckSwapCounts: number[],
  rng: Stage1Rng = Math.random
): { deckIdx: number; placeSlotIdx: number } {
  const maxDecks = deckOrders.length > 0 ? deckOrders.length : STAGE1_DECK_LAYERS.length;
  const decks = Array.from(loadedDecks).filter((d) => d >= 0 && d < maxDecks);
  if (decks.length >= 2) {
    const jitteredScore = new Map<number, number>();
    for (const d of decks) {
      jitteredScore.set(d, (deckSwapCounts[d] ?? 0) + Math.floor(rng() * 6));
    }
    const rankedDecks = [...decks].sort((a, b) => {
      const diff = (jitteredScore.get(a) ?? 0) - (jitteredScore.get(b) ?? 0);
      if (diff !== 0) return diff;
      if ((a === 0) !== (b === 0)) return a === 0 ? 1 : -1;
      return a - b;
    });
    for (const d of rankedDecks) {
      let order = deckOrders[d];
      if (!order || order.length === 0) {
        order = shuffleWithRng(
          Array.from({ length: 36 }, (_, i) => i),
          rng
        );
        deckOrders[d] = order;
      }
      for (let off = 0; off < 36; off++) {
        if ((deckCursors[d] ?? 0) >= order.length) {
          order = shuffleWithRng(order, rng);
          deckOrders[d] = order;
          deckCursors[d] = 0;
        }
        const cursor = deckCursors[d] ?? 0;
        const slotIdx = order[cursor];
        deckCursors[d] = cursor + 1;
        const key = `${d}:${slotIdx}`;
        if (!shownKeys.has(key)) {
          deckSwapCounts[d] = (deckSwapCounts[d] ?? 0) + 1;
          return { deckIdx: d, placeSlotIdx: slotIdx };
        }
      }
    }
  }
  const fallbackOrder = shuffleWithRng(
    deckOrders[0] ?? Array.from({ length: 36 }, (_, i) => i),
    rng
  );
  for (const slotIdx of fallbackOrder) {
    if (!shownKeys.has(`0:${slotIdx}`)) {
      return { deckIdx: 0, placeSlotIdx: slotIdx };
    }
  }
  return { deckIdx: 0, placeSlotIdx: (cur.placeSlotIdx + 13) % 36 };
}

export function advanceSingleStage1Cell(
  cells: Stage1CanvasCell[],
  cellIdx: number,
  _step: number,
  nowMs: number,
  loadedDecks: ReadonlySet<number>,
  deckOrders: number[][],
  deckCursors: number[],
  deckSwapCounts: number[],
  rng: Stage1Rng = Math.random,
  recentLayers: readonly number[] = []
): { mutatedCellIdx: number; rotatedRegion: boolean } {
  const cell = cells[cellIdx];
  const cur = cell.curr;
  const maxSteps = getStage1SlotMaxSteps(cur.deckIdx, cur.placeSlotIdx);
  const neighborLayers = getOrthogonalNeighborLayers(cells, cellIdx);
  const lastFlashedLayer = recentLayers[recentLayers.length - 1];
  const prevFlashedLayer = recentLayers[recentLayers.length - 2];

  const pickBestRemainingIndex = (order: number[], startCursor: number): number => {
    let bestPos = startCursor;
    let bestPenalty = Infinity;
    for (let pos = startCursor; pos < order.length; pos++) {
      const candidate = order[pos];
      if (candidate === cur.layerIdx) continue;
      let penalty = 0;
      if (candidate === lastFlashedLayer) penalty += 100;
      else if (candidate === prevFlashedLayer) penalty += 30;
      if (neighborLayers.has(candidate)) penalty += 10;
      if (penalty === 0) return pos;
      if (penalty < bestPenalty) {
        bestPenalty = penalty;
        bestPos = pos;
      }
    }
    return bestPos;
  };

  let nextFrame: Stage1TileFrame;
  let nextStepsDone: number;
  let nextLayerOrder: number[] = cell.layerOrder;
  let rotatedRegion = false;

  if (cell.stepsDoneInPlace < maxSteps) {
    const order =
      nextLayerOrder.length === maxSteps
        ? nextLayerOrder.slice()
        : createShuffledLayerOrder(maxSteps, rng, cur.layerIdx);
    const chosenPos = pickBestRemainingIndex(order, cell.stepsDoneInPlace);
    if (chosenPos !== cell.stepsDoneInPlace && chosenPos < order.length) {
      const tmp = order[cell.stepsDoneInPlace];
      order[cell.stepsDoneInPlace] = order[chosenPos];
      order[chosenPos] = tmp;
    }
    nextLayerOrder = order;
    const candidateLayer = order[cell.stepsDoneInPlace];
    const nextLayerIdx =
      candidateLayer !== undefined && candidateLayer !== cur.layerIdx
        ? candidateLayer
        : (cur.layerIdx + 1) % maxSteps;
    nextFrame = {
      deckIdx: cur.deckIdx,
      placeSlotIdx: cur.placeSlotIdx,
      layerIdx: nextLayerIdx,
    };
    nextStepsDone = cell.stepsDoneInPlace + 1;
  } else {
    const shownKeys = new Set<string>(
      cells.map((c) => `${c.curr.deckIdx}:${c.curr.placeSlotIdx}`)
    );
    shownKeys.delete(`${cur.deckIdx}:${cur.placeSlotIdx}`);
    const region = pickNextStage1Region(
      cur,
      loadedDecks,
      shownKeys,
      deckOrders,
      deckCursors,
      deckSwapCounts,
      rng
    );
    const regionMaxSteps = getStage1SlotMaxSteps(region.deckIdx, region.placeSlotIdx);
    const newOrder = createShuffledLayerOrder(regionMaxSteps, rng);
    const chosenPos = pickBestRemainingIndex(newOrder, 0);
    if (chosenPos !== 0 && chosenPos < newOrder.length) {
      const tmp = newOrder[0];
      newOrder[0] = newOrder[chosenPos];
      newOrder[chosenPos] = tmp;
    }
    nextLayerOrder = newOrder;
    nextFrame = {
      deckIdx: region.deckIdx,
      placeSlotIdx: region.placeSlotIdx,
      layerIdx: nextLayerOrder[0] ?? 0,
    };
    nextStepsDone = 1;
    rotatedRegion = true;
  }

  cells[cellIdx] = {
    prev: cur,
    curr: nextFrame,
    fadeStartMs: nowMs,
    stepsDoneInPlace: nextStepsDone,
    layerOrder: nextLayerOrder,
    introDelayMs: 0,
    introDurationMs: 0,
  };
  return { mutatedCellIdx: cellIdx, rotatedRegion };
}

export function computePullbackCamera(
  elapsedMs: number,
  cssW: number,
  cssH: number
): PullbackCameraState {
  const safeW = Math.max(cssW, 320);
  const safeH = Math.max(cssH, 240);
  const padX = safeW >= 640 ? 24 : 14;
  const outerW = Math.min(1660, safeW >= 768 ? safeW : Math.max(safeW * 1.25, 920));
  const gap = safeW >= 640 ? 7 : 6;
  const baseTile = Math.max(16, (outerW - padX * 2 - gap * 8) / 9);
  const wallW = baseTile * 9 + gap * 8;
  const wallH = baseTile * 4 + gap * 3;
  const restLeft = (safeW - wallW) / 2;
  const restTop = (safeH - wallH) / 2;

  const heroCol = STAGE1_HERO_SLOT_INDEX % 9;
  const heroRow = Math.floor(STAGE1_HERO_SLOT_INDEX / 9);
  const restHeroCx = restLeft + heroCol * (baseTile + gap) + baseTile * 0.5;
  const restHeroCy = restTop + heroRow * (baseTile + gap) + baseTile * 0.5;

  const startScale = Math.max(safeW / baseTile, safeH / baseTile) * 1.06;
  const rawP = clamp01(elapsedMs / STAGE1_INTRO_TOTAL_MS);
  // Uniform exponential zoom-out rate across nearly the entire pullback + tiny feather settle
  const easedP = uniformZoomCurve(rawP);

  const scale = Math.exp(Math.log(startScale) * (1 - easedP));
  const tileSize = baseTile * scale;
  const scaledGap = gap * scale;
  // Project center linearly in log-zoom space so the focal tile glides uniformly without drifting
  const heroCx = safeW * 0.5 + (restHeroCx - safeW * 0.5) * easedP;
  const heroCy = safeH * 0.5 + (restHeroCy - safeH * 0.5) * easedP;
  const originX = heroCx - heroCol * (tileSize + scaledGap) - tileSize * 0.5;
  const originY = heroCy - heroRow * (tileSize + scaledGap) - tileSize * 0.5;

  const vignetteAlpha = elapsedMs < 620 ? 0 : easeInOutSine((elapsedMs - 620) / 1000);
  const edgeMaskAlpha = elapsedMs <= 280 ? 0 : easeInOutSine((elapsedMs - 280) / 1100);
  const neighborRevealP = elapsedMs <= 120 ? 0 : easeOutCubic((elapsedMs - 120) / 1350);
  const neighborAlpha = 0.78 + (0.95 - 0.78) * neighborRevealP;
  const heroAlpha = 1 - 0.05 * easedP;

  return {
    scale,
    tileSize,
    gap: scaledGap,
    originX,
    originY,
    vignetteAlpha,
    edgeMaskAlpha,
    neighborAlpha,
    heroAlpha,
    done: elapsedMs >= STAGE1_INTRO_TOTAL_MS,
  };
}
