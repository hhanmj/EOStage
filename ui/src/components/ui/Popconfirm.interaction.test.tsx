import assert from "node:assert/strict";
import test from "node:test";
import { JSDOM } from "jsdom";
import React, { useRef, useState } from "react";

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

const { cleanup, fireEvent, render } = await import("@testing-library/react");
const { Popconfirm } = await import("./Popconfirm");

test.afterEach(() => {
  cleanup();
});

test("Popconfirm anchors to trigger, cancels on Escape/outside click returning focus, and executes named confirm", () => {
  const events: string[] = [];

  function Harness() {
    const [open, setOpen] = useState(false);
    const btnRef = useRef<HTMLButtonElement | null>(null);
    return (
      <div>
        <button
          ref={btnRef}
          type="button"
          data-testid="trigger-btn"
          onClick={() => setOpen(true)}
        >
          删除项目
        </button>
        <button type="button" data-testid="outside-btn">
          外部按钮
        </button>
        <Popconfirm
          open={open}
          anchorEl={btnRef.current}
          title="删除此项目？"
          description="删除后无法恢复。"
          confirmLabel="确认删除"
          onCancel={() => {
            events.push("cancel");
            setOpen(false);
          }}
          onConfirm={() => {
            events.push("confirm");
            setOpen(false);
          }}
        />
      </div>
    );
  }

  const view = render(<Harness />);
  const trigger = view.getByTestId("trigger-btn");
  trigger.focus();
  fireEvent.click(trigger);

  const bubble = dom.window.document.querySelector('[data-testid="sp-popconfirm"]');
  assert.ok(bubble, "Popconfirm should mount in portal");
  assert.ok(bubble.textContent?.includes("删除此项目？"));
  assert.ok(bubble.textContent?.includes("删除后无法恢复。"));

  // Escape closes and returns focus to trigger
  fireEvent.keyDown(dom.window.document, { key: "Escape" });
  assert.equal(dom.window.document.querySelector('[data-testid="sp-popconfirm"]'), null);
  assert.equal(dom.window.document.activeElement, trigger, "Focus returns to trigger on Escape");
  assert.deepEqual(events, ["cancel"]);

  // Re-open and click outside
  fireEvent.click(trigger);
  assert.ok(dom.window.document.querySelector('[data-testid="sp-popconfirm"]'));
  fireEvent.mouseDown(view.getByTestId("outside-btn"));
  assert.equal(dom.window.document.querySelector('[data-testid="sp-popconfirm"]'), null);
  assert.deepEqual(events, ["cancel", "cancel"]);

  // Re-open and confirm
  fireEvent.click(trigger);
  const confirmBtn = dom.window.document.querySelector('[data-testid="popconfirm-confirm-btn"]') as HTMLElement;
  assert.equal(confirmBtn.textContent, "确认删除");
  fireEvent.click(confirmBtn);
  assert.equal(dom.window.document.querySelector('[data-testid="sp-popconfirm"]'), null);
  assert.deepEqual(events, ["cancel", "cancel", "confirm"]);
});
