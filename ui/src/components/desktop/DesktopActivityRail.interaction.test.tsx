import "../../test-utils/setup-dom";
import assert from "node:assert/strict";
import test from "node:test";
import React, { useState } from "react";
import { act } from "react";
import { cleanup, fireEvent, render } from "../../test-utils/render-harness";
import { I18nProvider, LOCALE_STORAGE_KEY, setCurrentLocale, useI18n } from "../../i18n";
import { registerHooks } from "node:module";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { transformSync } from "esbuild";

// tsx does not provide Vite's import.meta.env. Patch only the module that reads it.
const scopedSource = (url: string) => `import.meta.env ??= { DEV: true };\n${readFileSync(fileURLToPath(url), "utf8")}`;
const hooks = registerHooks({
  load(url, context, nextLoad) {
    if (url.endsWith("/DesktopAgentWorkbench.tsx")) {
      return { format: "module", shortCircuit: true, source: transformSync(scopedSource(url), { loader: "tsx", format: "esm" }).code };
    }
    return nextLoad(url, context);
  },
});
const { DesktopActivityRail } = await import("./DesktopActivityRail");
const { DesktopWelcomeOnboarding, WELCOME_ONBOARDING_STORAGE_KEY } = await import("./DesktopWelcomeOnboarding");
const { ATLAS_COUNTRY_LIST, classifyOceanName, STAGE1_HERO_SLOT_INDEX } = await import("./onboarding/onboardingAtlasData");
const {
  advanceSingleStage1Cell,
  computePullbackCamera,
  computeTileEmergence,
  createInitialStage1Cells,
  createShuffledDeckOrders,
  createShuffledLayerOrder,
  createShuffledSlotOrder,
  STAGE1_DISSOLVE_MS,
  STAGE1_INTRO_TOTAL_MS,
  STAGE1_SPEED_MULTIPLIER,
  STAGE1_TICK_MS,
} = await import("./onboarding/stage1MosaicEngine");
hooks.deregister();

const toggles: boolean[] = [];

function Harness({ initialExpanded }: { initialExpanded: boolean }) {
  const [expanded, setExpanded] = useState(initialExpanded);
  return (
    <DesktopActivityRail
      activeRoute="workbench"
      onNavigate={() => {}}
      expanded={expanded}
      sidebarWidth={248}
      onToggleExpanded={() => {
        setExpanded((value) => {
          toggles.push(!value);
          return !value;
        });
      }}
    />
  );
}

const nav = (container: HTMLElement) => container.querySelector("nav") as HTMLElement;
const handle = (container: HTMLElement) => container.querySelector('[role="separator"]') as HTMLElement;
const move = async (clientX: number) => {
  await act(async () => {
    window.dispatchEvent(new window.MouseEvent("pointermove", { clientX }) as unknown as PointerEvent);
  });
};

test.afterEach(() => {
  cleanup();
  toggles.length = 0;
});

test("dragging follows the pointer 1:1, then animates to the 52px rail once the collapse threshold is crossed", async () => {
  const { container } = render(<Harness initialExpanded />);
  fireEvent.pointerDown(handle(container), { button: 0, clientX: 248 });

  await move(300);
  assert.equal(nav(container).style.width, "300px");
  assert.match(nav(container).className, /transition-none/, "live drag above the threshold must not lag behind the pointer");

  await move(140);
  assert.equal(nav(container).style.width, "52px");
  assert.equal(nav(container).dataset.sidebarState, "collapsed");
  assert.match(nav(container).className, /transition-\[width\]/, "crossing the threshold animates the width instead of snapping");

  // hysteresis: moving just above 150 does not flip back
  await move(160);
  assert.equal(nav(container).style.width, "52px");

  // moving far enough back re-expands to the dragged width
  await move(200);
  assert.equal(nav(container).dataset.sidebarState, "expanded");
  assert.equal(nav(container).style.width, "200px");

  await move(120);
  await act(async () => {
    window.dispatchEvent(new window.MouseEvent("pointerup", { clientX: 120 }) as unknown as PointerEvent);
  });
  assert.deepEqual(toggles, [false]);
  assert.equal(nav(container).style.width, "52px");
});

test("dragging outward from the collapsed rail animates to the remembered width", async () => {
  const { container } = render(<Harness initialExpanded={false} />);
  fireEvent.pointerDown(handle(container), { button: 0, clientX: 52 });

  await move(100);
  assert.equal(nav(container).style.width, "52px");

  await move(140);
  assert.equal(nav(container).style.width, "248px");
  assert.equal(nav(container).dataset.sidebarState, "expanded");
  assert.match(nav(container).className, /transition-\[width\]/);
  assert.ok(container.querySelector('[aria-label="全局命令与工作区面板"]'), "expanded labels mount immediately and fade in");

  await act(async () => {
    window.dispatchEvent(new window.MouseEvent("pointerup", { clientX: 140 }) as unknown as PointerEvent);
  });
  assert.deepEqual(toggles, [true]);
});

test("keyboard Enter on the resize handle resets and expands the collapsed rail", async () => {
  const { container } = render(<Harness initialExpanded={false} />);
  assert.equal(nav(container).style.width, "52px");

  fireEvent.keyDown(handle(container), { key: "Enter" });
  assert.deepEqual(toggles, [true]);
  assert.equal(nav(container).style.width, "248px");
  assert.match(nav(container).className, /transition-\[width\]/);
});

test("collapsed rail icons keep the same 15px scale as expanded icons without bloating", () => {
  const { container } = render(<Harness initialExpanded={false} />);
  assert.equal(nav(container).dataset.sidebarState, "collapsed");
  const svgs = Array.from(nav(container).querySelectorAll("button svg"));
  assert.ok(svgs.length >= 7, "collapsed rail renders new-chat, workbench, catalog, downloads, history, plugins, settings");
  for (const svg of svgs) {
    const cls = svg.getAttribute("class") ?? "";
    assert.match(cls, /h-\[15px\]\s+w-\[15px\]/, `collapsed icon must stay 15px, got: ${cls}`);
    assert.doesNotMatch(cls, /\bh-5\b|\bw-5\b/, `collapsed icon must not bloat to 20px, got: ${cls}`);
  }
  assert.ok(nav(container).querySelector('[data-testid="rail-nav-data-catalog"]'));
});

test("top-left search button opens GlobalCommandPalette and settings defaults to appearance instead of models", () => {
  window.localStorage.removeItem("eostage.shell.lastSettingsCategory");
  let commandPaletteOpens = 0;
  const openedCategories: string[] = [];
  const navigatedRoutes: string[] = [];

  const { getByTestId, container } = render(
    <DesktopActivityRail
      activeRoute="workbench"
      expanded={true}
      sidebarWidth={248}
      onNavigate={(route) => navigatedRoutes.push(route)}
      onOpenCommandPalette={() => {
        commandPaletteOpens += 1;
      }}
      onOpenSettingsCategory={(cat) => openedCategories.push(cat)}
    />
  );

  fireEvent.click(getByTestId("rail-open-command-palette-btn"));
  assert.equal(commandPaletteOpens, 1);

  fireEvent.click(getByTestId("rail-open-settings-btn"));
  assert.deepEqual(openedCategories, ["appearance"]);
  assert.ok(navigatedRoutes.includes("settings"));
});

test("expanded activity rail exposes unified spring rail attributes, stretchy nav highlight layer, and cascading row wrappers", () => {
  const { getByTestId, container } = render(
    <DesktopActivityRail
      activeRoute="workbench"
      expanded={true}
      sidebarWidth={248}
      onNavigate={() => {}}
    />
  );
  const navEl = nav(container);
  assert.equal(navEl.getAttribute("data-spring-rail"), "true");
  assert.ok(getByTestId("rail-spring-highlight"), "shared stretchy nav highlight layer mounted");
  const staggerWraps = navEl.querySelectorAll("[data-rail-spring-row]");
  assert.ok(staggerWraps.length >= 7, `expected header + 6 primary nav rows wrapped for stagger, got ${staggerWraps.length}`);
  const fadeLabels = navEl.querySelectorAll("[data-rail-fade]");
  assert.ok(fadeLabels.length >= 8, `expected brand header, 6 nav labels, session container, and footer help marked with data-rail-fade for smooth collapse dissolve, got ${fadeLabels.length}`);
  assert.equal(navEl.querySelector("svg[viewBox='2 1.5 97 97']"), null, "Sidebar header omits duplicate brand SVG icon per owner decision");
  assert.ok(navEl.classList.contains("sp-quiet-sidebar"), "sidebar uses sunken frosted-gray surface");
  assert.ok(!navEl.classList.contains("border-r"), "sidebar removes hard right border in favor of main plane leftward shadow");
  const springHighlight = getByTestId("rail-spring-highlight");
  assert.ok(springHighlight.classList.contains("rounded-[9px]"), "stretchy nav highlight uses 9px pill radius");
});

test("primary nav icon buttons keep identical DOM identity across expand and collapse without unmounting or disappearing", async () => {
  function StatefulHarness() {
    const [expanded, setExpanded] = useState(true);
    return (
      <>
        <button type="button" data-testid="ext-toggle" onClick={() => setExpanded((v) => !v)}>
          toggle
        </button>
        <DesktopActivityRail activeRoute="workbench" onNavigate={() => {}} expanded={expanded} sidebarWidth={248} />
      </>
    );
  }
  const { getByTestId, container } = render(<StatefulHarness />);
  const initialWorkbenchBtn = container.querySelector('button[aria-label="地图工作台"]');
  const initialCatalogBtn = container.querySelector('[data-testid="rail-nav-data-catalog"]');
  const initialSettingsBtn = container.querySelector('[data-testid="rail-open-settings-btn"]');
  assert.ok(initialWorkbenchBtn && initialCatalogBtn && initialSettingsBtn);

  await act(async () => {
    fireEvent.click(getByTestId("ext-toggle"));
  });
  await new Promise((r) => setTimeout(r, 320));

  assert.equal(container.querySelector('button[aria-label="地图工作台"]'), initialWorkbenchBtn);
  assert.equal(container.querySelector('[data-testid="rail-nav-data-catalog"]'), initialCatalogBtn);
  assert.equal(container.querySelector('[data-testid="rail-open-settings-btn"]'), initialSettingsBtn);
  assert.equal(container.querySelectorAll("[data-rail-fade]").length, 0);
});

test("collapsing spring rail in browser mode keeps data-rail-fade faded at opacity 0 until atomic unmount with zero post-collapse text flash", async () => {
  const originalOffsetWidth = Object.getOwnPropertyDescriptor(HTMLElement.prototype, "offsetWidth");
  Object.defineProperty(HTMLElement.prototype, "offsetWidth", {
    configurable: true,
    get() {
      if ((this as HTMLElement).tagName === "NAV") {
        const state = (this as HTMLElement).dataset.sidebarState;
        return state === "collapsed" ? 52 : 248;
      }
      return 120;
    },
  });

  const rafQueue: FrameRequestCallback[] = [];
  let now = 1000;
  const origRaf = window.requestAnimationFrame;
  const origCancelRaf = window.cancelAnimationFrame;
  const origPerfNow = performance.now.bind(performance);
  window.requestAnimationFrame = ((cb: FrameRequestCallback) => {
    rafQueue.push(cb);
    return rafQueue.length;
  }) as typeof window.requestAnimationFrame;
  window.cancelAnimationFrame = ((id: number) => {
    if (id > 0 && id <= rafQueue.length) {
      rafQueue[id - 1] = () => {};
    }
  }) as typeof window.cancelAnimationFrame;
  performance.now = () => now;

  try {
    let toggleFn: (() => void) | null = null;
    function ToggleHarness() {
      const [expanded, setExpanded] = useState(true);
      toggleFn = () => setExpanded((prev) => !prev);
      return (
        <DesktopActivityRail
          activeRoute="workbench"
          onNavigate={() => {}}
          expanded={expanded}
          sidebarWidth={248}
          onToggleExpanded={() => setExpanded((prev) => !prev)}
        />
      );
    }

    const { container } = render(<ToggleHarness />);
    const navEl = nav(container);
    assert.equal(navEl.dataset.sidebarState, "expanded");

    await act(async () => {
      toggleFn?.();
    });

    // Advance spring frames until collapse finishes; check that every frame while expanded fadeEls stay hidden as p drops and never flash opacity "" before unmounting
    let sawZeroOpacityBeforeUnmount = false;
    for (let step = 0; step < 80 && rafQueue.length > 0; step += 1) {
      now += 16;
      const cb = rafQueue.shift()!;
      await act(async () => {
        cb(now);
      });
      const stillExpanded = container.querySelectorAll("[data-rail-fade]");
      if (stillExpanded.length > 0) {
        const firstOp = (stillExpanded[0] as HTMLElement).style.opacity;
        if (firstOp === "0.000") sawZeroOpacityBeforeUnmount = true;
        // Once text has faded to 0, it must NEVER revert to "" or > 0 while expanded sheet is still mounted
        if (sawZeroOpacityBeforeUnmount) {
          assert.equal(firstOp, "0.000", `Unexpected text opacity flash before unmount: "${firstOp}"`);
        }
      } else {
        break;
      }
    }

    assert.equal(container.querySelectorAll("[data-rail-fade]").length, 0, "Expanded sheet must unmount immediately on settle");
    assert.equal(navEl.style.width, "52px");
  } finally {
    performance.now = origPerfNow;
    window.requestAnimationFrame = origRaf;
    window.cancelAnimationFrame = origCancelRaf;
    if (originalOffsetWidth) {
      Object.defineProperty(HTMLElement.prototype, "offsetWidth", originalOffsetWidth);
    } else {
      delete (HTMLElement.prototype as { offsetWidth?: number }).offsetWidth;
    }
  }
});

test("stalled requestAnimationFrame settles sidebar width deterministically via timer fallback without losing icons", async () => {
  const originalOffsetWidth = Object.getOwnPropertyDescriptor(HTMLElement.prototype, "offsetWidth");
  Object.defineProperty(HTMLElement.prototype, "offsetWidth", {
    configurable: true,
    get() {
      if ((this as HTMLElement).tagName === "NAV") {
        const state = (this as HTMLElement).dataset.sidebarState;
        return state === "collapsed" ? 52 : 248;
      }
      return 120;
    },
  });

  const origRaf = window.requestAnimationFrame;
  const origCancelRaf = window.cancelAnimationFrame;
  window.requestAnimationFrame = (() => 1) as typeof window.requestAnimationFrame;
  window.cancelAnimationFrame = (() => {}) as typeof window.cancelAnimationFrame;

  try {
    let toggleFn: (() => void) | null = null;
    function StalledHarness() {
      const [expanded, setExpanded] = useState(false);
      toggleFn = () => setExpanded((prev) => !prev);
      return (
        <DesktopActivityRail
          activeRoute="workbench"
          onNavigate={() => {}}
          expanded={expanded}
          sidebarWidth={248}
          onToggleExpanded={() => setExpanded((prev) => !prev)}
        />
      );
    }

    const { container } = render(<StalledHarness />);
    const navEl = nav(container);
    assert.equal(navEl.style.width, "52px");

    await act(async () => {
      toggleFn?.();
    });
    await act(async () => {
      await new Promise((r) => setTimeout(r, 520));
    });
    assert.equal(navEl.style.width, "248px", "stalled rAF must still expand nav width to 248px via timer guard");
    assert.ok(container.querySelector('[aria-label="全局命令与工作区面板"]'));

    await act(async () => {
      toggleFn?.();
    });
    await act(async () => {
      await new Promise((r) => setTimeout(r, 520));
    });
    assert.equal(navEl.style.width, "52px", "stalled rAF must collapse nav width back to 52px via timer guard");
    assert.ok(container.querySelector('[aria-label="地图工作台"]'), "primary icons remain mounted when collapsed");
    assert.ok(container.querySelector('[aria-label="下载中心"]'));
    assert.ok(container.querySelector('[aria-label="设置"]'));
  } finally {
    window.requestAnimationFrame = origRaf;
    window.cancelAnimationFrame = origCancelRaf;
    if (originalOffsetWidth) {
      Object.defineProperty(HTMLElement.prototype, "offsetWidth", originalOffsetWidth);
    } else {
      delete (HTMLElement.prototype as { offsetWidth?: number }).offsetWidth;
    }
  }
});

test("expanding spring rail from 52px to 248px morphs active highlight monotonically with zero post-settle snap-back twitch", async () => {
  const originalOffsetWidth = Object.getOwnPropertyDescriptor(HTMLElement.prototype, "offsetWidth");
  const originalOffsetHeight = Object.getOwnPropertyDescriptor(HTMLElement.prototype, "offsetHeight");
  const originalGetBoundingClientRect = HTMLElement.prototype.getBoundingClientRect;

  Object.defineProperty(HTMLElement.prototype, "offsetWidth", {
    configurable: true,
    get() {
      const el = this as HTMLElement;
      if (el.tagName === "NAV") {
        return el.dataset.sidebarState === "collapsed" ? 52 : 248;
      }
      if (el.tagName === "BUTTON" && el.getAttribute("aria-label") === "地图工作台") {
        const navAncestor = el.closest("nav");
        return navAncestor?.dataset.sidebarState === "collapsed" ? 30 : 228;
      }
      return 120;
    },
  });
  Object.defineProperty(HTMLElement.prototype, "offsetHeight", {
    configurable: true,
    get() {
      return 30;
    },
  });
  HTMLElement.prototype.getBoundingClientRect = function () {
    const el = this as HTMLElement;
    if (el.tagName === "BUTTON" && el.getAttribute("aria-label") === "地图工作台") {
      const navAncestor = el.closest("nav");
      const expanded = navAncestor?.dataset.sidebarState !== "collapsed";
      return {
        left: expanded ? 10 : 11,
        top: 38,
        right: expanded ? 238 : 41,
        bottom: 68,
        width: expanded ? 228 : 30,
        height: 30,
        x: expanded ? 10 : 11,
        y: 38,
        toJSON: () => ({}),
      } as DOMRect;
    }
    return {
      left: 0,
      top: 0,
      right: 248,
      bottom: 600,
      width: 248,
      height: 600,
      x: 0,
      y: 0,
      toJSON: () => ({}),
    } as DOMRect;
  };

  const rafMap = new Map<number, FrameRequestCallback>();
  let nextRafId = 1;
  let now = 2000;
  const origRaf = window.requestAnimationFrame;
  const origCancelRaf = window.cancelAnimationFrame;
  const origPerfNow = performance.now.bind(performance);
  window.requestAnimationFrame = ((cb: FrameRequestCallback) => {
    const id = nextRafId++;
    rafMap.set(id, cb);
    return id;
  }) as typeof window.requestAnimationFrame;
  window.cancelAnimationFrame = ((id: number) => {
    rafMap.delete(id);
  }) as typeof window.cancelAnimationFrame;
  performance.now = () => now;

  try {
    let toggleFn: (() => void) | null = null;
    function ExpandHarness() {
      const [expanded, setExpanded] = useState(false);
      toggleFn = () => setExpanded((prev) => !prev);
      return (
        <DesktopActivityRail
          activeRoute="workbench"
          onNavigate={() => {}}
          expanded={expanded}
          sidebarWidth={248}
          onToggleExpanded={() => setExpanded((prev) => !prev)}
        />
      );
    }

    const { container, getByTestId, unmount } = render(<ExpandHarness />);
    const navEl = nav(container);
    const pillEl = getByTestId("rail-spring-highlight") as HTMLElement;
    assert.equal(navEl.dataset.sidebarState, "collapsed");
    assert.equal(parseFloat(pillEl.style.width || "0"), 30);

    await act(async () => {
      toggleFn?.();
    });

    const sampledWidths: number[] = [];
    for (let step = 0; step < 120 && rafMap.size > 0; step += 1) {
      const entries = Array.from(rafMap.entries());
      rafMap.clear();
      now += 16;
      await act(async () => {
        for (const [, cb] of entries) {
          cb(now);
        }
      });
      const w = parseFloat(pillEl.style.width || "0");
      sampledWidths.push(w);
    }

    assert.ok(sampledWidths.length >= 2, `Expected multiple spring frames during expand, got ${sampledWidths.length}`);
    for (let i = 1; i < sampledWidths.length; i += 1) {
      assert.ok(
        sampledWidths[i] >= sampledWidths[i - 1] - 0.05,
        `Active highlight pill twitched backward on expand frame ${i}: ${sampledWidths[i - 1]} -> ${sampledWidths[i]} (${sampledWidths.join(", ")})`
      );
    }
    assert.equal(parseFloat(pillEl.style.width || "0"), 228, "Settled expanded highlight width must stay 228px");
    assert.equal(parseFloat(navEl.style.width || "0"), 248, "Settled expanded sidebar width must stay 248px");
    assert.equal(rafMap.size, 0, "Zero residual RAF loops may run after expand settles");
    unmount();
  } finally {
    performance.now = origPerfNow;
    window.requestAnimationFrame = origRaf;
    window.cancelAnimationFrame = origCancelRaf;
    HTMLElement.prototype.getBoundingClientRect = originalGetBoundingClientRect;
    if (originalOffsetWidth) {
      Object.defineProperty(HTMLElement.prototype, "offsetWidth", originalOffsetWidth);
    } else {
      delete (HTMLElement.prototype as { offsetWidth?: number }).offsetWidth;
    }
    if (originalOffsetHeight) {
      Object.defineProperty(HTMLElement.prototype, "offsetHeight", originalOffsetHeight);
    } else {
      delete (HTMLElement.prototype as { offsetHeight?: number }).offsetHeight;
    }
  }
});

test("hovering primary nav items on agent route clears rail-spring-highlight to opacity 0 on mouseLeave and route switch even when rAF stalls", async () => {
  const originalOffsetWidth = Object.getOwnPropertyDescriptor(HTMLElement.prototype, "offsetWidth");
  const originalOffsetHeight = Object.getOwnPropertyDescriptor(HTMLElement.prototype, "offsetHeight");
  Object.defineProperty(HTMLElement.prototype, "offsetWidth", {
    configurable: true,
    get() {
      const el = this as HTMLElement;
      if (el.tagName === "NAV") return 248;
      if (el.tagName === "BUTTON") return 232;
      return 248;
    },
  });
  Object.defineProperty(HTMLElement.prototype, "offsetHeight", {
    configurable: true,
    get() {
      return 30;
    },
  });

  const origRaf = window.requestAnimationFrame;
  const origCancelRaf = window.cancelAnimationFrame;
  window.requestAnimationFrame = (() => 1) as typeof window.requestAnimationFrame;
  window.cancelAnimationFrame = (() => {}) as typeof window.cancelAnimationFrame;

  try {
    function RouteHoverHarness() {
      const [route, setRoute] = useState<"agent" | "workbench" | "settings">("agent");
      const [sessionId, setSessionId] = useState<string | null>(null);
      return (
        <>
          <button type="button" data-testid="set-session" onClick={() => setSessionId("s-1")}>session</button>
          <button type="button" data-testid="go-settings" onClick={() => setRoute("settings")}>settings</button>
          <button type="button" data-testid="go-agent" onClick={() => setRoute("agent")}>agent</button>
          <DesktopActivityRail
            activeRoute={route}
            activeSessionId={sessionId}
            onNavigate={(next) => {
              if (next === "agent" || next === "workbench" || next === "settings") setRoute(next);
            }}
            expanded={true}
            sidebarWidth={248}
          />
        </>
      );
    }

    const { container, getByTestId, unmount } = render(<RouteHoverHarness />);
    let pillEl = getByTestId("rail-spring-highlight") as HTMLElement;
    assert.equal(parseFloat(pillEl.style.opacity || "0"), 0, "new agent route must not pin + 新建会话 as a stuck gray shadow block");

    const wbBtn = container.querySelector('button[aria-label="地图工作台"]') as HTMLButtonElement;
    assert.ok(wbBtn);
    await act(async () => {
      fireEvent.mouseEnter(wbBtn);
    });
    await act(async () => {
      await new Promise((r) => setTimeout(r, 250));
    });
    assert.equal(parseFloat(pillEl.style.opacity || "0"), 1, "hovering nav item makes highlight visible");

    await act(async () => {
      fireEvent.mouseLeave(wbBtn, { relatedTarget: null });
    });
    await act(async () => {
      await new Promise((r) => setTimeout(r, 260));
    });
    assert.equal(parseFloat(pillEl.style.opacity || "0"), 0, "leaving nav area must deterministically clear shadow block to opacity 0 even when rAF stalls");

    // Verify workbench -> settings -> agent route switch also keeps pill hidden
    await act(async () => {
      fireEvent.click(getByTestId("set-session"));
      fireEvent.click(wbBtn);
    });
    await act(async () => {
      fireEvent.click(getByTestId("go-settings"));
    });
    await act(async () => {
      fireEvent.click(getByTestId("go-agent"));
    });
    pillEl = getByTestId("rail-spring-highlight") as HTMLElement;
    assert.equal(parseFloat(pillEl.style.opacity || "0"), 0, "switching workbench -> settings -> agent must leave rail highlight hidden");
    unmount();
  } finally {
    window.requestAnimationFrame = origRaf;
    window.cancelAnimationFrame = origCancelRaf;
    if (originalOffsetWidth) {
      Object.defineProperty(HTMLElement.prototype, "offsetWidth", originalOffsetWidth);
    } else {
      delete (HTMLElement.prototype as { offsetWidth?: number }).offsetWidth;
    }
    if (originalOffsetHeight) {
      Object.defineProperty(HTMLElement.prototype, "offsetHeight", originalOffsetHeight);
    } else {
      delete (HTMLElement.prototype as { offsetHeight?: number }).offsetHeight;
    }
  }
});

test("DesktopWelcomeOnboarding separates Page-1 9x4 mosaic setup from Page-2 177-country 3D Earth Explorer with arrows-for-places and dots-for-modalities", async () => {
  window.localStorage.removeItem(WELCOME_ONBOARDING_STORAGE_KEY);
  let closed = 0;
  let pickedCache = 0;
  let resetCache = 0;

  const { getByTestId, queryByTestId, findByTestId } = render(
    <DesktopWelcomeOnboarding
      open={true}
      onClose={() => {
        closed += 1;
      }}
      cacheDir=""
      installRootDir="/Applications/EOStage"
      onPickCacheDir={() => {
        pickedCache += 1;
      }}
      onResetCacheDir={() => {
        resetCache += 1;
      }}
    />
  );

  // Page 1: Pure 9x4 sprite mosaic + centered glass Language/Cache setup + Skip + Continue (no Earth, no Sparkles pill)
  assert.ok(getByTestId("desktop-welcome-onboarding"));
  assert.equal(queryByTestId("welcome-enter-atlas-pill"), null, "removed Sparkles atlas prompt pill");
  assert.equal(queryByTestId("welcome-atlas-webgl-canvas"), null, "3D Earth is strictly separated onto Page 2");
  assert.ok(getByTestId("welcome-stage1-foreground"), "Page 1 setup foreground rendered");
  const iconImg = getByTestId("welcome-real-app-icon") as HTMLImageElement;
  assert.ok(iconImg.getAttribute("src")?.includes("/app-symbol-256.png"), "must render finalized C-2 EOStage icon");

  const mosaic = getByTestId("welcome-eo-tile-mosaic");
  assert.equal(mosaic.tagName, "CANVAS", "Page 1 background uses hardware-accelerated canvas");
  assert.equal((mosaic.textContent || "").trim(), "", "Page 1 background tiles contain zero burned text");
  assert.equal(mosaic.getAttribute("data-eo-slots"), "36", "renders 9x4 = 36 distinct global EO slots");
  assert.equal(mosaic.getAttribute("data-eo-distinct-pairs"), "36", "36 distinct places displayed on launch");
  assert.ok(
    (getByTestId("welcome-stage1-foreground").textContent || "").includes(
      "多源遥感检索、空间预览与智能下载的一站式地球观测工作台。"
    ),
    "renders updated precise Stage-1 tagline"
  );

  assert.ok(getByTestId("welcome-card-language"), "language card mounted");
  assert.ok(getByTestId("welcome-card-cache"), "cache directory card mounted");
  assert.equal(queryByTestId("welcome-card-readiness-sheet"), null, "no extra setup cards beyond language and cache");
  assert.ok(
    (getByTestId("welcome-cache-path-display").textContent || "").includes("EOStage › cache"),
    "default cache directory uses installation directory"
  );

  fireEvent.click(getByTestId("welcome-pick-cache-btn"));
  assert.equal(pickedCache, 1);

  fireEvent.click(getByTestId("welcome-lang-trigger"));
  assert.ok(getByTestId("welcome-lang-menu"));
  fireEvent.click(getByTestId("welcome-lang-option-zh-CN"));

  assert.equal((getByTestId("welcome-onboarding-continue-btn").textContent || "").trim(), "探索 3D 地球");
  assert.ok(!(getByTestId("welcome-onboarding-continue-btn").textContent || "").includes("↩"), "continue button omits extra return symbol");

  // Click Continue -> Page 2 3D Interactive Earth Explorer
  fireEvent.click(getByTestId("welcome-onboarding-continue-btn"));
  assert.equal(queryByTestId("welcome-stage1-foreground"), null, "Page 1 disappears on Continue");
  assert.ok(getByTestId("welcome-atlas-webgl-canvas"), "Page 2 mounts WebGL 3D Earth");
  assert.equal(
    getByTestId("welcome-atlas-webgl-canvas").getAttribute("data-eo-earth-shader"),
    "atlas-v4-crisp-space",
    "WebGL Earth runs atlas-v4-crisp-space shader"
  );
  assert.ok(getByTestId("welcome-atlas-hover-label"), "Stage 2 mounts country/ocean hover label");
  assert.ok(!(getByTestId("welcome-stage2-enter-app-btn").textContent || "").includes("↩"), "enter-app button omits extra return symbol");

  // Refined ocean bounds classifier distinguishes South China Sea / Western Pacific vs Indian, Atlantic, Arctic, Southern oceans
  assert.equal(classifyOceanName(15, 118, "zh-CN"), "太平洋");
  assert.equal(classifyOceanName(15, 118, "en"), "Pacific Ocean");
  assert.equal(classifyOceanName(-20, 80, "zh-CN"), "印度洋");
  assert.equal(classifyOceanName(25, -45, "zh-CN"), "大西洋");
  assert.equal(classifyOceanName(24, -90, "zh-CN"), "大西洋");
  assert.equal(classifyOceanName(75, 0, "zh-CN"), "北冰洋");
  assert.equal(classifyOceanName(-65, 120, "zh-CN"), "南大洋");

  const wordmark = getByTestId("welcome-atlas-wordmark");
  assert.ok((wordmark.textContent || "").includes("Stage"), "Stage 2 keeps consistent lowercase 'tage' wordmark");
  assert.ok(!(wordmark.textContent || "").includes("STAGE"), "Stage 2 does not uppercase to STAGE");

  // Continent drawer exposes merged 176 countries (Taiwan inside China) with zero decorative stars
  fireEvent.click(getByTestId("welcome-continent-toggle"));
  const continentDrawer = getByTestId("welcome-continent-drawer");
  assert.ok(continentDrawer, "collapsible continent drawer mounted");
  assert.ok(!(continentDrawer.textContent || "").includes("★"), "country picker renders zero stars");
  assert.equal(queryByTestId("welcome-country-pick-TWN"), null, "Taiwan is merged into China instead of a standalone entry");

  const aus = ATLAS_COUNTRY_LIST.find((c) => c.iso3 === "AUS");
  assert.ok(aus && aus.places.length >= 5, "Australia expanded with >= 5 classic landforms");
  const usa = ATLAS_COUNTRY_LIST.find((c) => c.iso3 === "USA");
  assert.ok(usa && usa.places.some((p) => p.id === "texas_pivots"), "USA includes classic Texas circular center-pivot fields");
  assert.equal(
    ATLAS_COUNTRY_LIST.some((c) => c.places.some((p) => p.label["zh-CN"].includes("区域地貌"))),
    false,
    "no generic 区域地貌 place titles remain"
  );
  assert.equal(
    ATLAS_COUNTRY_LIST.some((c) =>
      c.places.some((p) =>
        p.modalities.some(
          (m) =>
            m.label["zh-CN"].includes("高光谱") ||
            (m.capturedAt || "").includes("1954") ||
            m.key.startsWith("insar")
        )
      )
    ),
    false,
    "catalog has zero Hyperspectral SWIR, 1954 DEM, or InSAR leftovers"
  );

  // Filter resets cleanly when reopening continent drawer after selecting a country
  const filterInput = getByTestId("welcome-continent-filter-input") as HTMLInputElement;
  fireEvent.change(filterInput, { target: { value: "中国" } });
  assert.equal(queryByTestId("welcome-continent-tab-europe"), null, "typing filter hides continent tabs");

  fireEvent.click(getByTestId("welcome-country-pick-CHN"));

  // Open Country: zero top sensor pill buttons; hover label remains mounted; side panel dynamically centers/clamps; Launch App stays visible; < > switches data modalities; numbered rows switch places
  const sidePanel = getByTestId("welcome-atlas-side-panel");
  const dialogText = getByTestId("desktop-welcome-onboarding").textContent || "";
  assert.ok(sidePanel, "clicking a country opens inspector panel");
  assert.ok(sidePanel.className.includes("md:justify-center"), "side panel dynamically centers when place list is short and clamps cleanly at top ceiling");
  assert.ok(getByTestId("welcome-atlas-hover-label"), "hover label stays mounted even while a country panel is open");
  assert.equal(queryByTestId("welcome-sensor-pills"), null, "removed bulky top sensor pill buttons from imagery card");
  assert.ok(getByTestId("welcome-stage2-enter-app-btn"), "Launch App stays visible bottom-right while country panel is open");
  assert.ok(getByTestId("welcome-panel-place-row-alishan_fan"), "China list includes Taiwan landmarks");
  assert.ok(!dialogText.includes("遥感观象"), "country tagline contains pure landform text without 遥感观象");
  assert.ok(!(sidePanel.textContent || "").includes("干涉带") && !(sidePanel.textContent || "").includes("夜光带"), "place titles contain pure landforms");
  const captionPill = getByTestId("welcome-active-modality-caption");
  assert.equal(captionPill.tagName, "DIV", "modality caption badge is non-clickable text");
  assert.match(captionPill.textContent || "", /\d{4}/, "modality caption shows capture date/year");

  const modalityDots = getByTestId("welcome-modality-dots");
  const dotButtons = Array.from(modalityDots.querySelectorAll("button"));
  assert.ok(dotButtons.length >= 2, "bottom interactive dot bar switches data modalities");
  assert.equal(dotButtons[0].getAttribute("aria-pressed"), "true");

  fireEvent.click(getByTestId("welcome-modality-next-btn"));
  assert.equal(dotButtons[1].getAttribute("aria-pressed"), "true", "right chevron cycles data modality in sync with bottom dots");

  // Numbered list row switches active landmark place inside China
  const secondRow = getByTestId("welcome-panel-place-row-poyang_lake");
  fireEvent.click(secondRow);
  assert.ok(secondRow.className.includes("opacity-100"), "clicking numbered list row selects landmark place");

  // Full-bleed view opens cleanly and Esc closes one level at a time
  fireEvent.click(getByTestId("welcome-atlas-expand-btn"));
  const fullBleed = getByTestId("welcome-atlas-fullbleed");
  assert.ok(fullBleed, "expand button opens full-bleed HD view");
  assert.ok(getByTestId("welcome-fullbleed-modality-dots"), "full-bleed reuses bottom modality dots");
  fireEvent.keyDown(window, { key: "Escape" });

  // Back to globe -> reopening continent drawer resets filter -> Back to setup -> Skip completes onboarding
  fireEvent.click(getByTestId("welcome-atlas-back-btn"));
  const continentBtnAfterReturn = await findByTestId("welcome-continent-toggle", {}, { timeout: 2000 });
  fireEvent.click(continentBtnAfterReturn);
  assert.equal(
    (getByTestId("welcome-continent-filter-input") as HTMLInputElement).value,
    "",
    "continent filter resets cleanly after picking a country"
  );
  assert.ok(getByTestId("welcome-continent-tab-europe"), "continent tabs are visible again on reopen");
  const backSetupBtn = getByTestId("welcome-atlas-back-setup-btn");
  fireEvent.click(backSetupBtn);
  assert.ok(getByTestId("welcome-stage1-foreground"), "back button returns cleanly to Page 1 setup");

  fireEvent.click(getByTestId("welcome-onboarding-skip-btn"));
  assert.equal(closed, 1);
  assert.equal(resetCache, 0);
  assert.equal(window.localStorage.getItem(WELCOME_ONBOARDING_STORAGE_KEY), "1");
});

test("DesktopWelcomeOnboarding prefers defaultCacheDir over installRootDir when cacheDir is empty", () => {
  window.localStorage.setItem(LOCALE_STORAGE_KEY, "zh-CN");
  const realAppCache = "C:\\Users\\Alice\\AppData\\Roaming\\InSAR Studio\\v3\\cache";
  const releaseInstallRoot = "F:\\EOStage-verify\\target\\release";

  const { getByTestId } = render(
    <I18nProvider>
      <DesktopWelcomeOnboarding
        open={true}
        onClose={() => {}}
        cacheDir=""
        defaultCacheDir={realAppCache}
        installRootDir={releaseInstallRoot}
      />
    </I18nProvider>
  );

  const pathEl = getByTestId("welcome-cache-path-display");
  assert.equal(pathEl.getAttribute("title"), realAppCache);
  assert.ok(!(pathEl.getAttribute("title") || "").includes("target\\release"));
  assert.ok((pathEl.textContent || "").includes("v3 › cache"));
});

test("DesktopActivityRail updates workspace subtitle and command palette aria-label when switching to ja", () => {
  window.localStorage.setItem(LOCALE_STORAGE_KEY, "zh-CN");
  setCurrentLocale("zh-CN");
  function RailLangHarness({ loggedIn = false }: { loggedIn?: boolean }) {
    const { setLocale } = useI18n();
    return (
      <div>
        <button type="button" data-testid="rail-switch-ja" onClick={() => setLocale("ja")}>
          ja
        </button>
        <DesktopActivityRail
          activeRoute="workbench"
          onNavigate={() => {}}
          expanded={true}
          accountStatus={
            loggedIn
              ? { logged_in: true, email: "qa@example.com", total_credits: 120, user_id: 1, tier: "free" } as any
              : undefined
          }
        />
      </div>
    );
  }
  try {
    const { getByTestId, container, unmount } = render(
      <I18nProvider>
        <RailLangHarness />
      </I18nProvider>,
    );
    assert.ok((container.textContent || "").includes("遥感智能工作台"));
    fireEvent.click(getByTestId("rail-switch-ja"));
    assert.equal((container.textContent || "").includes("遥感智能工作台"), false);
    assert.ok((container.textContent || "").includes("リモートセンシング作業台"));
    assert.ok(container.querySelector('[aria-label="コマンド＆ワークスペースパレット"]'));

    const accountBtn = getByTestId("rail-account-btn");
    const aboutBtn = getByTestId("rail-open-about-btn");
    const accountText = `${accountBtn.getAttribute("aria-label") || ""} ${accountBtn.getAttribute("title") || ""} ${accountBtn.textContent || ""}`;
    const aboutText = `${aboutBtn.getAttribute("aria-label") || ""} ${aboutBtn.getAttribute("title") || ""}`;
    assert.equal(accountText.includes("登录 EOStage 账号"), false);
    assert.equal(accountText.includes("登录 EOStage"), false);
    assert.equal(aboutText.includes("快捷键与帮助"), false);
    assert.equal(aboutText.includes("关于 EOStage"), false);
    assert.ok(accountText.includes("EOStage アカウントにログイン"));
    assert.ok(aboutText.includes("ショートカットとヘルプ"));
    assert.ok(aboutText.includes("EOStage について"));
    unmount();

    // Logged-in account popover under ja
    window.localStorage.setItem(LOCALE_STORAGE_KEY, "zh-CN");
    setCurrentLocale("zh-CN");
    const loggedInView = render(
      <I18nProvider>
        <RailLangHarness loggedIn={true} />
      </I18nProvider>,
    );
    fireEvent.click(loggedInView.getByTestId("rail-switch-ja"));
    const loggedInAccountBtn = loggedInView.getByTestId("rail-account-btn");
    assert.ok((loggedInAccountBtn.getAttribute("aria-label") || "").includes("EOStage アカウント qa@example.com"));
    fireEvent.click(loggedInAccountBtn);
    const popover = loggedInView.getByTestId("rail-account-popover");
    const popText = popover.textContent || "";
    const refreshBtn = loggedInView.getByTestId("rail-account-refresh-btn");
    assert.equal(refreshBtn.getAttribute("aria-label"), "Credits 残高を更新");
    assert.equal(refreshBtn.getAttribute("title"), "残高を更新");
    for (const forbidden of ["可用", "余额与账单", "AI 用量明细", "官方模型设置", "退出登录"]) {
      assert.equal(popText.includes(forbidden), false, `account popover leaked Simplified Chinese: ${forbidden}`);
    }
    for (const expected of ["利用可能 120 Credits", "残高と請求", "AI 使用明細", "公式モデル設定", "ログアウト"]) {
      assert.ok(popText.includes(expected), `expected account popover ja text: ${expected}`);
    }
  } finally {
    cleanup();
    window.localStorage.setItem(LOCALE_STORAGE_KEY, "zh-CN");
    setCurrentLocale("zh-CN");
    document.documentElement.lang = "zh-CN";
  }
});

test("Stage-1 canvas engine computes uniform-rate zoom-out at 2.0x cadence, Planet tile emergence, anti-clump random layers, and 4-sided edge mask", () => {
  // 0. 2.0x speed cadence parameters (`120ms` tick / `360ms` dissolve)
  assert.equal(STAGE1_SPEED_MULTIPLIER, 2.0);
  assert.equal(STAGE1_TICK_MS, 120);
  assert.equal(STAGE1_DISSOLVE_MS, 360);
  assert.equal(STAGE1_INTRO_TOTAL_MS, 1800);

  // 1. Uniform-rate exponential pullback camera + 4-sided edge mask ramp
  const c0 = computePullbackCamera(0, 1440, 900);
  const c200 = computePullbackCamera(200, 1440, 900);
  const c600 = computePullbackCamera(600, 1440, 900);
  const c1000 = computePullbackCamera(1000, 1440, 900);
  const c1400 = computePullbackCamera(1400, 1440, 900);
  const cEnd = computePullbackCamera(STAGE1_INTRO_TOTAL_MS, 1440, 900);

  assert.ok(c0.scale >= 8, `t=0 hero scale covers full viewport edge-to-edge, got ${c0.scale}`);
  assert.equal(c0.vignetteAlpha, 0, "vignette stays off at opening hero");
  assert.equal(c0.edgeMaskAlpha, 0, "4-sided edge feather mask ramps in smoothly after pullback starts");
  // Exponential zoom-out rate d(ln scale)/dt must be uniform across equal 400ms slices
  const d1 = Math.log(c200.scale) - Math.log(c600.scale);
  const d2 = Math.log(c600.scale) - Math.log(c1000.scale);
  const d3 = Math.log(c1000.scale) - Math.log(c1400.scale);
  assert.ok(Math.abs(d1 - d2) / d1 < 0.01 && Math.abs(d2 - d3) / d2 < 0.01, `zoom velocity must be uniform across slices: d1=${d1}, d2=${d2}, d3=${d3}`);
  assert.ok(c1000.neighborAlpha > c200.neighborAlpha && cEnd.neighborAlpha > c1000.neighborAlpha, "neighbor tiles reveal progressively");
  assert.equal(cEnd.scale, 1, "settled camera reaches scale=1 cleanly");
  assert.equal(cEnd.done, true);
  assert.equal(cEnd.vignetteAlpha, 1);
  assert.equal(cEnd.edgeMaskAlpha, 1);

  // Per-tile emergence scales 0.84 -> 1.0 and fades 0 -> 1 cleanly
  const e0 = computeTileEmergence(100, 300, 500);
  const eMid = computeTileEmergence(550, 300, 500);
  const eEnd = computeTileEmergence(800, 300, 500);
  assert.deepEqual(e0, { alpha: 0, scale: 0.84 });
  assert.ok(eMid.alpha > 0 && eMid.alpha < 1 && eMid.scale > 0.84 && eMid.scale < 1);
  assert.deepEqual(eEnd, { alpha: 1, scale: 1 });

  // 2. Seeded RNG helper verifies true launch-to-launch randomization cleanly without flaky tests
  const makeRng = (seed: number) => {
    let s = seed >>> 0;
    return () => {
      s = (Math.imul(1664525, s) + 1013904223) >>> 0;
      return s / 0x100000000;
    };
  };

  const rngA = makeRng(1337);
  const rngB = makeRng(9001);
  const cellsA = createInitialStage1Cells(rngA);
  const cellsB = createInitialStage1Cells(rngB);

  assert.equal(cellsA.length, 36);
  assert.deepEqual(cellsA[STAGE1_HERO_SLOT_INDEX].curr, {
    deckIdx: 0,
    placeSlotIdx: STAGE1_HERO_SLOT_INDEX,
    layerIdx: 0,
  });
  assert.equal(cellsA[STAGE1_HERO_SLOT_INDEX].stepsDoneInPlace, 1);
  assert.equal(cellsA[STAGE1_HERO_SLOT_INDEX].introDelayMs, 0);
  assert.equal(cellsA[STAGE1_HERO_SLOT_INDEX].introDurationMs, 0);
  assert.deepEqual(cellsB[STAGE1_HERO_SLOT_INDEX].curr, {
    deckIdx: 0,
    placeSlotIdx: STAGE1_HERO_SLOT_INDEX,
    layerIdx: 0,
  });
  assert.equal(new Set(cellsA.map((c) => c.curr.placeSlotIdx)).size, 36, "all 36 initial places are unique");

  const sig = (list: typeof cellsA) =>
    list.map((c) => `${c.curr.deckIdx}:${c.curr.placeSlotIdx}:${c.curr.layerIdx}`).join(",");
  assert.notEqual(sig(cellsA), sig(cellsB), "distinct launches randomize outer 35 tile placements & modalities");

  // Randomized modality permutation covers all layers without fixed monotonic order
  const modSeqsA = [0, 1, 2, 3].map((k) => createShuffledLayerOrder(4, makeRng(100 + k)));
  for (const seq of modSeqsA) {
    assert.deepEqual([...seq].sort((a, b) => a - b), [0, 1, 2, 3], "modality order visits every layer once");
  }
  const modSeqsB = [0, 1, 2, 3].map((k) => createShuffledLayerOrder(4, makeRng(700 + k)));
  assert.notDeepEqual(modSeqsA, modSeqsB, "distinct launches shuffle per-place modality orders");

  const orderA = createShuffledSlotOrder(rngA);
  const orderB = createShuffledSlotOrder(rngB);
  assert.notDeepEqual(orderA, orderB, "distinct launches produce different flip sequences");

  // 3. Anti-clumping bag boundary avoids orthogonally adjacent next tile on 9x4 grid
  for (const avoidIdx of [0, 13, 18, 35]) {
    const bag = createShuffledSlotOrder(makeRng(42 + avoidIdx), avoidIdx);
    const manhattan =
      Math.abs((bag[0] % 9) - (avoidIdx % 9)) +
      Math.abs(Math.floor(bag[0] / 9) - Math.floor(avoidIdx / 9));
    assert.ok(manhattan > 1, `first tile ${bag[0]} must not touch previous tile ${avoidIdx}`);
  }

  // 4. Randomized multi-deck stepping across 60 ticks: 1 tile per tick, zero duplicates, multi-CDN decks, zero layer repeat
  const loadedDecks = new Set([0, 1, 2, 3, 4, 5, 6]);
  const deckOrdersA = createShuffledDeckOrders(rngA, 7);
  const deckCursorsA = Array(7).fill(0);
  const deckSwapCountsA = [36, 0, 0, 0, 0, 0, 0];

  const deckOrdersB = createShuffledDeckOrders(rngB, 7);
  const deckCursorsB = Array(7).fill(0);
  const deckSwapCountsB = [36, 0, 0, 0, 0, 0, 0];

  const beforeSnap = cellsA.map((c) => `${c.curr.deckIdx}:${c.curr.placeSlotIdx}:${c.curr.layerIdx}`);
  const res1 = advanceSingleStage1Cell(
    cellsA,
    orderA[0],
    1,
    2500,
    loadedDecks,
    deckOrdersA,
    deckCursorsA,
    deckSwapCountsA,
    rngA
  );
  assert.equal(res1.mutatedCellIdx, orderA[0]);
  assert.equal(
    cellsA.filter((c, idx) => `${c.curr.deckIdx}:${c.curr.placeSlotIdx}:${c.curr.layerIdx}` !== beforeSnap[idx]).length,
    1,
    "exactly 1 single tile mutates per tick"
  );

  let rotations = res1.rotatedRegion ? 1 : 0;
  const cdnDecksSeen = new Set<number>();
  let bagA = orderA;
  let bagCursorA = 1;
  let bagB = orderB;
  let bagCursorB = 0;
  const recentLayersA: number[] = [cellsA[orderA[0]].curr.layerIdx];
  let repeatBursts = 0;

  for (let s = 2; s <= 60; s++) {
    if (bagCursorA >= bagA.length) {
      bagA = createShuffledSlotOrder(rngA, bagA[bagA.length - 1]);
      bagCursorA = 0;
    }
    const idxA = bagA[bagCursorA++];
    const prevCell = cellsA[idxA].curr;
    const prevGlobalLayer = recentLayersA[recentLayersA.length - 1];
    const r = advanceSingleStage1Cell(
      cellsA,
      idxA,
      s,
      2500 + s * STAGE1_TICK_MS,
      loadedDecks,
      deckOrdersA,
      deckCursorsA,
      deckSwapCountsA,
      rngA,
      recentLayersA
    );
    const nextCell = cellsA[idxA].curr;
    if (nextCell.deckIdx === prevCell.deckIdx && nextCell.placeSlotIdx === prevCell.placeSlotIdx) {
      assert.notEqual(nextCell.layerIdx, prevCell.layerIdx, "in-place modality step never repeats the same layer");
    }
    if (nextCell.layerIdx === prevGlobalLayer) repeatBursts += 1;
    recentLayersA.push(nextCell.layerIdx);
    if (recentLayersA.length > 2) recentLayersA.shift();
    if (r.rotatedRegion) rotations += 1;
    if (nextCell.deckIdx >= 1) cdnDecksSeen.add(nextCell.deckIdx);

    if (s <= 20) {
      if (bagCursorB >= bagB.length) {
        bagB = createShuffledSlotOrder(rngB, bagB[bagB.length - 1]);
        bagCursorB = 0;
      }
      advanceSingleStage1Cell(
        cellsB,
        bagB[bagCursorB++],
        s,
        2500 + s * STAGE1_TICK_MS,
        loadedDecks,
        deckOrdersB,
        deckCursorsB,
        deckSwapCountsB,
        rngB
      );
    }
  }
  assert.ok(repeatBursts <= 8, `anti-clump logic suppresses back-to-back same-layer bursts below pure chance, got ${repeatBursts}`);
  assert.ok(rotations >= 12, `expected cells to rotate across global regions, got ${rotations}`);
  const distinctPairs = new Set(cellsA.map((c) => `${c.curr.deckIdx}:${c.curr.placeSlotIdx}`));
  assert.equal(distinctPairs.size, 36, "zero duplicate regions visible across the 36 slots simultaneously");
  assert.ok(cdnDecksSeen.size >= 2, `expected multiple randomized CDN decks >= 1, got ${Array.from(cdnDecksSeen)}`);
  assert.notEqual(sig(cellsA), sig(cellsB), "distinct launches diverge throughout randomized playback");

  // 5. Mounted reduced-motion, 1.0x baseline attributes & zero heavy backdrop-blur
  const origMatchMedia = window.matchMedia;
  window.matchMedia = ((query: string) => ({
    matches: query.includes("prefers-reduced-motion"),
    media: query,
    onchange: null,
    addListener: () => {},
    removeListener: () => {},
    addEventListener: () => {},
    removeEventListener: () => {},
    dispatchEvent: () => false,
  })) as typeof window.matchMedia;

  try {
    const { getByTestId } = render(<DesktopWelcomeOnboarding open={true} onClose={() => {}} />);
    const canvas = getByTestId("welcome-eo-tile-mosaic");
    assert.equal(canvas.tagName, "CANVAS");
    assert.equal(canvas.getAttribute("data-eo-phase"), "static", "reduced-motion enters static canvas mode");
    assert.equal(canvas.getAttribute("data-eo-speed"), "2.0x");
    assert.equal(canvas.getAttribute("data-eo-tick-ms"), "120");
    assert.equal(canvas.getAttribute("data-eo-dissolve-ms"), "360");
    assert.equal(canvas.getAttribute("data-eo-edge-mask"), "4-sided");
    assert.ok(
      getByTestId("welcome-card-language").className.includes("backdrop-blur-[14px]"),
      "language card preserves lightweight translucent frosted glass"
    );
    assert.ok(
      getByTestId("welcome-card-language").className.includes("bg-white/[0.70]"),
      "language card uses translucent glass surface over canvas"
    );
    assert.equal(
      getByTestId("welcome-card-language").className.includes("backdrop-blur-2xl"),
      false,
      "language card avoids heavy 40px backdrop-blur-2xl GPU stall"
    );
    assert.ok(
      getByTestId("welcome-card-cache").className.includes("backdrop-blur-[14px]"),
      "cache card preserves lightweight translucent frosted glass"
    );
    assert.equal(
      getByTestId("welcome-card-cache").className.includes("backdrop-blur-2xl"),
      false,
      "cache card avoids heavy 40px backdrop-blur-2xl GPU stall"
    );
  } finally {
    window.matchMedia = origMatchMedia;
  }
});

test("collapsing rail and returning from settings to map workbench keep ? button visible without RAF jank", async () => {
  const originalOffsetWidth = Object.getOwnPropertyDescriptor(HTMLElement.prototype, "offsetWidth");
  const originalOffsetHeight = Object.getOwnPropertyDescriptor(HTMLElement.prototype, "offsetHeight");
  Object.defineProperty(HTMLElement.prototype, "offsetWidth", {
    configurable: true,
    get() {
      if ((this as HTMLElement).tagName === "NAV") {
        return (this as HTMLElement).dataset.sidebarState === "collapsed" ? 52 : 248;
      }
      return 120;
    },
  });
  Object.defineProperty(HTMLElement.prototype, "offsetHeight", {
    configurable: true,
    get() {
      return 44;
    },
  });

  try {
    let setHarnessState: ((next: { route: "workbench" | "settings"; expanded: boolean }) => void) | null = null;
    function CollapseHarness() {
      const [state, setState] = useState<{ route: "workbench" | "settings"; expanded: boolean }>({
        route: "workbench",
        expanded: true,
      });
      setHarnessState = setState;
      return (
        <DesktopActivityRail
          activeRoute={state.route}
          onNavigate={() => {}}
          expanded={state.expanded}
          sidebarWidth={248}
          onToggleExpanded={() => setState((prev) => ({ ...prev, expanded: !prev.expanded }))}
        />
      );
    }

    const { container, getByTestId } = render(<CollapseHarness />);
    const navEl = nav(container);
    assert.equal(navEl.style.width, "248px");

    // 1. Enter settings (expanded), then return directly to collapsed map workbench -> snaps immediately to 52px
    await act(async () => {
      setHarnessState?.({ route: "settings", expanded: true });
    });
    assert.equal(navEl.style.width, "248px");

    await act(async () => {
      setHarnessState?.({ route: "workbench", expanded: false });
    });
    assert.equal(navEl.style.width, "52px", "returning from settings to collapsed map workbench snaps directly to 52px");

    const aboutBtn = getByTestId("rail-open-about-btn") as HTMLElement;
    const aboutWrap = aboutBtn.parentElement as HTMLElement;
    assert.ok(!aboutWrap.hasAttribute("data-rail-fade"), "help button wrapper must never carry data-rail-fade");
    assert.notEqual(aboutWrap.style.opacity, "0");
    assert.notEqual(aboutWrap.style.opacity, "0.000");
    assert.notEqual(aboutWrap.style.width, "0px");
  } finally {
    if (originalOffsetWidth) {
      Object.defineProperty(HTMLElement.prototype, "offsetWidth", originalOffsetWidth);
    } else {
      delete (HTMLElement.prototype as { offsetWidth?: number }).offsetWidth;
    }
    if (originalOffsetHeight) {
      Object.defineProperty(HTMLElement.prototype, "offsetHeight", originalOffsetHeight);
    } else {
      delete (HTMLElement.prototype as { offsetHeight?: number }).offsetHeight;
    }
  }
});

test("Issue #48: primary nav preserves 新对话 -> 地图工作台 -> 下载中心 -> 数据清单 -> 全部对话 -> 插件与技能 order in expanded and collapsed states", async () => {
  const expectedOrder = ["新对话", "地图工作台", "下载中心", "数据清单", "全部对话", "插件与技能"];
  for (const expanded of [true, false]) {
    const { container } = render(
      <DesktopActivityRail
        activeRoute="downloads"
        onNavigate={() => {}}
        expanded={expanded}
        sidebarWidth={248}
      />
    );
    const primaryNav = container.querySelector('[data-hovered-nav]');
    assert.ok(primaryNav, "primary nav container should render");
    const labels = Array.from(primaryNav.querySelectorAll("button"))
      .map((btn) => btn.getAttribute("aria-label") || "")
      .filter((label) => expectedOrder.includes(label));
    assert.deepEqual(
      labels,
      expectedOrder,
      `primary nav order mismatch in ${expanded ? "expanded" : "collapsed"} state`
    );
  }
});
