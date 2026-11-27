import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";

const css = readFileSync(new URL("../index.css", import.meta.url), "utf8");
const workbenchMap = readFileSync(
  new URL("../components/WorkbenchMap.tsx", import.meta.url),
  "utf8",
);
const allowedSceneFocusSelector = ".scene-footprint:focus";

function selectorsWithOutlineNone(source: string): string[] {
  const selectors: string[] = [];
  const rules = /([^{}]+)\{([^{}]*)\}/g;
  for (const match of source.matchAll(rules)) {
    if (!/\boutline\s*:\s*none\b/.test(match[2])) continue;
    selectors.push(...match[1].split(",").map((selector) => selector.trim()).filter(Boolean));
  }
  return selectors;
}

function assertSceneFocusOutlineContract(source: string): void {
  for (const selector of selectorsWithOutlineNone(source)) {
    if (!selector.includes(":focus")) continue;
    assert.equal(selector, allowedSceneFocusSelector);
  }
}

test("selected scene SVG focus removes only its bounding-box outline", () => {
  assert.match(
    css,
    /\.scene-footprint:focus\s*\{[^}]*outline:\s*none/s,
  );
  assert.doesNotMatch(css, /(?:^|\})\s*\*:focus[^\{]*\{[^}]*outline:\s*none/s);
  assert.doesNotMatch(css, /\.leaflet-interactive:focus\s*\{[^}]*outline:\s*none/s);
  assertSceneFocusOutlineContract(css);
});

test("WorkbenchMap emits the preview-selected halo outside Leaflet interactivity", () => {
  assert.match(workbenchMap, /className:\s*"scene-footprint scene-footprint--selected"/);
  assert.match(workbenchMap, /pathOptions=\{haloOptions\}\s+interactive=\{false\}/);
  assert.doesNotMatch(css, /\.scene-footprint(?:\.leaflet-interactive)?:focus-visible/);
});

test("scene focus contract rejects broad focus-outline fixtures", () => {
  const fixtures = [
    "svg path:focus { outline: none; }",
    ".leaflet-container .leaflet-interactive:focus { outline: none; }",
    "*:focus { outline: none; }",
    `${allowedSceneFocusSelector}, svg path:focus { outline: none; }`,
  ];

  for (const fixture of fixtures) {
    assert.throws(() => assertSceneFocusOutlineContract(fixture), assert.AssertionError);
  }
});
