import assert from "node:assert/strict";
import test from "node:test";

import { JSDOM } from "jsdom";
import React from "react";

import { AsfRestrictedAccessDialog } from "./AsfRestrictedAccessDialog";
import { asfRestrictedAccessPresentation } from "../lib/asfRestrictedAccess";

const dom = new JSDOM("<!doctype html><html><body></body></html>", {
  url: "http://127.0.0.1/",
});
Object.assign(globalThis, {
  window: dom.window,
  document: dom.window.document,
  HTMLElement: dom.window.HTMLElement,
  Node: dom.window.Node,
  MutationObserver: dom.window.MutationObserver,
  getComputedStyle: dom.window.getComputedStyle,
  IS_REACT_ACT_ENVIRONMENT: true,
});
Object.defineProperty(globalThis, "navigator", {
  configurable: true,
  value: dom.window.navigator,
});

const { cleanup, render, screen } = await import("@testing-library/react");
const userEvent = (await import("@testing-library/user-event")).default;

test.afterEach(() => cleanup());

test("restricted JERS dialog shows ASF's translated reply and opens its support URL", async () => {
  const opened: string[] = [];
  let closed = 0;
  const presentation = asfRestrictedAccessPresentation(
    "JERS-1",
    "ASF_DATASET_ACCESS_RESTRICTED",
  );
  assert.ok(presentation);
  render(
    <AsfRestrictedAccessDialog
      presentation={presentation}
      onClose={() => { closed += 1; }}
      onOpenSupport={(url) => { opened.push(url); }}
    />,
  );

  assert.ok(screen.getByRole("dialog", { name: "禁止访问" }));
  assert.ok(screen.getByText("JERS-1 是受限数据集。有关如何访问这些数据的信息，请联系 ASF 用户支持。"));
  await userEvent.click(screen.getByRole("button", { name: "ASF 用户支持" }));
  assert.deepEqual(opened, ["https://asf.alaska.edu/asf/contact/"]);
  await userEvent.click(screen.getByRole("button", { name: "关闭" }));
  assert.equal(closed, 1);
});
