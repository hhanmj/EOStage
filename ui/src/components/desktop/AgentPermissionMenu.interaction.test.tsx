import "../../test-utils/setup-dom";
import assert from "node:assert/strict";
import test from "node:test";
import React, { useState } from "react";
import { waitFor, act } from "@testing-library/react";
import { render, cleanup, fireEvent } from "../../test-utils/render-harness";
import { AgentPermissionMenu } from "./AgentPermissionMenu";
import { FullAccessRiskDialog } from "./FullAccessRiskDialog";
import type {
  AgentPermissionMode,
  AgentPermissionModeView,
} from "../../lib/generated/v3-contracts";
import type { SimpleStorage } from "../../shell/api/shellRoutes";
import { FULL_ACCESS_RISK_ACK_KEY } from "../../shell/api/agentPermissionLogic";

test.afterEach(() => {
  cleanup();
});

function createMockStorage(initialData: Record<string, string> = {}): SimpleStorage {
  const store = { ...initialData };
  return {
    getItem: (k: string) => store[k] ?? null,
    setItem: (k: string, v: string) => {
      store[k] = v;
    },
    removeItem: (k: string) => {
      delete store[k];
    },
  };
}

const globalConfirm: AgentPermissionModeView = {
  effective: "confirm_changes",
  session_override: null,
  global_default: "confirm_changes",
};

function ControlledDialog({
  onConfirm,
  onCancel,
  triggerRef,
}: {
  onConfirm: () => void;
  onCancel: () => void;
  triggerRef?: React.RefObject<HTMLButtonElement | null>;
}) {
  const [open, setOpen] = useState(true);
  return (
    <FullAccessRiskDialog
      open={open}
      onConfirm={() => {
        onConfirm();
        setOpen(false);
      }}
      onCancel={() => {
        onCancel();
        setOpen(false);
      }}
      triggerRef={triggerRef}
    />
  );
}

test("FullAccessRiskDialog requires acknowledgment and preserves cancel/focus behavior", async (suite) => {
  async function settleDialog() {
    await act(async () => {
      await new Promise((resolve) => setTimeout(resolve, 30));
    });
  }

  await suite.test("confirm is disabled until checked, then calls onConfirm; initial focus is Cancel", async () => {
    let confirmCalls = 0;
    const view = render(<ControlledDialog onConfirm={() => { confirmCalls += 1; }} onCancel={() => {}} />);
    const confirm = view.getByRole("button", { name: "确认切换" }) as HTMLButtonElement;
    const checkbox = view.getByRole("checkbox", { name: "我已了解风险" }) as HTMLInputElement;
    const cancel = view.getByRole("button", { name: "取消" });

    assert.equal(confirm.disabled, true);
    assert.equal(checkbox.checked, false);
    await settleDialog();
    assert.equal(document.activeElement, cancel);

    fireEvent.click(confirm);
    assert.equal(confirmCalls, 0);
    fireEvent.click(checkbox);
    assert.equal(confirm.disabled, false);
    fireEvent.click(confirm);
    assert.equal(confirmCalls, 1);
    assert.equal(view.queryByRole("dialog"), null);
  });

  await suite.test("Escape cancels and returns focus to the trigger", async () => {
    const triggerRef = React.createRef<HTMLButtonElement>();
    let cancelCalls = 0;
    let confirmCalls = 0;
    const view = render(
      <>
        <button ref={triggerRef} type="button">权限模式</button>
        <ControlledDialog
          triggerRef={triggerRef}
          onConfirm={() => { confirmCalls += 1; }}
          onCancel={() => { cancelCalls += 1; }}
        />
      </>,
    );
    await settleDialog();
    assert.equal(document.activeElement, view.getByRole("button", { name: "取消" }));
    fireEvent.keyDown(window, { key: "Escape" });
    await settleDialog();
    assert.equal(view.queryByRole("dialog"), null);
    assert.equal(cancelCalls, 1);
    assert.equal(confirmCalls, 0);
    assert.ok(document.activeElement === triggerRef.current);
  });

  await suite.test("inner panel does not cancel; backdrop click cancels without confirming", async () => {
    let cancelCalls = 0;
    let confirmCalls = 0;
    const firstView = render(
      <ControlledDialog onConfirm={() => { confirmCalls += 1; }} onCancel={() => { cancelCalls += 1; }} />,
    );
    fireEvent.click(firstView.getByRole("dialog").children[1]);
    assert.equal(cancelCalls, 0, "面板内部点击不得取消");
    cleanup();

    let selected: AgentPermissionMode | null = null;
    const view = render(
      <AgentPermissionMenu
        view={globalConfirm}
        status="ready"
        riskStorage={createMockStorage()}
        onSelect={(mode) => { selected = mode; }}
      />,
    );
    fireEvent.click(view.getByRole("button", { name: "权限模式" }));
    fireEvent.click(view.getByText("完全访问"));
    const dialog = view.getByRole("dialog");
    const backdrop = dialog.querySelector('[aria-hidden="true"]');
    assert.ok(backdrop);
    fireEvent.click(backdrop);
    assert.equal(view.queryByRole("dialog"), null);
    assert.equal(selected, null, "背景取消不得切换权限模式");
    assert.equal(confirmCalls, 0);
  });

  await suite.test("reopening resets the acknowledgment", () => {
    let confirmCalls = 0;
    function ReopenableDialog() {
      const [open, setOpen] = useState(true);
      return (
        <>
          <button type="button" onClick={() => setOpen(true)}>重新打开</button>
          <FullAccessRiskDialog
            open={open}
            onConfirm={() => {
              confirmCalls += 1;
              setOpen(false);
            }}
            onCancel={() => setOpen(false)}
          />
        </>
      );
    }

    const view = render(<ReopenableDialog />);
    fireEvent.click(view.getByRole("checkbox", { name: "我已了解风险" }));
    fireEvent.click(view.getByRole("button", { name: "确认切换" }));
    fireEvent.click(view.getByRole("button", { name: "重新打开" }));
    assert.equal((view.getByRole("button", { name: "确认切换" }) as HTMLButtonElement).disabled, true);
    assert.equal((view.getByRole("checkbox", { name: "我已了解风险" }) as HTMLInputElement).checked, false);
    assert.equal(confirmCalls, 1);
  });
});

test("AgentPermissionMenu 交互测试套件（W5-2g 三档）", async (suite) => {
  await suite.test("1. 读写往返：onSelect 更新为新视图后按钮文字同步更新", async () => {
    function TestWrapper() {
      const [view, setView] = useState<AgentPermissionModeView>(globalConfirm);
      return (
        <AgentPermissionMenu
          view={view}
          status="ready"
          riskStorage={createMockStorage({ [FULL_ACCESS_RISK_ACK_KEY]: "1" })}
          onSelect={(mode) => {
            setView({ effective: mode, session_override: mode, global_default: "confirm_changes" });
          }}
          onSetGlobal={() => {}}
          onClearSession={() => setView(globalConfirm)}
        />
      );
    }

    const { getByRole, findByText } = render(<TestWrapper />);
    const btn = getByRole("button", { name: "权限模式" });
    assert.ok(btn.textContent?.includes("变更前确认"));

    fireEvent.click(btn);
    fireEvent.click(await findByText("完全访问"));
    await waitFor(() => {
      assert.ok(btn.textContent?.includes("完全访问"));
    });

    fireEvent.click(btn);
    fireEvent.click(await findByText("自主审批"));
    await waitFor(() => {
      assert.ok(btn.textContent?.includes("自主审批"));
    });
  });

  // Owner 10-31：不再区分「全局默认 / 会话设置」，按钮只显示模式名；发送时才生效的待定状态仍提示。
  await suite.test("2. 按钮只显示模式名，不显示全局默认或会话设置；待发送时提示", async () => {
    const viewGlobal: AgentPermissionModeView = {
      effective: "auto_approve",
      session_override: null,
      global_default: "auto_approve",
    };
    const { getByRole, rerender } = render(
      <AgentPermissionMenu view={viewGlobal} status="ready" onSelect={() => {}} />
    );
    const btn = getByRole("button", { name: "权限模式" });
    assert.ok(btn.textContent?.includes("自主审批"));
    assert.ok(!btn.textContent?.includes("全局默认"));
    rerender(
      <AgentPermissionMenu
        view={{ ...viewGlobal, session_override: "auto_approve" }}
        status="ready"
        onSelect={() => {}}
      />
    );
    assert.ok(!btn.textContent?.includes("会话设置"));
    rerender(
      <AgentPermissionMenu view={viewGlobal} status="ready" onSelect={() => {}} pendingApply />
    );
    assert.ok(btn.textContent?.includes("将在发送时应用"));
  });

  await suite.test("3. 三项与说明：变更前确认 / 自主审批 / 完全访问各一句说明，没有任何输入框与「额度」字样", async () => {
    const { getByRole, findByRole, getAllByRole, queryAllByRole } = render(
      <AgentPermissionMenu view={globalConfirm} status="ready" onSelect={() => {}} onSetGlobal={() => {}} />
    );
    fireEvent.click(getByRole("button", { name: "权限模式" }));
    const menu = await findByRole("menu", { name: "权限模式" });
    const items = getAllByRole("menuitemradio");
    assert.deepStrictEqual(
      items.map((i) => i.querySelector("[data-mode-label]")?.textContent),
      ["变更前确认", "自主审批", "完全访问"],
    );
    assert.ok(items[0].textContent?.includes("创建或修改任务前需经确认"));
    assert.ok(items[1].textContent?.includes("信息完整时自动执行"));
    assert.ok(items[2].textContent?.includes("全部自动执行"));
    assert.strictEqual(items[0].getAttribute("aria-checked"), "true");
    assert.strictEqual(queryAllByRole("textbox").length, 0, "不再有额度表单");
    assert.ok(!menu.textContent?.includes("额度"));
  });

  await suite.test("4. 选「自主审批」：直接以字符串 auto_approve 调用 onSelect，不弹风险提示", async () => {
    let selected: AgentPermissionMode | null = null;
    const { getByRole, findByText, queryByText } = render(
      <AgentPermissionMenu
        view={globalConfirm}
        status="ready"
        onSelect={(m) => {
          selected = m;
        }}
        onSetGlobal={() => {}}
      />
    );
    fireEvent.click(getByRole("button", { name: "权限模式" }));
    fireEvent.click(await findByText("自主审批"));
    await waitFor(() => assert.strictEqual(selected, "auto_approve"));
    assert.strictEqual(queryByText("要切换到完全访问吗？"), null);
  });

  await suite.test("5. 风险提示：未确认时选「完全访问」弹出风险对话框，取消不保存；确认后写入存储并调用；之后不再弹", async () => {
    let selectedMode: AgentPermissionMode | null = null;
    const storage = createMockStorage();
    const { getByRole, findByText, queryByText, findByRole } = render(
      <AgentPermissionMenu
        view={globalConfirm}
        status="ready"
        riskStorage={storage}
        onSelect={(mode) => {
          selectedMode = mode;
        }}
        onSetGlobal={() => {}}
      />
    );

    const btn = getByRole("button", { name: "权限模式" });
    fireEvent.click(btn);
    fireEvent.click(await findByText("完全访问"));

    const dialog = await findByRole("dialog");
    assert.ok(dialog.textContent?.includes("要切换到完全访问吗？"));
    assert.ok(!dialog.textContent?.includes("额度"), "风险提示不得出现额度");
    assert.ok(!dialog.textContent?.includes("逐项"), "风险提示不得沿用旧模式口径");
    assert.ok(dialog.textContent?.includes("下载目录未确定时仍会询问"));
    assert.strictEqual(selectedMode, null, "弹窗出现前不应调用 onSelect");

    fireEvent.click(await findByText("取消"));
    await waitFor(() => assert.strictEqual(queryByText("要切换到完全访问吗？"), null));
    assert.strictEqual(selectedMode, null, "取消后不应调用 onSelect");
    assert.strictEqual(storage.getItem(FULL_ACCESS_RISK_ACK_KEY), null);

    fireEvent.click(btn);
    fireEvent.click(await findByText("完全访问"));
    fireEvent.click(getByRole("checkbox", { name: "我已了解风险" }));
    fireEvent.click(await findByText("确认切换"));
    await waitFor(() => {
      assert.strictEqual(queryByText("要切换到完全访问吗？"), null);
      assert.strictEqual(selectedMode, "full_access");
      assert.strictEqual(storage.getItem(FULL_ACCESS_RISK_ACK_KEY), "1");
    });

    selectedMode = null;
    fireEvent.click(btn);
    fireEvent.click(await findByText("完全访问"));
    await waitFor(() => {
      assert.strictEqual(queryByText("要切换到完全访问吗？"), null, "已确认风险后不应再次弹窗");
      assert.strictEqual(selectedMode, "full_access");
    });
  });

  await suite.test("6. 选「变更前确认」：直接调用 onSelect('confirm_changes') 且不弹风险提示", async () => {
    let selectedMode: AgentPermissionMode | null = null;
    const { getByRole, findByText, queryByText } = render(
      <AgentPermissionMenu
        view={{ effective: "full_access", session_override: "full_access", global_default: "confirm_changes" }}
        status="ready"
        onSelect={(mode) => {
          selectedMode = mode;
        }}
        onSetGlobal={() => {}}
      />
    );
    fireEvent.click(getByRole("button", { name: "权限模式" }));
    fireEvent.click(await findByText("变更前确认"));
    assert.strictEqual(queryByText("要切换到完全访问吗？"), null);
    await waitFor(() => assert.strictEqual(selectedMode, "confirm_changes"));
  });

  await suite.test("7. 极简模式：菜单内不再渲染「设为全局默认」与「跟随全局默认」按钮，纯粹用于选择模式", async () => {
    const { getByRole, queryByText } = render(
      <AgentPermissionMenu
        view={{ effective: "full_access", session_override: "full_access", global_default: "confirm_changes" }}
        status="ready"
        onSelect={() => {}}
        onSetGlobal={() => {}}
      />
    );
    fireEvent.click(getByRole("button", { name: "权限模式" }));
    assert.strictEqual(queryByText("设为全局默认"), null, "不再渲染设为全局默认按钮");
    assert.strictEqual(queryByText("跟随全局默认"), null, "不再渲染跟随全局默认按钮");
    assert.strictEqual(queryByText("已是全局默认"), null);
  });

  await suite.test("8. 风险确认回调抛错：确认后 onSelect 抛错时重新打开选择器菜单并展示错误文字", async () => {
    const { getByRole, findByText } = render(
      <AgentPermissionMenu
        view={globalConfirm}
        status="ready"
        riskStorage={createMockStorage()}
        onSelect={async () => {
          throw new Error("后端连接失败，权限保存受阻");
        }}
        onSetGlobal={() => {}}
      />
    );
    fireEvent.click(getByRole("button", { name: "权限模式" }));
    fireEvent.click(await findByText("完全访问"));
    fireEvent.click(getByRole("checkbox", { name: "我已了解风险" }));
    fireEvent.click(await findByText("确认切换"));
    await waitFor(async () => {
      assert.ok(await findByText("权限模式未能保存"));
    });
  });

  await suite.test("9. 自适应方向（A1）：触发按钮靠近窗口底部时弹层向上展开", async () => {
    const originalInnerHeight = window.innerHeight;
    Object.defineProperty(window, "innerHeight", { writable: true, configurable: true, value: 600 });
    const { getByRole, findByRole } = render(
      <AgentPermissionMenu view={globalConfirm} status="ready" compact={false} onSelect={() => {}} onSetGlobal={() => {}} />
    );
    const btn = getByRole("button", { name: "权限模式" });
    btn.getBoundingClientRect = () =>
      ({ top: 540, bottom: 572, left: 50, right: 150, width: 100, height: 32, x: 50, y: 540, toJSON: () => {} } as DOMRect);
    fireEvent.click(btn);
    const menu = await findByRole("menu", { name: "权限模式" });
    assert.strictEqual(menu.getAttribute("data-placement"), "up");
    assert.ok(menu.className.includes("bottom-[calc(100%+8px)]"));
    Object.defineProperty(window, "innerHeight", { writable: true, configurable: true, value: originalInnerHeight });
  });

  await suite.test("10. 待应用权限模式徽章（W5-6 R5）：pendingApply 为 true 时按钮文本与 title 含「将在发送时应用」，缺省时不含", () => {
    const pendingView: AgentPermissionModeView = {
      effective: "auto_approve",
      session_override: "auto_approve",
      global_default: "confirm_changes",
    };
    const MenuWithPending = AgentPermissionMenu as React.ComponentType<
      React.ComponentProps<typeof AgentPermissionMenu> & { pendingApply?: boolean }
    >;
    const { getByRole, rerender } = render(
      <MenuWithPending
        view={pendingView}
        status="ready"
        pendingApply={true}
        onSelect={() => {}}
        onSetGlobal={() => {}}
      />
    );
    const btn = getByRole("button", { name: "权限模式" });
    assert.ok(
      btn.textContent?.includes("将在发送时应用"),
      "pendingApply=true 时按钮文本必须包含「将在发送时应用」"
    );
    assert.ok(
      btn.getAttribute("title")?.includes("将在发送时应用"),
      "pendingApply=true 时按钮 title 必须包含「将在发送时应用」"
    );

    rerender(
      <MenuWithPending
        view={pendingView}
        status="ready"
        onSelect={() => {}}
        onSetGlobal={() => {}}
      />
    );
    assert.ok(
      !btn.textContent?.includes("将在发送时应用"),
      "pendingApply 缺省时按钮文本不得包含「将在发送时应用」"
    );
    assert.ok(btn.textContent?.includes("自主审批"));
  });
});
