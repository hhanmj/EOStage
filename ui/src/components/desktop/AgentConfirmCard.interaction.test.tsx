import "../../test-utils/setup-dom";
import assert from "node:assert/strict";
import test from "node:test";
import React from "react";
import { render, cleanup, fireEvent } from "../../test-utils/render-harness";
import { act } from "@testing-library/react";
import { AgentConfirmCard } from "./AgentConfirmCard";
import type {
  StreamConfirmCardItem,
  StreamProgressCardItem,
} from "../../shell/api/agentStreamLogic";
import { findTerminalProgressForPlan } from "../../shell/api/agentConversationLogic";
import type { ApplicationTask } from "../../shell/api/types";
import { getPreciseOrbitPreference } from "../../lib/preciseOrbitPreferences";

test.afterEach(() => {
  cleanup();
});

const baseSnapshot = {
  source: "Sentinel-1 SLC",
  range: {
    aoi_name: "大同盆地",
    area_km2: 120,
    time_range: { start: "2024-04-01", end: "2024-04-30" },
    product: "SLC",
  },
  estimated_bytes: 5000000000,
  output_dir: "D:\\downloads",
};

test("AgentConfirmCard 交互测试套件", async (suite) => {
  // 1. auto 渲染
  await suite.test("1. auto: 渲染两个徽章且无「加入下载队列」「拒绝」按钮、点「取消」调用 onCancelAuto 收到 run_id 与 created 任务 id 数组", async () => {
    let cancelledRunId: string | null = null;
    let cancelledTaskIds: string[] = [];

    const item: StreamConfirmCardItem = {
      kind: "confirm_card",
      seq: 1,
      run_id: "run-auto-1",
      plan_id: "plan-auto-1",
      plan_hash: "hash123",
      snapshot: baseSnapshot,
      approval: {
        decision: "auto_by_mode",
        mode: "full_access",
        card_reasons: [],
      },
      auto_executed: true,
    };

    const { getByText, queryByText } = render(
      <AgentConfirmCard
        item={item}
        createdTaskIds={["task-1", "task-2"]}
        onViewOnMap={() => {}}
        onCancelAuto={(runId, taskIds) => {
          cancelledRunId = runId;
          cancelledTaskIds = taskIds;
        }}
      />
    );

    // 两个徽章
    assert.ok(getByText("已自动执行"));
    assert.ok(getByText("完全访问"));

    // 不包含人工确认按钮与在地图上查看按钮（自动执行后只剩取消）
    assert.strictEqual(queryByText("加入下载队列"), null);
    assert.strictEqual(queryByText("拒绝"), null);
    assert.strictEqual(queryByText("在地图上查看"), null);

    // 底部提示句
    assert.ok(getByText("已按当前权限模式自动加入下载队列，可以取消。"));

    // 点击「取消」按钮
    const cancelBtn = getByText("取消");
    fireEvent.click(cancelBtn);

    assert.strictEqual(cancelledRunId, "run-auto-1");
    assert.deepStrictEqual(cancelledTaskIds, ["task-1", "task-2"]);
  });

  // 2. fallback 渲染
  await suite.test("2. reasons: 显示「需要你确认的原因：」与各原因中文且有「加入下载队列」", async () => {
    let approved = false;

    const item: StreamConfirmCardItem = {
      kind: "confirm_card",
      seq: 2,
      run_id: "run-fallback-1",
      plan_id: "plan-fallback-1",
      plan_hash: "hash456",
      snapshot: baseSnapshot,
      approval: {
        decision: "manual",
        mode: "auto_approve",
        card_reasons: ["size_unknown", "targets_user_task"],
      },
      auto_executed: false,
    };

    const { getByText, queryByText } = render(
      <AgentConfirmCard
        item={item}
        onApprove={() => {
          approved = true;
        }}
      />
    );

    // 提示块
    assert.ok(getByText("需要你确认的原因："));
    assert.ok(getByText("预计大小未知"));
    assert.ok(getByText("涉及你自己建的任务，需要确认"));

    // 包含常规按钮
    const approveBtn = getByText("加入下载队列");
    assert.ok(approveBtn);
    assert.ok(getByText("拒绝"));
    assert.strictEqual(queryByText("已自动执行"), null);

    fireEvent.click(approveBtn);
    assert.strictEqual(approved, true);
  });

  // 3. manual 渲染
  await suite.test("3. manual: 有「加入下载队列」「拒绝」、无原因块", async () => {
    const item: StreamConfirmCardItem = {
      kind: "confirm_card",
      seq: 3,
      run_id: "run-manual-1",
      plan_id: "plan-manual-1",
      plan_hash: "hash789",
      snapshot: baseSnapshot,
      approval: {
        decision: "manual",
        mode: "confirm_changes",
        card_reasons: [],
      },
      auto_executed: false,
    };

    const { getByText, queryByText } = render(<AgentConfirmCard item={item} />);

    assert.ok(getByText("加入下载队列"));
    assert.ok(getByText("拒绝"));
    assert.strictEqual(queryByText("需要你确认的原因："), null);
    assert.strictEqual(queryByText("已自动执行"), null);
    assert.ok(getByText("加入后可在下载中心暂停或取消。"));
  });

  // 4. approval 缺失
  await suite.test("4. approval 缺失时等同 manual", async () => {
    const item: StreamConfirmCardItem = {
      kind: "confirm_card",
      seq: 4,
      run_id: "run-legacy-1",
      plan_id: "plan-legacy-1",
      plan_hash: "hash000",
      snapshot: baseSnapshot,
    };

    const { getByText, queryByText } = render(<AgentConfirmCard item={item} />);

    assert.ok(getByText("加入下载队列"));
    assert.ok(getByText("拒绝"));
    assert.strictEqual(queryByText("需要你确认的原因："), null);
    assert.strictEqual(queryByText("已自动执行"), null);
  });

  // 5. create_task 卡目录为 null
  await suite.test(
    "5. create_task 卡目录为 null：显示「未选择」与「请先选择下载目录」；「加入下载队列」为 disabled；点击不调用 onApprove",
    async () => {
      let approved = false;
      const snapshot = {
        ...baseSnapshot,
        output_dir: null,
        output_dir_source: null,
        already_queued_count: 0,
      };
      const item: StreamConfirmCardItem = {
        kind: "confirm_card",
        seq: 5,
        run_id: "run-5",
        plan_id: "plan-5",
        plan_hash: "hash5",
        snapshot,
        approval: { decision: "manual", mode: "confirm_changes", card_reasons: [] },
        auto_executed: false,
      };

      const { getByText } = render(
        <AgentConfirmCard
          item={item}
          onApprove={() => {
            approved = true;
          }}
        />
      );

      assert.ok(getByText("未选择"));
      assert.ok(getByText("请先选择下载目录"));
      const approveBtn = getByText("加入下载队列").closest("button");
      assert.ok(approveBtn);
      assert.ok(approveBtn.disabled);
      assert.strictEqual(approveBtn.getAttribute("aria-disabled"), "true");
      assert.strictEqual(approveBtn.getAttribute("title"), "请先选择下载目录");

      fireEvent.click(approveBtn);
      assert.strictEqual(approved, false);
    }
  );

  // 6. 无 onPickDirectory 行内编辑
  await suite.test(
    "6. 无 onPickDirectory：点「更改…」出现可访问名「下载目录」的输入框；输入 relative\\dir 点「确定」→ 显示「只接受本机盘符路径…」，onApprove 不被调用；改为 D:\\New 点「确定」→ 行内显示 D:\\New 与「卡上选择」徽章，按钮可用；点「加入下载队列」→ onApprove 收到 (plan_id, 与 item.snapshot 同一引用, \"D:\\\\New\")",
    async () => {
      let approvedPlanId: string | null = null;
      let approvedSnapshot: unknown = null;
      let approvedOutputDir: string | undefined = undefined;

      const snapshot = {
        ...baseSnapshot,
        output_dir: null,
        output_dir_source: null,
        already_queued_count: 0,
      };
      const item: StreamConfirmCardItem = {
        kind: "confirm_card",
        seq: 6,
        run_id: "run-6",
        plan_id: "plan-6",
        plan_hash: "hash6",
        snapshot,
        approval: { decision: "manual", mode: "confirm_changes", card_reasons: [] },
        auto_executed: false,
      };

      const { getByText, getByLabelText } = render(
        <AgentConfirmCard
          item={item}
          onApprove={(planId, snap, outDir) => {
            approvedPlanId = planId;
            approvedSnapshot = snap;
            approvedOutputDir = outDir;
          }}
        />
      );

      const changeBtn = getByText("更改…");
      fireEvent.click(changeBtn);

      const input = getByLabelText("下载目录");
      assert.ok(input);

      // 输入相对路径
      fireEvent.change(input, { target: { value: "relative\\dir" } });
      const confirmBtn = getByText("确定");
      fireEvent.click(confirmBtn);

      assert.ok(getByText("只接受本机盘符路径，例如 D:\\EOStage\\Downloads"));

      const approveBtn = getByText("加入下载队列").closest("button");
      assert.ok(approveBtn);
      assert.ok(approveBtn.disabled);
      fireEvent.click(approveBtn);
      assert.strictEqual(approvedPlanId, null);

      // 改为合法路径 D:\New
      fireEvent.change(input, { target: { value: "D:\\New" } });
      fireEvent.click(confirmBtn);

      assert.ok(getByText("D:\\New"));
      assert.ok(getByText("卡上选择"));
      assert.strictEqual(approveBtn.disabled, false);

      fireEvent.click(approveBtn);
      assert.strictEqual(approvedPlanId, "plan-6");
      assert.strictEqual(approvedSnapshot, snapshot); // 必须是原对象
      assert.strictEqual(approvedOutputDir, "D:\\New");
    }
  );

  // 7. 有 onPickDirectory
  await suite.test(
    "7. 有 onPickDirectory：返回 E:\\Picked → 应用并以第三参数传出；返回 null → 不变；返回 \\\\server\\x → 显示错误、不应用",
    async () => {
      let approvedPlanId: string | null = null;
      let approvedOutputDir: string | undefined = undefined;
      let mockPickResult: string | null = "E:\\Picked";

      const snapshot = {
        ...baseSnapshot,
        output_dir: null,
        output_dir_source: null,
        already_queued_count: 0,
      };
      const item: StreamConfirmCardItem = {
        kind: "confirm_card",
        seq: 7,
        run_id: "run-7",
        plan_id: "plan-7",
        plan_hash: "hash7",
        snapshot,
        approval: { decision: "manual", mode: "confirm_changes", card_reasons: [] },
        auto_executed: false,
      };

      const { getByText } = render(
        <AgentConfirmCard
          item={item}
          onPickDirectory={async () => mockPickResult}
          onApprove={(planId, _, outDir) => {
            approvedPlanId = planId;
            approvedOutputDir = outDir;
          }}
        />
      );

      const changeBtn = getByText("更改…");

      // 1) 返回 E:\Picked
      await act(async () => {
        fireEvent.click(changeBtn);
      });

      assert.ok(getByText("E:\\Picked"));
      assert.ok(getByText("卡上选择"));

      const approveBtn = getByText("加入下载队列").closest("button");
      assert.ok(approveBtn);
      assert.strictEqual(approveBtn.disabled, false);
      fireEvent.click(approveBtn);
      assert.strictEqual(approvedPlanId, "plan-7");
      assert.strictEqual(approvedOutputDir, "E:\\Picked");

      // 2) 返回 null -> 不变
      approvedPlanId = null;
      approvedOutputDir = undefined;
      mockPickResult = null;
      await act(async () => {
        fireEvent.click(changeBtn);
      });
      assert.ok(getByText("E:\\Picked"));

      // 3) 返回 \\server\x -> 显示错误，不应用
      mockPickResult = "\\\\server\\x";
      await act(async () => {
        fireEvent.click(changeBtn);
      });
      assert.ok(getByText("只接受本机盘符路径，例如 D:\\EOStage\\Downloads"));
      // 仍然保持先前的 E:\Picked
      assert.ok(getByText("E:\\Picked"));
    }
  );

  // 8. model_suggested 卡
  await suite.test(
    "8. model_suggested 卡：显示「模型建议的目录」徽章；直接点「加入下载队列」→ 第三参数为 undefined",
    async () => {
      let approvedOutputDir: string | undefined = "initial";

      const snapshot = {
        ...baseSnapshot,
        output_dir: "E:\\SAR\\茂县叠溪",
        output_dir_source: "model_suggested",
        already_queued_count: 0,
      };
      const item: StreamConfirmCardItem = {
        kind: "confirm_card",
        seq: 8,
        run_id: "run-8",
        plan_id: "plan-8",
        plan_hash: "hash8",
        snapshot,
        approval: { decision: "manual", mode: "auto_approve", card_reasons: [] },
        auto_executed: false,
      };

      const { getByText } = render(
        <AgentConfirmCard
          item={item}
          onApprove={(_planId, _snap, outDir) => {
            approvedOutputDir = outDir;
          }}
        />
      );

      assert.ok(getByText("模型建议的目录"));
      const approveBtn = getByText("加入下载队列").closest("button");
      assert.ok(approveBtn);
      assert.strictEqual(approveBtn.disabled, false);

      fireEvent.click(approveBtn);
      assert.strictEqual(approvedOutputDir, undefined);
    }
  );

  // 9. task_action 形快照
  await suite.test(
    "9. task_action 形快照（无 output_dir_source 键、计数 null、有 output_dir）：显示只读「保存位置」，没有「更改…」，按钮可用",
    async () => {
      let approved = false;
      const snapshot = {
        ...baseSnapshot,
        output_dir: "D:\\downloads",
        already_queued_count: null,
      };
      const item: StreamConfirmCardItem = {
        kind: "confirm_card",
        seq: 9,
        run_id: "run-9",
        plan_id: "plan-9",
        plan_hash: "hash9",
        snapshot,
        approval: { decision: "manual", mode: "confirm_changes", card_reasons: [] },
        auto_executed: false,
      };

      const { getByText, queryByText } = render(
        <AgentConfirmCard
          item={item}
          onApprove={() => {
            approved = true;
          }}
        />
      );

      assert.ok(getByText("保存位置"));
      assert.strictEqual(queryByText("更改…"), null);
      assert.strictEqual(queryByText("下载目录"), null);

      const approveBtn = getByText("加入下载队列").closest("button");
      assert.ok(approveBtn);
      assert.strictEqual(approveBtn.disabled, false);

      fireEvent.click(approveBtn);
      assert.strictEqual(approved, true);
    }
  );

  // 10. 有 defaultDownloadDir 且与当前不同
  await suite.test(
    "10. 有 defaultDownloadDir 且与当前不同：出现「使用默认目录」，点后显示该目录与「卡上选择」，批准第三参数为该目录；与当前相同则不出现",
    async () => {
      let approvedOutputDir: string | undefined = undefined;

      const snapshot = {
        ...baseSnapshot,
        output_dir: "E:\\OtherDir",
        output_dir_source: "default",
        already_queued_count: 0,
      };
      const item: StreamConfirmCardItem = {
        kind: "confirm_card",
        seq: 10,
        run_id: "run-10",
        plan_id: "plan-10",
        plan_hash: "hash10",
        snapshot,
        approval: { decision: "manual", mode: "confirm_changes", card_reasons: [] },
        auto_executed: false,
      };

      const { getByText, queryByText } = render(
        <AgentConfirmCard
          item={item}
          defaultDownloadDir={"D:\\ScenePort\\Downloads"}
          onApprove={(_planId, _snap, outDir) => {
            approvedOutputDir = outDir;
          }}
        />
      );

      const defaultBtn = getByText("使用默认目录");
      assert.ok(defaultBtn);

      await act(async () => {
        fireEvent.click(defaultBtn);
      });

      assert.ok(getByText("D:\\ScenePort\\Downloads"));
      assert.ok(getByText("卡上选择"));

      // 点击使用默认目录后，当前已是默认目录，所以「使用默认目录」按钮不再出现
      assert.strictEqual(queryByText("使用默认目录"), null);

      const approveBtn = getByText("加入下载队列").closest("button");
      assert.ok(approveBtn);
      fireEvent.click(approveBtn);

      assert.strictEqual(approvedOutputDir, "D:\\ScenePort\\Downloads");
    }
  );

  // 11. 自动执行卡
  await suite.test(
    "11. 自动执行卡：有目录行与来源徽章，没有「更改…」「使用默认目录」",
    async () => {
      const snapshot = {
        ...baseSnapshot,
        output_dir: "D:\\ScenePort\\Downloads",
        output_dir_source: "default",
        already_queued_count: 0,
      };
      const item: StreamConfirmCardItem = {
        kind: "confirm_card",
        seq: 11,
        run_id: "run-11",
        plan_id: "plan-11",
        plan_hash: "hash11",
        snapshot,
        approval: {
          decision: "auto_by_mode",
          mode: "full_access",
          card_reasons: [],
        },
        auto_executed: true,
      };

      const { getByText, queryByText } = render(
        <AgentConfirmCard
          item={item}
          defaultDownloadDir={"E:\\OtherDefault"}
        />
      );

      assert.ok(getByText("下载目录"));
      assert.ok(getByText("D:\\ScenePort\\Downloads"));
      assert.ok(getByText("默认目录"));

      assert.strictEqual(queryByText("更改…"), null);
      assert.strictEqual(queryByText("使用默认目录"), null);
    }
  );

  // 12. 两个确认卡编辑模式使用不同 id
  await suite.test(
    "12. 两个确认卡同时处于编辑模式：使用 React useId 确保具有不同 id，getAllByLabelText 返回两个不同元素",
    async () => {
      const item1: StreamConfirmCardItem = {
        kind: "confirm_card",
        seq: 12,
        run_id: "run-12-1",
        plan_id: "plan-12-1",
        plan_hash: "hash121",
        snapshot: {
          ...baseSnapshot,
          output_dir: null,
          output_dir_source: null,
          already_queued_count: 0,
        },
        approval: { decision: "manual", mode: "confirm_changes", card_reasons: [] },
        auto_executed: false,
      };

      const item2: StreamConfirmCardItem = {
        kind: "confirm_card",
        seq: 13,
        run_id: "run-12-2",
        plan_id: "plan-12-2",
        plan_hash: "hash122",
        snapshot: {
          ...baseSnapshot,
          output_dir: null,
          output_dir_source: null,
          already_queued_count: 0,
        },
        approval: { decision: "manual", mode: "confirm_changes", card_reasons: [] },
        auto_executed: false,
      };

      const { getAllByText, getAllByLabelText } = render(
        <>
          <AgentConfirmCard item={item1} />
          <AgentConfirmCard item={item2} />
        </>
      );

      const changeButtons = getAllByText("更改…");
      assert.strictEqual(changeButtons.length, 2);

      fireEvent.click(changeButtons[0]!);
      fireEvent.click(changeButtons[1]!);

      const inputs = getAllByLabelText("下载目录");
      assert.strictEqual(inputs.length, 2);
      assert.notStrictEqual(inputs[0], inputs[1]);
      assert.ok(inputs[0]!.id);
      assert.ok(inputs[1]!.id);
      assert.notStrictEqual(inputs[0]!.id, inputs[1]!.id);
    }
  );
  // 13. from_message 卡
  await suite.test(
    "13. from_message 卡：显示「消息中指定的目录」徽章",
    async () => {
      const snapshot = {
        ...baseSnapshot,
        output_dir: "D:\\data\\maoxian",
        output_dir_source: "from_message",
        already_queued_count: 0,
      };
      const item: StreamConfirmCardItem = {
        kind: "confirm_card",
        seq: 14,
        run_id: "run-13",
        plan_id: "plan-13",
        plan_hash: "hash13",
        snapshot,
        approval: { decision: "manual", mode: "auto_approve", card_reasons: [] },
        auto_executed: false,
      };

      const { getByText } = render(<AgentConfirmCard item={item} />);

      assert.ok(getByText("消息中指定的目录"));

    }
  );
  // 14. W5-8 P1-6: 已批准确认卡（L4 D4 历史重开序列）只读且渲染任务卡
  await suite.test(
    "14. 已批准确认卡（L4 D4 序列 running -> done + created_task_ids）：只读无「拒绝」「加入下载队列」按钮，且渲染含任务名、不含任务 ID 的任务卡",
    async () => {
      const planId = "plan-1a17a56f09d-5";
      const taskId = "task-unified-65046dba-c55e-4a01-a49f-728eac13d277";
      const confirmItem: StreamConfirmCardItem = {
        kind: "confirm_card",
        seq: 50,
        run_id: "run-1a17a54dea1-3",
        plan_id: planId,
        plan_hash: "b060b8ab2b17d7fe94bead369c285edfb72a0d4a109bfe60fb6324ce26f69703",
        snapshot: {
          ...baseSnapshot,
          source: "asf.catalog",
          output_dir: "D:\ScenePort\downloads",
          output_dir_source: "default",
          already_queued_count: 0,
        },
        approval: {
          decision: "manual",
          mode: "confirm_changes",
          card_reasons: ["size_unknown"],
        },
        auto_executed: false,
      };

      const progressSequence: StreamProgressCardItem[] = [
        {
          kind: "progress_card",
          seq: 51,
          run_id: "run-1a17a54dea1-3",
          plan_id: planId,
          step_index: 0,
          tool_id: "create_task",
          status: "running",
          code: null,
          created_task_ids: [],
        },
        {
          kind: "progress_card",
          seq: 53,
          run_id: "run-1a17a54dea1-3",
          plan_id: planId,
          step_index: 0,
          tool_id: "create_task",
          status: "done",
          code: null,
          created_task_ids: [taskId],
        },
      ];

      const terminalProgress = findTerminalProgressForPlan(progressSequence, planId);
      assert.ok(terminalProgress);
      assert.strictEqual(terminalProgress.status, "done");

      const mockTasks: ApplicationTask[] = [
        {
          task_id: taskId,
          name: "Sentinel-1D 茂县叠溪 SLC 下载",
          provider: "asf",
          dataset: "s1_slc",
          status: "running",
          items_count: 1,
          total_bytes: 4_700_000_000,
          done_bytes: 210_000_000,
          output_dir: "D:\ScenePort\downloads",
          created_at: "2026-10-26T22:31:00Z",
          updated_at: "2026-10-26T22:31:10Z",
          revision: 2,
        },
      ];

      const { getByText, getByLabelText, getByRole, queryByRole, queryByText, unmount } = render(
        <AgentConfirmCard
          item={confirmItem}
          subsequentProgress={terminalProgress}
          createdTaskIds={[taskId]}
          agentTasks={mockTasks}
          onTaskAction={async () => {
            throw Object.assign(new Error("任务不存在或已结束"), {
              code: "TASK_NOT_FOUND",
              message_zh: "任务不存在或已结束",
            });
          }}
        />
      );

      // 已批准徽章可见
      assert.ok(getByText("已批准"));

      // 只读：无「拒绝」「加入下载队列」「更改…」按钮
      assert.strictEqual(queryByRole("button", { name: "拒绝" }), null);
      assert.strictEqual(queryByRole("button", { name: "加入下载队列" }), null);
      assert.strictEqual(queryByRole("button", { name: "更改…" }), null);

      // 任务卡可见，显示任务名；任务 ID 是内部标识，不出现在界面上
      const taskCard = getByLabelText("任务卡");
      assert.ok(taskCard);
      assert.ok(getByText("Sentinel-1D 茂县叠溪 SLC 下载"));
      // 断言布尔值：直接比较 DOM 节点时 assert 的差异输出会深度展开 jsdom 对象
      assert.strictEqual(
        queryByText(taskId) === null,
        true,
        "已批准确认卡的任务卡不得把任务 ID 当作产品行显示"
      );

      // 点击任务卡动作抛错时，优先展示 err.message_zh（而非退回通用「操作未能完成」）
      const pauseBtn = getByRole("button", { name: "暂停" });
      await act(async () => {
        fireEvent.click(pauseBtn);
      });
      assert.ok(
        getByText("任务不存在或已结束"),
        "AgentTaskCard 动作报错时应优先显示错误对象上的 message_zh"
      );

      unmount();

      // 即使 agentTasks 尚未加载（空列表），仍根据 createdTaskIds 渲染「状态未知」只读任务卡，且 DOM 中不出现任务 ID 原文
      const fallbackRender = render(
        <AgentConfirmCard
          item={confirmItem}
          subsequentProgress={terminalProgress}
          createdTaskIds={[taskId]}
          agentTasks={[]}
        />
      );
      assert.strictEqual(fallbackRender.queryByRole("button", { name: "拒绝" }), null);
      assert.strictEqual(fallbackRender.queryByRole("button", { name: "加入下载队列" }), null);
      assert.ok(fallbackRender.getByLabelText("任务卡"));
      assert.ok(fallbackRender.getByText("下载任务"));
      assert.ok(fallbackRender.getByText("状态未知"));
      assert.strictEqual(fallbackRender.queryByText("排队中"), null);
      assert.strictEqual(
        fallbackRender.queryByText(taskId),
        null,
        "回退任务卡 DOM 中严禁显示任务 ID 文本"
      );
    }
  );

  // 15. precise orbit: Sentinel-1 SLC 确认卡展示精密轨道勾选项与「已默认带上精密轨道」，允许取消勾选并记忆偏好，并在加入下载队列时向 onApprove 传入 includePreciseOrbit
  await suite.test(
    "15. precise orbit: Sentinel-1 SLC 确认卡展示精密轨道勾选项与「已默认带上精密轨道」，允许取消勾选并记忆偏好，并在加入下载队列时向 onApprove 传入 includePreciseOrbit",
    async () => {
      window.localStorage?.clear();
      let approvedPlanId: string | null = null;
      let approvedOutputDir: string | undefined = undefined;
      let approvedIncludePreciseOrbit: boolean | undefined = undefined;
      const orbitChanges: boolean[] = [];

      const item: StreamConfirmCardItem = {
        kind: "confirm_card",
        seq: 16,
        run_id: "run-orbit-1",
        plan_id: "plan-orbit-1",
        plan_hash: "hash-orbit-1",
        snapshot: {
          source: "Sentinel-1 SLC",
          range: {
            aoi_name: "大同盆地",
            area_km2: 120,
            time_range: { start: "2024-04-01", end: "2024-04-30" },
            product: "SLC",
          },
          estimated_bytes: 5000000000,
          output_dir: "D:\\downloads",
        },
        approval: {
          decision: "manual",
          mode: "auto_approve",
          card_reasons: ["size_large"],
        },
        auto_executed: false,
      };

      const { getByRole, getByText, queryByText, unmount } = render(
        <AgentConfirmCard
          item={item}
          onApprove={(planId, _snapshot, outputDir, includePreciseOrbit) => {
            approvedPlanId = planId;
            approvedOutputDir = outputDir;
            approvedIncludePreciseOrbit = includePreciseOrbit;
          }}
          onPreciseOrbitChange={(enabled) => {
            orbitChanges.push(enabled);
          }}
        />
      );

      // 默认勾选并显示提示文案
      const checkbox = getByRole("checkbox", {
        name: "同时下载精密轨道（Orbit / POEORB）",
      }) as HTMLInputElement;
      assert.strictEqual(checkbox.checked, true);
      assert.ok(getByText("已默认附带精密轨道"));

      // 点击取消勾选
      fireEvent.click(checkbox);
      assert.strictEqual(checkbox.checked, false);
      assert.ok(queryByText("已默认附带精密轨道") === null);
      assert.deepStrictEqual(orbitChanges, [false]);

      // 验证本地偏好已持久化为 false
      assert.strictEqual(
        getPreciseOrbitPreference("Sentinel-1 SLC", "SLC"),
        false
      );

      // 点击加入下载队列，onApprove 收到 includePreciseOrbit = false
      const enqueueBtn = getByRole("button", { name: "加入下载队列" });
      fireEvent.click(enqueueBtn);

      assert.strictEqual(approvedPlanId, "plan-orbit-1");
      assert.strictEqual(approvedIncludePreciseOrbit, false);
      assert.strictEqual(approvedOutputDir, undefined);

      unmount();

      // 再次为同一数据源渲染新卡片，应继承用户先前保存的 false 偏好
      const secondItem: StreamConfirmCardItem = {
        ...item,
        plan_id: "plan-orbit-1-recheck",
      };
      const secondRender = render(
        <AgentConfirmCard
          item={secondItem}
          onApprove={(_planId, _snapshot, _outputDir, includePreciseOrbit) => {
            approvedIncludePreciseOrbit = includePreciseOrbit;
          }}
        />
      );

      const secondCheckbox = secondRender.getByRole("checkbox", {
        name: "同时下载精密轨道（Orbit / POEORB）",
      }) as HTMLInputElement;
      assert.strictEqual(secondCheckbox.checked, false);
      assert.ok(secondRender.queryByText("已默认附带精密轨道") === null);

      secondRender.unmount();
      window.localStorage?.clear();
    }
  );

  // 16. precise orbit: Sentinel-1 GRD 确认卡展示精密轨道选项，但默认未勾选且不展示「已默认带上精密轨道」提示，手动勾选后更新偏好并传给 onApprove
  await suite.test(
    "16. precise orbit: Sentinel-1 GRD 确认卡展示精密轨道选项，但默认未勾选且不展示「已默认带上精密轨道」提示，手动勾选后更新偏好并传给 onApprove",
    async () => {
      window.localStorage?.clear();
      let approvedIncludePreciseOrbit: boolean | undefined = undefined;
      const orbitChanges: boolean[] = [];

      const item: StreamConfirmCardItem = {
        kind: "confirm_card",
        seq: 17,
        run_id: "run-orbit-grd",
        plan_id: "plan-orbit-grd",
        plan_hash: "hash-orbit-grd",
        snapshot: {
          source: "Sentinel-1 GRD",
          range: {
            aoi_name: "大同盆地",
            area_km2: 120,
            time_range: { start: "2024-04-01", end: "2024-04-30" },
            product: "GRD",
          },
          estimated_bytes: 1000000000,
          output_dir: "D:\\downloads",
        },
        approval: {
          decision: "manual",
          mode: "auto_approve",
          card_reasons: [],
        },
        auto_executed: false,
      };

      const { getByRole, getByText, queryByText, unmount } = render(
        <AgentConfirmCard
          item={item}
          onApprove={(_planId, _snapshot, _outputDir, includePreciseOrbit) => {
            approvedIncludePreciseOrbit = includePreciseOrbit;
          }}
          onPreciseOrbitChange={(enabled) => {
            orbitChanges.push(enabled);
          }}
        />
      );

      // GRD 默认未勾选，且不显示「已默认带上精密轨道」
      const checkbox = getByRole("checkbox", {
        name: "同时下载精密轨道（Orbit / POEORB）",
      }) as HTMLInputElement;
      assert.strictEqual(checkbox.checked, false);
      assert.ok(queryByText("已默认附带精密轨道") === null);

      // 手动勾选
      fireEvent.click(checkbox);
      assert.strictEqual(checkbox.checked, true);
      assert.ok(getByText("已默认附带精密轨道"));
      assert.deepStrictEqual(orbitChanges, [true]);

      // 验证本地偏好已持久化为 true
      assert.strictEqual(
        getPreciseOrbitPreference("Sentinel-1 GRD", "GRD"),
        true
      );

      // 确认批准
      const enqueueBtn = getByRole("button", { name: "加入下载队列" });
      fireEvent.click(enqueueBtn);
      assert.strictEqual(approvedIncludePreciseOrbit, true);

      unmount();
      window.localStorage?.clear();
    }
  );

  // 17. precise orbit: 非 Sentinel-1 场景（如 Sentinel-2）确认卡不展示精密轨道复选框与提示
  await suite.test(
    "17. precise orbit: 非 Sentinel-1 场景（如 Sentinel-2）确认卡不展示精密轨道复选框与提示",
    async () => {
      let approvedIncludePreciseOrbit: boolean | undefined = undefined;

      const item: StreamConfirmCardItem = {
        kind: "confirm_card",
        seq: 18,
        run_id: "run-s2",
        plan_id: "plan-s2",
        plan_hash: "hash-s2",
        snapshot: {
          source: "Sentinel-2 MSI",
          range: {
            aoi_name: "大同盆地",
            area_km2: 120,
            time_range: { start: "2024-04-01", end: "2024-04-30" },
            product: "L2A",
          },
          estimated_bytes: 200000000,
          output_dir: "D:\\downloads",
        },
        approval: {
          decision: "manual",
          mode: "auto_approve",
          card_reasons: [],
        },
        auto_executed: false,
      };

      const { queryByRole, queryByText, getByRole, unmount } = render(
        <AgentConfirmCard
          item={item}
          onApprove={(_planId, _snapshot, _outputDir, includePreciseOrbit) => {
            approvedIncludePreciseOrbit = includePreciseOrbit;
          }}
        />
      );

      // 复选框和提示文案均不应存在
      assert.strictEqual(
        queryByRole("checkbox", {
          name: "同时下载精密轨道（Orbit / POEORB）",
        }),
        null
      );
      assert.ok(queryByText("已默认附带精密轨道") === null);
      assert.strictEqual(queryByText("精密轨道"), null);

      // 确认批准
      const enqueueBtn = getByRole("button", { name: "加入下载队列" });
      fireEvent.click(enqueueBtn);
      assert.strictEqual(approvedIncludePreciseOrbit, false);

      unmount();
    }
  );

  // 18. superseded confirm card: findTerminalProgressForPlan 识别 plan_superseded_*，展示「已作废（条件已变更）」徽章并禁用审批按钮
  await suite.test(
    "18. superseded confirm card: findTerminalProgressForPlan 识别 plan_superseded_*，展示「已作废（条件已变更）」徽章并禁用审批按钮",
    () => {
      const planId = "plan-sup-1";
      const confirmItem: StreamConfirmCardItem = {
        kind: "confirm_card",
        seq: 19,
        run_id: "run-sup-1",
        plan_id: planId,
        plan_hash: "hash-sup-1",
        snapshot: {
          ...baseSnapshot,
          source_display_name_zh: "Sentinel-1 IW SLC",
        },
        approval: {
          decision: "manual",
          mode: "confirm_changes",
          card_reasons: ["size_unknown"],
        },
        auto_executed: false,
      };

      const progressSequence: StreamProgressCardItem[] = [
        {
          kind: "progress_card",
          seq: 20,
          run_id: "run-sup-1",
          plan_id: planId,
          step_index: null,
          tool_id: null,
          status: "expired",
          code: "plan_superseded_by_new_turn",
          created_task_ids: [],
        },
      ];

      const terminalProgress = findTerminalProgressForPlan(progressSequence, planId);
      assert.ok(terminalProgress);
      assert.strictEqual(terminalProgress.status, "expired");
      assert.strictEqual(terminalProgress.code, "plan_superseded_by_new_turn");

      const { getByTestId, queryByRole, unmount } = render(
        <AgentConfirmCard
          item={confirmItem}
          subsequentProgress={terminalProgress}
        />
      );

      const badge = getByTestId("agent-confirm-superseded-badge");
      assert.strictEqual(badge.textContent?.trim(), "已作废（条件已变更）");
      assert.strictEqual(queryByRole("button", { name: "加入下载队列" }), null);
      assert.strictEqual(queryByRole("button", { name: "拒绝" }), null);

      unmount();
    }
  );
});
