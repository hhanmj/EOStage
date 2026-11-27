import "../test-utils/setup-dom";
import assert from "node:assert/strict";
import test from "node:test";
import React from "react";
import { render, cleanup, fireEvent } from "../test-utils/render-harness";
import { SettingsModal } from "./SettingsModal";
import { ChannelListItem } from "../shell/components/settings/ChannelListItem";
import { ChannelEditorTitle } from "../shell/components/settings/ChannelEditorTitle";

test.afterEach(() => {
  cleanup();
});

function renderModal(open: boolean, onClose: () => void) {
  return render(
    <SettingsModal
      open={open}
      onClose={onClose}
      initialCategory="storage"
      storagePanel={
        <input
          aria-label="嵌套输入"
          onKeyDown={(e) => {
            if (e.key === "Escape" && e.currentTarget.dataset.consume === "1") {
              e.preventDefault();
            }
          }}
        />
      }
    />,
  );
}

test("W5-8 R5 条目 9：设置弹窗 Esc 关闭", async (t) => {
  await t.test("打开时按 Esc → onClose 调用一次", () => {
    let closes = 0;
    const { getByLabelText } = renderModal(true, () => {
      closes += 1;
    });
    fireEvent.keyDown(getByLabelText("嵌套输入"), { key: "Escape" });
    assert.equal(closes, 1, "焦点在弹窗内按 Esc 必须关闭偏好设置");
  });

  await t.test("嵌套控件已 preventDefault → 不关闭", () => {
    let closes = 0;
    const { getByLabelText } = renderModal(true, () => {
      closes += 1;
    });
    const input = getByLabelText("嵌套输入") as HTMLInputElement;
    input.dataset.consume = "1";
    fireEvent.keyDown(input, { key: "Escape" });
    assert.equal(closes, 0, "子对话框已消费的 Esc 不得关闭偏好设置");
  });

  await t.test("输入法组字中 → 不关闭", () => {
    let closes = 0;
    const { getByLabelText } = renderModal(true, () => {
      closes += 1;
    });
    fireEvent.keyDown(getByLabelText("嵌套输入"), { key: "Escape", isComposing: true });
    assert.equal(closes, 0, "isComposing 时按 Esc 不得关闭偏好设置");
  });

  await t.test("其他按键不关闭", () => {
    let closes = 0;
    const { getByLabelText } = renderModal(true, () => {
      closes += 1;
    });
    fireEvent.keyDown(getByLabelText("嵌套输入"), { key: "Enter" });
    assert.equal(closes, 0);
  });

  await t.test("open=false 时按 Esc 不调用", () => {
    let closes = 0;
    renderModal(false, () => {
      closes += 1;
    });
    fireEvent.keyDown(document.body, { key: "Escape" });
    assert.equal(closes, 0);
  });

  await t.test("关闭（open 变 false）后监听移除", () => {
    let closes = 0;
    const onClose = () => {
      closes += 1;
    };
    const view = renderModal(true, onClose);
    view.rerender(
      <SettingsModal open={false} onClose={onClose} initialCategory="storage" />,
    );
    fireEvent.keyDown(document.body, { key: "Escape" });
    assert.equal(closes, 0, "open=false 后不得残留 Esc 监听");
  });
});

test("W5-8 R5 条目 9：渠道 id 用界面字体，编辑标题单行", async (t) => {
  await t.test("ChannelListItem 的 id 元素不含 font-mono", () => {
    const { getByText } = render(
      <ChannelListItem
        channel={{
          id: "l4",
          protocol: "chat_completions",
          base_url: "https://example.invalid/v1",
          model: "m",
          key_ready: true,
          is_default: false,
        }}
        isSelected={false}
        onSelect={() => {}}
      />,
    );
    const idEl = getByText("l4");
    assert.ok(
      !idEl.className.includes("font-mono"),
      `渠道 id 不得用等宽字体（「l4」似「14」），实际 class：${idEl.className}`,
    );
    assert.ok(idEl.className.includes("break-all"), "列表中 id 仍需 break-all 兜底完整显示");
  });

  await t.test("编辑标题 id 元素 truncate 且 title 为完整 id", () => {
    const longId = "channel-with-a-very-long-identifier-l4";
    const { container } = render(
      <ChannelEditorTitle isNewDraft={false} channelId={longId} isDefault={true} />,
    );
    const idEl = container.querySelector(`[title="${longId}"]`);
    assert.ok(idEl, "编辑标题的 id 元素必须带 title=完整 id 以便悬停查看");
    assert.ok(idEl!.className.includes("truncate"), "编辑标题的 id 必须单行截断（truncate）");
    assert.equal(idEl!.textContent, longId);
    assert.ok(!idEl!.className.includes("font-mono"));
    assert.ok(container.textContent?.includes("编辑渠道："));
    assert.ok(container.textContent?.includes("当前默认"));
  });

  await t.test("新建草稿标题", () => {
    const { container } = render(
      <ChannelEditorTitle isNewDraft={true} channelId="draft-1" isDefault={false} />,
    );
    assert.ok(container.textContent?.includes("新建模型渠道"));
    assert.ok(!container.textContent?.includes("draft-1"));
  });
});
