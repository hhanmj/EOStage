import "../../../test-utils/setup-dom";
import assert from "node:assert/strict";
import test from "node:test";
import React from "react";
import { render, cleanup, fireEvent } from "../../../test-utils/render-harness";
import { ChannelDeleteControl } from "./ChannelDeleteControl";

test.afterEach(() => {
  cleanup();
});

test("ChannelDeleteControl 两步就地确认交互测试套件", async (suite) => {
  await suite.test(
    "1. 静止态只有「删除渠道」且未出现确认文案；点击后确认行出现（含「不可恢复」与「确认删除」），未调用 onConfirmDelete",
    () => {
      let confirmCount = 0;
      const { getByRole, getByText, queryByText, queryByRole } = render(
        <ChannelDeleteControl
          onConfirmDelete={() => {
            confirmCount += 1;
          }}
        />,
      );

      const deleteBtn = getByRole("button", { name: "删除渠道" });
      assert.ok(deleteBtn, "静止态应显示「删除渠道」按钮");
      assert.equal(deleteBtn.getAttribute("title"), "删除渠道");
      assert.equal(queryByText(/不可恢复/), null, "静止态不应显示确认提示文案");
      assert.equal(queryByRole("button", { name: "确认删除" }), null);

      fireEvent.click(deleteBtn);

      assert.ok(
        getByText("删除后渠道配置与已存密钥一并移除，不可恢复。"),
        "点击「删除渠道」后应显示不可恢复提示文案",
      );
      assert.ok(getByRole("button", { name: "确认删除" }), "确认态应出现「确认删除」按钮");
      assert.ok(getByRole("button", { name: "取消" }), "确认态应出现「取消」按钮");
      assert.equal(
        queryByRole("button", { name: "删除渠道" }),
        null,
        "确认态期间「删除渠道」按钮不得同时出现",
      );
      assert.equal(confirmCount, 0, "仅进入确认态不应触发 onConfirmDelete");
    },
  );

  await suite.test(
    "2. 点「取消」回到静止态且未调用 onConfirmDelete；再点「删除渠道」可再次进入确认态",
    () => {
      let confirmCount = 0;
      const { getByRole, queryByText } = render(
        <ChannelDeleteControl
          onConfirmDelete={() => {
            confirmCount += 1;
          }}
        />,
      );

      fireEvent.click(getByRole("button", { name: "删除渠道" }));
      const cancelBtn = getByRole("button", { name: "取消" });
      fireEvent.click(cancelBtn);

      assert.equal(confirmCount, 0, "点击取消不应调用 onConfirmDelete");
      assert.equal(queryByText(/不可恢复/), null, "点击取消后应回到静止态");
      const deleteBtnAgain = getByRole("button", { name: "删除渠道" });
      assert.ok(deleteBtnAgain, "回到静止态后应重新显示「删除渠道」按钮");

      fireEvent.click(deleteBtnAgain);
      assert.ok(
        getByRole("button", { name: "确认删除" }),
        "再次点击「删除渠道」应可再次进入确认态",
      );
    },
  );

  await suite.test(
    "3. 确认态点「确认删除」恰调用 onConfirmDelete 一次；deleting=true 时确认按钮 disabled 且重复点击不再调用",
    () => {
      let confirmCount = 0;
      const { getByRole, unmount } = render(
        <ChannelDeleteControl
          onConfirmDelete={() => {
            confirmCount += 1;
          }}
        />,
      );

      fireEvent.click(getByRole("button", { name: "删除渠道" }));
      fireEvent.click(getByRole("button", { name: "确认删除" }));
      assert.equal(confirmCount, 1, "点击「确认删除」应恰调用 onConfirmDelete 1 次");
      assert.ok(
        getByRole("button", { name: "删除渠道" }),
        "确认后应回到静止态",
      );
      unmount();

      let deletingCalls = 0;
      function DeletingHarness() {
        const [deleting, setDeleting] = React.useState(false);
        return (
          <div>
            <button type="button" onClick={() => setDeleting(true)}>
              置为删除中
            </button>
            <ChannelDeleteControl
              deleting={deleting}
              onConfirmDelete={() => {
                deletingCalls += 1;
              }}
            />
          </div>
        );
      }

      const rendered = render(<DeletingHarness />);
      fireEvent.click(rendered.getByRole("button", { name: "删除渠道" }));
      fireEvent.click(rendered.getByRole("button", { name: "置为删除中" }));

      const confirmBtn = rendered.getByRole("button", { name: "确认删除" }) as HTMLButtonElement;
      assert.equal(confirmBtn.disabled, true, "deleting=true 时「确认删除」按钮必须为 disabled");
      fireEvent.click(confirmBtn);
      fireEvent.click(confirmBtn);
      assert.equal(deletingCalls, 0, "deleting=true 时重复点击不得调用 onConfirmDelete");
    },
  );

  // Owner 2026-10-29: 消除独立删除行，删除控件按 DSH 模式移入渠道卡片头部行操作簇
  await suite.test(
    "4. 触发按钮与确认态按钮禁止文字换行（whiteSpace === 'nowrap'），确认态说明文字允许换行（whiteSpace === 'normal'），且设置页卡片头部正确嵌入删除控件",
    async () => {
      const fs = await import("node:fs");
      const path = await import("node:path");
      const { fileURLToPath } = await import("node:url");

      const { getByRole, getByText } = render(
        <ChannelDeleteControl onConfirmDelete={() => {}} />,
      );

      const triggerBtn = getByRole("button", { name: "删除渠道" });
      assert.ok(
        triggerBtn.className.includes("text-sp-danger-text"),
        "「删除渠道」可点击态必须使用清晰的危险色文字 text-sp-danger-text 而非低对比度灰字",
      );
      assert.ok(
        triggerBtn.className.includes("whitespace-nowrap"),
        "触发按钮 className 必须包含 whitespace-nowrap",
      );
      assert.equal(
        getComputedStyle(triggerBtn).whiteSpace,
        "nowrap",
        "触发按钮 getComputedStyle(el).whiteSpace 必须为 nowrap",
      );

      fireEvent.click(triggerBtn);

      const cancelBtn = getByRole("button", { name: "取消" });
      const confirmBtn = getByRole("button", { name: "确认删除" });
      const descSpan = getByText("删除后渠道配置与已存密钥一并移除，不可恢复。");

      assert.ok(
        cancelBtn.className.includes("whitespace-nowrap"),
        "「取消」按钮 className 必须包含 whitespace-nowrap",
      );
      assert.equal(
        getComputedStyle(cancelBtn).whiteSpace,
        "nowrap",
        "「取消」按钮 getComputedStyle(el).whiteSpace 必须为 nowrap",
      );

      assert.ok(
        confirmBtn.className.includes("whitespace-nowrap"),
        "「确认删除」按钮 className 必须包含 whitespace-nowrap",
      );
      assert.equal(
        getComputedStyle(confirmBtn).whiteSpace,
        "nowrap",
        "「确认删除」按钮 getComputedStyle(el).whiteSpace 必须为 nowrap",
      );

      assert.equal(
        descSpan.className.includes("whitespace-nowrap"),
        false,
        "确认态说明 span 不得包含 whitespace-nowrap",
      );
      assert.equal(
        getComputedStyle(descSpan).whiteSpace,
        "normal",
        "确认态说明 span 的 white-space 必须为 normal",
      );

      // Owner 2026-10-29: 消除独立删除行，删除控件按 DSH 模式移入渠道卡片头部行操作簇
      const here = path.dirname(fileURLToPath(import.meta.url));
      const tabSrc = fs.readFileSync(
        path.resolve(here, "./ModelChannelsSettingsTab.tsx"),
        "utf-8",
      );
      assert.ok(
        tabSrc.includes("<ChannelDeleteControl"),
        "ModelChannelsSettingsTab 渠道卡片头部必须渲染 ChannelDeleteControl",
      );
      assert.ok(
        tabSrc.includes('variant="menu-item"'),
        "ModelChannelsSettingsTab 更多操作菜单中的删除渠道应使用 flat menu-item 样式",
      );

      // menu-item variant renders without pill border
      const menuView = render(
        <ChannelDeleteControl variant="menu-item" onConfirmDelete={() => {}} />,
      );
      const menuTrigger = menuView.getByRole("button", { name: "删除渠道" });
      assert.equal(menuTrigger.className.includes("border-sp-danger-text"), false);
      assert.ok(menuTrigger.className.includes("text-sp-danger-text"));
    },
  );
});
