import "../../test-utils/setup-dom";
import assert from "node:assert/strict";
import test from "node:test";
import React from "react";
import { render, cleanup, fireEvent } from "../../test-utils/render-harness";
import { DesktopTitlebar } from "./DesktopTitlebar";

test.afterEach(() => {
  cleanup();
});

test("DesktopTitlebar 新对话按钮交互测试套件 (W5-8 R4b / R4c-2)", async (suite) => {
  await suite.test("1. 传入 newChat={{ enabled: true, onClick }} 时渲染「新对话」按钮且点击触发 onClick", () => {
    let newChatClicked = false;
    const { getByRole } = render(
      <DesktopTitlebar
        newChat={{
          enabled: true,
          onClick: () => {
            newChatClicked = true;
          },
        }}
      />
    );

    const btn = getByRole("button", { name: "新对话" }) as HTMLButtonElement;
    assert.strictEqual(btn.disabled, false);
    assert.strictEqual(btn.title, "新对话");
    fireEvent.click(btn);
    assert.strictEqual(newChatClicked, true);
  });

  await suite.test("2. newChat enabled=false 时按钮禁用；不传 newChat 时不渲染该按钮", () => {
    let newChatClicked = false;
    const { getByRole, unmount } = render(
      <DesktopTitlebar
        newChat={{
          enabled: false,
          onClick: () => {
            newChatClicked = true;
          },
        }}
      />
    );

    const disabledBtn = getByRole("button", { name: "新对话" }) as HTMLButtonElement;
    assert.strictEqual(disabledBtn.disabled, true);
    fireEvent.click(disabledBtn);
    assert.strictEqual(newChatClicked, false);

    unmount();

    const omittedRender = render(<DesktopTitlebar />);
    assert.strictEqual(omittedRender.queryByRole("button", { name: "新对话" }), null);
  });
});
