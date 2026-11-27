import React, { useEffect, useRef } from "react";
import {
  STAGE1_DECK_LAYERS,
  STAGE1_HERO_SLOT_INDEX,
  STAGE1_HERO_TILE_SRC,
  STAGE1_SPRITE_LAYERS,
} from "./onboardingAtlasData";
import {
  advanceSingleStage1Cell,
  computePullbackCamera,
  computeTileEmergence,
  createInitialStage1Cells,
  createShuffledDeckOrders,
  createShuffledSlotOrder,
  easeInOutSine,
  shuffleWithRng,
  STAGE1_DISSOLVE_MS,
  STAGE1_INTRO_TOTAL_MS,
  STAGE1_SPEED_MULTIPLIER,
  STAGE1_TICK_MS,
  type Stage1CanvasCell,
  type Stage1TileFrame,
} from "./stage1MosaicEngine";

type DecodedStage1Source = ImageBitmap | HTMLImageElement;

const BITMAP_CACHE = new Map<string, DecodedStage1Source>();
const BITMAP_INFLIGHT = new Map<string, Promise<DecodedStage1Source | null>>();
const LOADED_DECKS = new Set<number>();

function isSourceUsable(src: DecodedStage1Source | undefined): src is DecodedStage1Source {
  if (!src) return false;
  if (typeof ImageBitmap !== "undefined" && src instanceof ImageBitmap) {
    return src.width > 0 && src.height > 0;
  }
  const img = src as HTMLImageElement;
  return Boolean(img.complete && img.naturalWidth > 0);
}

function getImageConstructor(): typeof Image | null {
  if (typeof Image === "function") return Image;
  if (typeof window !== "undefined" && typeof window.Image === "function") {
    return window.Image;
  }
  return null;
}

function loadStage1Bitmap(url: string): Promise<DecodedStage1Source | null> {
  const cached = BITMAP_CACHE.get(url);
  if (isSourceUsable(cached)) {
    return Promise.resolve(cached);
  }
  const existing = BITMAP_INFLIGHT.get(url);
  if (existing) return existing;

  const ImageCtor = getImageConstructor();
  if (!ImageCtor) {
    return Promise.resolve(null);
  }

  const task = new Promise<DecodedStage1Source | null>((resolve) => {
    try {
      const img = new ImageCtor();
      img.decoding = "async";
      img.onload = () => {
        if (typeof createImageBitmap === "function") {
          void createImageBitmap(img)
            .then((bmp) => {
              BITMAP_CACHE.set(url, bmp);
              resolve(bmp);
            })
            .catch(() => {
              BITMAP_CACHE.set(url, img);
              resolve(img);
            });
        } else {
          BITMAP_CACHE.set(url, img);
          resolve(img);
        }
      };
      img.onerror = () => resolve(null);
      img.src = url;
    } catch {
      resolve(null);
    }
  }).finally(() => {
    BITMAP_INFLIGHT.delete(url);
  });

  BITMAP_INFLIGHT.set(url, task);
  return task;
}

function prefersReducedMotion(): boolean {
  return (
    typeof window !== "undefined" &&
    typeof window.matchMedia === "function" &&
    window.matchMedia("(prefers-reduced-motion: reduce)").matches
  );
}

function drawSpriteTile(
  ctx: CanvasRenderingContext2D,
  frame: Stage1TileFrame,
  dx: number,
  dy: number,
  tileSize: number,
  alpha: number,
  heroHighRes: DecodedStage1Source | undefined
): void {
  if (alpha <= 0.005) return;
  const spriteUrl = (STAGE1_DECK_LAYERS[frame.deckIdx] ?? STAGE1_SPRITE_LAYERS)[frame.layerIdx];
  const bmp = BITMAP_CACHE.get(spriteUrl);
  ctx.globalAlpha = alpha;

  // Use crisp 800x800 hero image when slot 13 is zoomed larger than a 200x200 sprite cell
  if (
    tileSize > 220 &&
    frame.deckIdx === 0 &&
    frame.placeSlotIdx === STAGE1_HERO_SLOT_INDEX &&
    frame.layerIdx === 0 &&
    isSourceUsable(heroHighRes)
  ) {
    const sw = heroHighRes instanceof HTMLImageElement ? heroHighRes.naturalWidth : heroHighRes.width;
    const sh = heroHighRes instanceof HTMLImageElement ? heroHighRes.naturalHeight : heroHighRes.height;
    ctx.drawImage(heroHighRes, 0, 0, sw, sh, dx, dy, tileSize, tileSize);
    return;
  }

  if (!isSourceUsable(bmp)) return;
  const col = frame.placeSlotIdx % 9;
  const row = Math.floor(frame.placeSlotIdx / 9);
  const fullW = bmp instanceof HTMLImageElement ? bmp.naturalWidth : bmp.width;
  const fullH = bmp instanceof HTMLImageElement ? bmp.naturalHeight : bmp.height;
  const cellW = fullW / 9;
  const cellH = fullH / 4;
  ctx.drawImage(bmp, col * cellW, row * cellH, cellW, cellH, dx, dy, tileSize, tileSize);
}

function syncCanvasDataset(
  canvas: HTMLCanvasElement,
  phase: "pullback" | "breathing" | "static",
  cells: readonly Stage1CanvasCell[],
  ticks: number
): void {
  const ds = canvas.dataset;
  ds.eoPhase = phase;
  ds.eoSlots = String(cells.length);
  ds.eoTicks = String(ticks);
  const decks = new Set<number>();
  const pairs = new Set<string>();
  for (const c of cells) {
    decks.add(c.curr.deckIdx);
    pairs.add(`${c.curr.deckIdx}:${c.curr.placeSlotIdx}`);
  }
  ds.eoDecks = Array.from(decks).sort((a, b) => a - b).join(",");
  ds.eoDistinctPairs = String(pairs.size);
  ds.eoSpeed = `${STAGE1_SPEED_MULTIPLIER.toFixed(1)}x`;
  ds.eoTickMs = String(STAGE1_TICK_MS);
  ds.eoDissolveMs = String(STAGE1_DISSOLVE_MS);
  ds.eoEdgeMask = "4-sided";
  ds.eoLayoutSig = cells
    .slice(0, 6)
    .map((c) => `${c.curr.deckIdx}:${c.curr.placeSlotIdx}:${c.curr.layerIdx}`)
    .join("|");
}

export interface Stage1MosaicCanvasProps {
  active: boolean;
}

export const Stage1MosaicCanvas: React.FC<Stage1MosaicCanvasProps> = ({ active }) => {
  const canvasRef = useRef<HTMLCanvasElement | null>(null);

  useEffect(() => {
    if (!active) return;
    const canvas = canvasRef.current;
    if (!canvas) return;
    let ctx: CanvasRenderingContext2D | null = null;
    try {
      ctx = canvas.getContext("2d", { alpha: false });
    } catch {
      ctx = null;
    }

    const reduced = prefersReducedMotion();
    const cells = createInitialStage1Cells(Math.random);
    const deckOrders = createShuffledDeckOrders(Math.random);
    const deckCursors = STAGE1_DECK_LAYERS.map(() => 0);
    const deckSwapCounts = STAGE1_DECK_LAYERS.map((_, d) => (d === 0 ? 36 : 0));
    let slotOrder = createShuffledSlotOrder(Math.random);
    let slotOrderCursor = 0;
    let lastFlippedIdx: number | undefined = undefined;
    let stepCount = 0;

    let cancelled = false;
    let rafId = 0;
    let clockStarted = false;
    let introStartMs = 0;
    let lastTickMs = 0;
    let currentPhase: "pullback" | "breathing" | "static" = reduced ? "static" : "pullback";

    let cssW = Math.max(window.innerWidth || 1280, 320);
    let cssH = Math.max(window.innerHeight || 800, 240);
    let dpr = Math.min(window.devicePixelRatio || 1, 1.5);
    let bgGrad: CanvasGradient | null = null;
    let vignetteGrad: CanvasGradient | null = null;
    let edgeTopGrad: CanvasGradient | null = null;
    let edgeBottomGrad: CanvasGradient | null = null;
    let edgeLeftGrad: CanvasGradient | null = null;
    let edgeRightGrad: CanvasGradient | null = null;
    let edgePadX = 180;
    let edgePadY = 140;

    const addObsidianStops = (grad: CanvasGradient) => {
      grad.addColorStop(0.0, "rgba(2, 4, 9, 0.99)");
      grad.addColorStop(0.24, "rgba(2, 4, 9, 0.88)");
      grad.addColorStop(0.52, "rgba(2, 4, 9, 0.48)");
      grad.addColorStop(0.78, "rgba(2, 4, 9, 0.14)");
      grad.addColorStop(1.0, "rgba(2, 4, 9, 0.0)");
    };

    const rebuildSize = () => {
      cssW = Math.max(window.innerWidth || 1280, 320);
      cssH = Math.max(window.innerHeight || 800, 240);
      dpr = Math.min(window.devicePixelRatio || 1, 1.5);
      canvas.width = Math.round(cssW * dpr);
      canvas.height = Math.round(cssH * dpr);
      if (!ctx) return;

      bgGrad = ctx.createRadialGradient(
        cssW * 0.5,
        cssH * 0.48,
        0,
        cssW * 0.5,
        cssH * 0.48,
        Math.hypot(cssW, cssH) * 0.62
      );
      bgGrad.addColorStop(0, "#070f1d");
      bgGrad.addColorStop(0.5, "#030710");
      bgGrad.addColorStop(1, "#020409");

      vignetteGrad = ctx.createRadialGradient(
        cssW * 0.5,
        cssH * 0.5,
        Math.min(cssW, cssH) * 0.06,
        cssW * 0.5,
        cssH * 0.5,
        Math.hypot(cssW, cssH) * 0.58
      );
      vignetteGrad.addColorStop(0, "rgba(4, 8, 16, 0.42)");
      vignetteGrad.addColorStop(0.52, "rgba(3, 6, 14, 0.24)");
      vignetteGrad.addColorStop(1, "rgba(2, 4, 9, 0.52)");

      edgePadX = Math.max(88, Math.min(270, Math.round(cssW * 0.17)));
      edgePadY = Math.max(72, Math.min(195, Math.round(cssH * 0.19)));

      edgeTopGrad = ctx.createLinearGradient(0, 0, 0, edgePadY);
      addObsidianStops(edgeTopGrad);

      edgeBottomGrad = ctx.createLinearGradient(0, cssH, 0, cssH - edgePadY);
      addObsidianStops(edgeBottomGrad);

      edgeLeftGrad = ctx.createLinearGradient(0, 0, edgePadX, 0);
      addObsidianStops(edgeLeftGrad);

      edgeRightGrad = ctx.createLinearGradient(cssW, 0, cssW - edgePadX, 0);
      addObsidianStops(edgeRightGrad);
    };

    const drawFrame = (nowMs: number, forceStatic = false) => {
      if (!ctx) return;
      const elapsed = forceStatic ? STAGE1_INTRO_TOTAL_MS + 100 : Math.max(0, nowMs - introStartMs);
      const cam = computePullbackCamera(elapsed, cssW, cssH);
      const heroBmp = BITMAP_CACHE.get(STAGE1_HERO_TILE_SRC);

      ctx.save();
      ctx.scale(dpr, dpr);
      ctx.globalAlpha = 1;
      if (bgGrad) {
        ctx.fillStyle = bgGrad;
      } else {
        ctx.fillStyle = "#020409";
      }
      ctx.fillRect(0, 0, cssW, cssH);

      const useClip = cam.tileSize < cssW * 0.92;
      const radius = Math.max(2, Math.min(12, 6 * cam.scale));
      const drawStroke = cam.scale < 2.2;
      const stride = cam.tileSize + cam.gap;

      for (let i = 0; i < 36; i++) {
        const col = i % 9;
        const row = Math.floor(i / 9);
        const dx = cam.originX + col * stride;
        const dy = cam.originY + row * stride;

        // Viewport culling: draws 1 tile at t=0, progressively reveals outward during pullback
        if (dx + cam.tileSize < 0 || dx > cssW || dy + cam.tileSize < 0 || dy > cssH) {
          continue;
        }

        const cell = cells[i];
        const emergence = forceStatic
          ? { alpha: 1, scale: 1 }
          : computeTileEmergence(elapsed, cell.introDelayMs, cell.introDurationMs);
        if (emergence.alpha <= 0.005) {
          continue;
        }

        const tileAlpha = (i === STAGE1_HERO_SLOT_INDEX ? cam.heroAlpha : cam.neighborAlpha) * emergence.alpha;
        const drawSize = cam.tileSize * emergence.scale;
        const tx = dx + (cam.tileSize - drawSize) * 0.5;
        const ty = dy + (cam.tileSize - drawSize) * 0.5;
        const tileRadius = radius * emergence.scale;

        ctx.save();
        if (useClip) {
          ctx.beginPath();
          if (typeof ctx.roundRect === "function") {
            ctx.roundRect(tx, ty, drawSize, drawSize, tileRadius);
          } else {
            ctx.rect(tx, ty, drawSize, drawSize);
          }
          ctx.clip();
        }

        ctx.fillStyle = "#060b14";
        ctx.fillRect(tx, ty, drawSize, drawSize);

        const fadeRaw = (nowMs - cell.fadeStartMs) / STAGE1_DISSOLVE_MS;
        if (cell.prev && !forceStatic && fadeRaw < 1) {
          const blend = easeInOutSine(fadeRaw);
          drawSpriteTile(ctx, cell.prev, tx, ty, drawSize, tileAlpha, heroBmp);
          drawSpriteTile(ctx, cell.curr, tx, ty, drawSize, tileAlpha * blend, heroBmp);
        } else {
          if (cell.prev && fadeRaw >= 1) {
            cell.prev = null;
          }
          drawSpriteTile(ctx, cell.curr, tx, ty, drawSize, tileAlpha, heroBmp);
        }

        ctx.restore();

        if (drawStroke) {
          ctx.save();
          ctx.globalAlpha = tileAlpha;
          ctx.strokeStyle = "rgba(255, 255, 255, 0.055)";
          ctx.lineWidth = 1;
          ctx.beginPath();
          if (typeof ctx.roundRect === "function") {
            ctx.roundRect(tx + 0.5, ty + 0.5, drawSize - 1, drawSize - 1, tileRadius);
          } else {
            ctx.rect(tx + 0.5, ty + 0.5, drawSize - 1, drawSize - 1);
          }
          ctx.stroke();
          ctx.restore();
        }
      }

      if (cam.vignetteAlpha > 0.005 && vignetteGrad) {
        ctx.globalAlpha = cam.vignetteAlpha;
        ctx.fillStyle = vignetteGrad;
        ctx.fillRect(0, 0, cssW, cssH);
      }

      // Planet-inspired 4-sided obsidian black feather mask melting outer mosaic edges into the dark frame
      const maskAlpha = forceStatic ? 1 : cam.edgeMaskAlpha;
      if (maskAlpha > 0.005) {
        ctx.globalAlpha = maskAlpha;
        if (edgeTopGrad) {
          ctx.fillStyle = edgeTopGrad;
          ctx.fillRect(0, 0, cssW, edgePadY);
        }
        if (edgeBottomGrad) {
          ctx.fillStyle = edgeBottomGrad;
          ctx.fillRect(0, cssH - edgePadY, cssW, edgePadY);
        }
        if (edgeLeftGrad) {
          ctx.fillStyle = edgeLeftGrad;
          ctx.fillRect(0, 0, edgePadX, cssH);
        }
        if (edgeRightGrad) {
          ctx.fillStyle = edgeRightGrad;
          ctx.fillRect(cssW - edgePadX, 0, edgePadX, cssH);
        }
      }

      ctx.restore();
    };

    rebuildSize();
    syncCanvasDataset(canvas, currentPhase, cells, stepCount);

    const onResize = () => {
      rebuildSize();
      if (reduced) {
        drawFrame(performance.now(), true);
      }
    };
    window.addEventListener("resize", onResize);

    // Non-graphical test environments (e.g. JSDOM without node-canvas) expose dataset state only.
    if (!ctx) {
      return () => {
        cancelled = true;
        window.removeEventListener("resize", onResize);
      };
    }

    // Kick off immediate load of hero-tile + Deck 0 sprites (capped at 250ms wait so animation never stalls)
    const deck0Ready = Promise.all([
      loadStage1Bitmap(STAGE1_HERO_TILE_SRC),
      ...STAGE1_SPRITE_LAYERS.map((u) => loadStage1Bitmap(u)),
    ]).then((res) => {
      if (res.slice(1).every(Boolean)) {
        LOADED_DECKS.add(0);
      }
    });

    if (reduced) {
      void deck0Ready.then(() => {
        if (cancelled) return;
        drawFrame(performance.now(), true);
        syncCanvasDataset(canvas, "static", cells, 0);
      });
      return () => {
        cancelled = true;
        window.removeEventListener("resize", onResize);
      };
    }

    const startLoop = () => {
      if (cancelled || clockStarted) return;
      clockStarted = true;
      introStartMs = performance.now();
      lastTickMs = introStartMs;
      const recentLayers: number[] = [];

      const pickActiveCell = (elapsed: number): number | null => {
        const cam = computePullbackCamera(elapsed, cssW, cssH);
        const stride = cam.tileSize + cam.gap;
        for (let attempt = 0; attempt < 36; attempt++) {
          if (slotOrderCursor >= slotOrder.length) {
            slotOrder = createShuffledSlotOrder(Math.random, lastFlippedIdx);
            slotOrderCursor = 0;
          }
          const idx = slotOrder[slotOrderCursor];
          slotOrderCursor += 1;
          if (elapsed < 550 && idx === STAGE1_HERO_SLOT_INDEX) continue;
          const col = idx % 9;
          const row = Math.floor(idx / 9);
          const dx = cam.originX + col * stride;
          const dy = cam.originY + row * stride;
          const inView = dx + cam.tileSize > 0 && dx < cssW && dy + cam.tileSize > 0 && dy < cssH;
          if (!inView) continue;
          const cell = cells[idx];
          const em = computeTileEmergence(elapsed, cell.introDelayMs, cell.introDurationMs);
          if (em.alpha >= 0.45) {
            return idx;
          }
        }
        return null;
      };

      const tick = (nowMs: number) => {
        if (cancelled) return;
        rafId = window.requestAnimationFrame(tick);

        if (typeof document !== "undefined" && document.visibilityState === "hidden") {
          return;
        }

        const elapsed = nowMs - introStartMs;
        if (elapsed >= STAGE1_INTRO_TOTAL_MS && currentPhase !== "breathing") {
          currentPhase = "breathing";
          syncCanvasDataset(canvas, "breathing", cells, stepCount);
        }

        // Flash emerging & visible tiles at regular cadence BOTH during zoom-out pullback and after settling
        if (elapsed >= STAGE1_TICK_MS && nowMs - lastTickMs >= STAGE1_TICK_MS) {
          lastTickMs = nowMs;
          const cellIdx = elapsed >= STAGE1_INTRO_TOTAL_MS
            ? (() => {
                if (slotOrderCursor >= slotOrder.length) {
                  slotOrder = createShuffledSlotOrder(Math.random, lastFlippedIdx);
                  slotOrderCursor = 0;
                }
                const idx = slotOrder[slotOrderCursor];
                slotOrderCursor += 1;
                return idx;
              })()
            : pickActiveCell(elapsed);

          if (cellIdx !== null) {
            lastFlippedIdx = cellIdx;
            stepCount += 1;
            advanceSingleStage1Cell(
              cells,
              cellIdx,
              stepCount,
              nowMs,
              LOADED_DECKS,
              deckOrders,
              deckCursors,
              deckSwapCounts,
              Math.random,
              recentLayers
            );
            recentLayers.push(cells[cellIdx].curr.layerIdx);
            if (recentLayers.length > 2) recentLayers.shift();
            syncCanvasDataset(
              canvas,
              elapsed >= STAGE1_INTRO_TOTAL_MS ? "breathing" : "pullback",
              cells,
              stepCount
            );
          }
        }

        drawFrame(nowMs, false);
      };

      drawFrame(introStartMs, false);
      rafId = window.requestAnimationFrame(tick);
    };

    const maxWaitTimer = window.setTimeout(startLoop, 250);
    void deck0Ready.then(() => {
      window.clearTimeout(maxWaitTimer);
      startLoop();
    });

    // Deferred & randomly shuffled background CDN preloader: starts after 1.75s pullback settles
    let preloadTimer: number | null = null;
    const waitMs = (ms: number): Promise<void> =>
      new Promise((resolve) => {
        preloadTimer = window.setTimeout(() => {
          preloadTimer = null;
          resolve();
        }, ms);
      });

    const preloadDeckOrder = shuffleWithRng(
      Array.from({ length: STAGE1_DECK_LAYERS.length - 1 }, (_, i) => i + 1),
      Math.random
    );

    void (async () => {
      await waitMs(2100);
      for (let i = 0; i < preloadDeckOrder.length; i++) {
        const d = preloadDeckOrder[i];
        if (cancelled) return;
        if (LOADED_DECKS.has(d)) continue;
        const urls = STAGE1_DECK_LAYERS[d];
        let allOk = true;
        for (let k = 0; k < urls.length; k++) {
          if (cancelled) return;
          const bmp = await loadStage1Bitmap(urls[k]);
          if (!bmp) allOk = false;
          if (k < urls.length - 1) {
            await waitMs(180);
          }
        }
        if (cancelled) return;
        if (allOk) {
          LOADED_DECKS.add(d);
        }
        if (i < preloadDeckOrder.length - 1) {
          await waitMs(1000);
        }
      }
    })();

    return () => {
      cancelled = true;
      window.clearTimeout(maxWaitTimer);
      if (preloadTimer !== null) {
        window.clearTimeout(preloadTimer);
      }
      if (rafId) {
        window.cancelAnimationFrame(rafId);
      }
      window.removeEventListener("resize", onResize);
    };
  }, [active]);

  return (
    <canvas
      ref={canvasRef}
      data-testid="welcome-eo-tile-mosaic"
      aria-hidden="true"
      className="pointer-events-none fixed inset-0 z-[2] h-full w-full"
    />
  );
};
