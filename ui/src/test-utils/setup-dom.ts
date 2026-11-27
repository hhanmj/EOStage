import { JSDOM } from "jsdom";

function isDomNodeLike(value: unknown): value is Element & Node {
  return (
    Boolean(value) &&
    typeof value === "object" &&
    typeof (value as Node).nodeType === "number" &&
    typeof (value as Node).nodeName === "string"
  );
}

function summarizeDomNode(value: unknown): unknown {
  if (!isDomNodeLike(value)) {
    return value;
  }
  if (value.nodeType === 1) {
    const tag = (value.tagName || "ELEMENT").toLowerCase();
    const id = value.id ? ` id="${value.id}"` : "";
    const role = value.getAttribute?.("role") ? ` role="${value.getAttribute("role")}"` : "";
    const ariaLabel = value.getAttribute?.("aria-label")
      ? ` aria-label="${value.getAttribute("aria-label")}"`
      : "";
    const testId = value.getAttribute?.("data-testid")
      ? ` data-testid="${value.getAttribute("data-testid")}"`
      : "";
    return `[DOMElement <${tag}${id}${role}${ariaLabel}${testId}>]`;
  }
  return `[DOMNode ${value.nodeName}]`;
}

let assertPatched = false;
function patchNodeAssertForDomNodes() {
  if (assertPatched) return;
  assertPatched = true;
  const proc = (globalThis as { process?: { getBuiltinModule?: (id: string) => unknown } }).process;
  if (!proc || typeof proc.getBuiltinModule !== "function") return;
  const mods = [
    proc.getBuiltinModule("node:assert"),
    proc.getBuiltinModule("node:assert/strict"),
  ];
  for (const mod of mods as Record<string, unknown>[]) {
    if (!mod) continue;
    for (const method of ["equal", "strictEqual"] as const) {
      const orig = mod[method];
      if (typeof orig !== "function") continue;
      mod[method] = function patchedDomAssertEqual(
        this: unknown,
        actual: unknown,
        expected: unknown,
        message?: string | Error
      ) {
        if (actual !== expected && (isDomNodeLike(actual) || isDomNodeLike(expected))) {
          return orig.call(this, summarizeDomNode(actual), summarizeDomNode(expected), message);
        }
        return orig.call(this, actual, expected, message);
      };
    }
  }
}

export interface SetupDomOptions {
  url?: string;
  html?: string;
}

export function setupDomEnvironment(options: SetupDomOptions = {}) {
  patchNodeAssertForDomNodes();
  const dom = new JSDOM(options.html ?? "<!doctype html><html><body></body></html>", {
    url: options.url ?? "http://127.0.0.1/",
  });

  class TestResizeObserver {
    observe() {}
    unobserve() {}
    disconnect() {}
  }

  Object.assign(globalThis, {
    window: dom.window,
    document: dom.window.document,
    HTMLElement: dom.window.HTMLElement,
    HTMLInputElement: dom.window.HTMLInputElement,
    HTMLButtonElement: dom.window.HTMLButtonElement,
    HTMLSelectElement: dom.window.HTMLSelectElement,
    HTMLTextAreaElement: dom.window.HTMLTextAreaElement,
    Node: dom.window.Node,
    Event: dom.window.Event,
    MutationObserver: dom.window.MutationObserver,
    getComputedStyle: dom.window.getComputedStyle.bind(dom.window),
    ResizeObserver: TestResizeObserver,
    IS_REACT_ACT_ENVIRONMENT: true,
  });

  Object.defineProperty(globalThis, "navigator", {
    configurable: true,
    value: dom.window.navigator,
  });

  return {
    dom,
    window: dom.window,
    document: dom.window.document,
  };
}

export const testDom = setupDomEnvironment();
