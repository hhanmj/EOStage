import "../../../test-utils/setup-dom";
import assert from "node:assert/strict";
import test from "node:test";
import React, { act } from "react";
import { render, cleanup, fireEvent } from "../../../test-utils/render-harness";
import {
  AgentMarkdownRenderer,
  splitInlineThink,
  cleanCopyableMarkdown,
  serializeMarkdownTable,
  serializeTableCsv,
} from "./AgentMarkdownRenderer";
import { AgentThinkingCard } from "./AgentThinkingCard";
import { AgentMessageActionBar } from "./AgentMessageActionBar";
import {
  AgentQuestionCard,
  isPassiveDismissQuestionOption,
} from "./AgentQuestionCard";
import { AgentMapSceneDrawer } from "./AgentMapSceneDrawer";
import { AgentConclusionCard } from "../AgentConclusionCard";
import { AgentToolGroupCard } from "../AgentToolGroupCard";
import { AgentErrorCard } from "../AgentErrorCard";
import type {
  StreamConclusionItem,
  StreamErrorItem,
  StreamToolGroupItem,
} from "../../../shell/api/agentStreamLogic";

test.afterEach(() => {
  cleanup();
});

test("Agent 2.0 Slice C 组件与交互测试套件", async (suite) => {
  await suite.test(
    "1. AgentMarkdownRenderer 渲染标题/加粗/斜体/行内代码/列表/引用/GFM 表格/代码复制，且绝不泄露 <think>",
    async () => {
      const md = [
        "<think>内部推理步骤：不应出现在正文 DOM</think>",
        "### 合肥市 Sentinel-1 检索摘要",
        "",
        "已定位 **安徽省合肥市**（*InSAR 干涉像对*），工具 `working_aoi_set` 与景号 `S1A_IW_SLC__1SDV_20251001` 下划线保持原样。",
        "",
        "> 建议使用同轨同极化 SLC 数据做干涉配准。",
        "",
        "- 升轨 Track 142（VV+VH）",
        "- 降轨 Track 47（VV+VH）",
        "",
        "| 景号 | 轨道 | 云量 |",
        "| :--- | :---: | ---: |",
        "| S1A_001 | 升轨 | 0.0% |",
        "| LC09_002 | 降轨 | 4.5% |",
        "",
        "```json",
        '{"adcode": "340100", "city": "合肥市"}',
        "```",
      ].join("\n");

      const view = render(<AgentMarkdownRenderer content={md} />);
      const root = view.getByTestId("agent-markdown-renderer");
      assert.ok(root.classList.contains("select-text"), "正文容器必须允许文本选取");
      assert.ok(
        !(root.textContent || "").includes("内部推理步骤"),
        "<think> 内容绝不能渲染在正文中"
      );
      assert.ok(!(root.textContent || "").includes("<think>"));

      // 标题、加粗、斜体、下划线免疫
      assert.ok(view.getByText("合肥市 Sentinel-1 检索摘要"));
      assert.strictEqual(view.getByText("安徽省合肥市").tagName, "STRONG");
      assert.strictEqual(view.getByText("InSAR 干涉像对").tagName, "EM");
      assert.ok((root.textContent || "").includes("S1A_IW_SLC__1SDV_20251001"));

      // GFM 表格
      const tableWrap = view.getByTestId("agent-markdown-table");
      assert.strictEqual(tableWrap.querySelectorAll("th").length, 3);
      assert.strictEqual(tableWrap.querySelectorAll("tbody tr").length, 2);

      // 围栏代码块与复制反馈
      const codeBlock = view.getByTestId("agent-code-block");
      assert.ok((codeBlock.textContent || "").includes("json"));
      const copyBtn = view.getByTestId("btn-copy-code-block");
      assert.ok((copyBtn.textContent || "").includes("复制代码"));
      await act(async () => {
        fireEvent.click(copyBtn);
      });
      assert.ok((copyBtn.textContent || "").includes("已复制"));

      cleanup();

      // 未闭合 <think> 尾部也不得渲染
      const unclosedView = render(<AgentMarkdownRenderer content="<think>尚未闭合的思考流..." />);
      assert.strictEqual(unclosedView.queryByTestId("agent-markdown-renderer"), null);
      assert.strictEqual(splitInlineThink("<think>流式思考").answerMarkdown, "");
    }
  );

  await suite.test(
    "2. AgentThinkingCard 空思考非思考态严格返回 null；思考中实时读秒，完成后自动折叠并支持点击展开/收起",
    async () => {
      // 2.1 非思考且空内容 -> 严格返回 null（绝不渲染「已思考 0.0 秒」）
      const emptyDone = render(<AgentThinkingCard status="done" thinkingText="" />);
      assert.strictEqual(emptyDone.queryByTestId("agent-thinking-card"), null);
      assert.strictEqual(emptyDone.container.textContent, "");
      cleanup();

      const wsIdle = render(<AgentThinkingCard status="idle" thinkingText="   " />);
      assert.strictEqual(wsIdle.queryByTestId("agent-thinking-card"), null);
      cleanup();

      // 2.2 思考中 -> 显示「思考中 · X.X 秒」，转换为 done 后自动收起为「已思考 X.X 秒」
      const startedAt = Date.now() - 1600;
      const view = render(
        <AgentThinkingCard
          status="thinking"
          thinkingText="第一步：匹配合肥行政区代码 340100"
          startedAtMs={startedAt}
        />
      );

      const toggle = view.getByTestId("agent-thinking-toggle");
      assert.match(toggle.textContent || "", /思考中 · \d+\.\d 秒/);
      const liveBody = view.getByTestId("agent-thinking-body");
      assert.ok(liveBody.classList.contains("select-text"));
      assert.ok((liveBody.textContent || "").includes("340100"));

      // 切换为 done -> 自动收起
      view.rerender(
        <AgentThinkingCard
          status="done"
          thinkingText="第一步：匹配合肥行政区代码 340100"
          startedAtMs={startedAt}
          durationMs={2340}
        />
      );

      const collapsedToggle = view.getByTestId("agent-thinking-toggle");
      assert.ok((collapsedToggle.textContent || "").includes("已思考 2.3 秒"));
      assert.strictEqual(view.queryByTestId("agent-thinking-body"), null);

      // 点击展开 -> 再次显示可复制推理内容
      fireEvent.click(collapsedToggle);
      assert.ok(view.getByTestId("agent-thinking-body"));

      // 再次点击收起
      fireEvent.click(collapsedToggle);
      assert.strictEqual(view.queryByTestId("agent-thinking-body"), null);
    }
  );

  await suite.test(
    "3. AgentMessageActionBar 用户轮（复制/重新编辑并重发/回撤）与助手轮（纯净复制剔除 <think>/重试/停止）",
    async () => {
      const copiedPayloads: string[] = [];
      const editedCalls: Array<{ text: string; turnIndex?: number }> = [];
      const revertedTurns: Array<number | undefined> = [];

      const userView = render(
        <AgentMessageActionBar
          role="user"
          content="查询合肥最近一个月的 Sentinel-1 影像"
          turnIndex={2}
          onCopy={(txt) => copiedPayloads.push(txt)}
          onEditResend={(newText, idx) => {
            editedCalls.push({ text: newText, turnIndex: idx });
          }}
          onRevertTurn={(idx) => {
            revertedTurns.push(idx);
          }}
        />
      );

      assert.ok(userView.getByTestId("agent-user-turn-actions"));
      await act(async () => {
        fireEvent.click(userView.getByTestId("btn-copy-user-msg"));
      });
      assert.deepStrictEqual(copiedPayloads, ["查询合肥最近一个月的 Sentinel-1 影像"]);

      // 打开内联编辑框 -> 修改并确认重发
      fireEvent.click(userView.getByTestId("btn-edit-resend-msg"));
      const textarea = userView.getByTestId("agent-edit-textarea") as HTMLTextAreaElement;
      assert.strictEqual(textarea.value, "查询合肥最近一个月的 Sentinel-1 影像");
      fireEvent.change(textarea, {
        target: { value: "查询武汉最近三个月的 Sentinel-1 升轨影像" },
      });
      await act(async () => {
        fireEvent.click(userView.getByTestId("btn-confirm-edit-resend"));
      });
      assert.deepStrictEqual(editedCalls, [
        { text: "查询武汉最近三个月的 Sentinel-1 升轨影像", turnIndex: 2 },
      ]);

      // 回撤此轮
      fireEvent.click(userView.getByTestId("btn-revert-turn"));
      assert.deepStrictEqual(revertedTurns, [2]);

      cleanup();

      // 助手轮操作栏：复制时自动剔除 <think> 块，支持重试与停止切换
      const assistantCopied: string[] = [];
      const retriedRuns: string[] = [];
      const stoppedRuns: string[] = [];

      const asstView = render(
        <AgentMessageActionBar
          role="assistant"
          content={"<think>这是隐藏思考内容</think>\n\n已为您检索到 **6 景** 升轨数据。"}
          runId="run-88"
          turnIndex={3}
          isRunning={false}
          onCopy={(txt) => assistantCopied.push(txt)}
          onRetryTurn={(id) => {
            if (id) retriedRuns.push(id);
          }}
          onStopRun={(id) => {
            if (id) stoppedRuns.push(id);
          }}
        />
      );

      assert.ok(asstView.getByTestId("agent-assistant-turn-actions"));
      await act(async () => {
        fireEvent.click(asstView.getByTestId("btn-copy-assistant-msg"));
      });
      assert.deepStrictEqual(assistantCopied, ["已为您检索到 **6 景** 升轨数据。"]);
      assert.ok(!cleanCopyableMarkdown("<think>secret</think>公开结果").includes("secret"));

      fireEvent.click(asstView.getByTestId("btn-retry-turn"));
      assert.deepStrictEqual(retriedRuns, ["run-88"]);

      asstView.rerender(
        <AgentMessageActionBar
          role="assistant"
          content="正在生成..."
          runId="run-88"
          isRunning={true}
          onStopRun={(id) => {
            if (id) stoppedRuns.push(id);
          }}
        />
      );
      assert.strictEqual(asstView.queryByTestId("btn-retry-turn"), null);
      fireEvent.click(asstView.getByTestId("btn-stop-run"));
      assert.deepStrictEqual(stoppedRuns, ["run-88"]);
    }
  );

  await suite.test("4. AgentQuestionCard 澄清问题选项卡片点击回传", () => {
    const selected: string[] = [];
    const view = render(
      <AgentQuestionCard
        questionZh="检索到升降轨两套覆盖，请确认优先使用的轨道方向："
        options={[
          { label_zh: "升轨 (Track 142)", value: "ASCENDING", recommended: true },
          { label_zh: "降轨 (Track 47)", value: "DESCENDING" },
        ]}
        onSelectOption={(val) => selected.push(val)}
      />
    );

    assert.ok(view.getByTestId("agent-question-card"));
    assert.ok(view.getByText("推荐"));
    const opt0 = view.getByTestId("agent-question-option-0");
    assert.strictEqual(opt0.getAttribute("data-option-value"), "ASCENDING");
    fireEvent.click(opt0);
    assert.deepStrictEqual(selected, ["ASCENDING"]);
    assert.strictEqual(opt0.getAttribute("aria-pressed"), "true");
    cleanup();

    // Owner 2026-11-21: 选中「暂不下载，先保留检索结果」后，仅高亮真实选中项，不再把第 1 个推荐项标蓝
    assert.equal(isPassiveDismissQuestionOption("keep_only", "暂不下载，先保留检索结果"), true);
    assert.equal(isPassiveDismissQuestionOption("download_all", "下载全部 35 景"), false);
    const settledView = render(
      <AgentQuestionCard
        questionZh="按哪种方式提交局部裁剪任务？"
        selectedValue="keep_only"
        dismissed
        options={[
          { label_zh: "下载全部 35 景", value: "download_all", recommended: true },
          { label_zh: "暂不下载，先保留检索结果", value: "keep_only" },
        ]}
      />
    );
    assert.equal(settledView.getByTestId("agent-question-option-0").getAttribute("aria-pressed"), "false");
    assert.equal(settledView.getByTestId("agent-question-option-1").getAttribute("aria-pressed"), "true");
    assert.equal(
      settledView.getByTestId("agent-question-option-0").className.includes("border-sp-accent"),
      false
    );
    assert.equal(settledView.queryByText("推荐"), null);
    assert.ok(settledView.getByTestId("agent-question-settled-badge"));
  });

  await suite.test("5. AgentMapSceneDrawer 分栏地图底部景详情抽屉交互", () => {
    const empty = render(<AgentMapSceneDrawer scene={null} />);
    assert.strictEqual(empty.queryByTestId("agent-map-scene-drawer"), null);
    cleanup();

    const events: string[] = [];
    const view = render(
      <AgentMapSceneDrawer
        scene={{
          scene_id: "S1A_IW_SLC__1SDV_20261015T101230_056112",
          platform: "Sentinel-1A",
          product_type: "SLC",
          beam_mode: "IW",
          polarization: "VV+VH",
          orbit_direction: "ASCENDING",
          relative_orbit: 142,
          acquisition_datetime: "2026-10-15T10:12:30Z",
          incidence_angle_deg: 38.4,
          file_size_remote: 4_294_967_296,
        }}
        isChecked={false}
        isQuicklookActive={false}
        onToggleQuicklook={(id, next) => events.push(`ql:${id}:${next}`)}
        onToggleCheckScene={(id) => events.push(`chk:${id}`)}
        onDownloadScene={(id) => events.push(`dl:${id}`)}
        onClose={() => events.push("close")}
      />
    );

    const drawer = view.getByTestId("agent-map-scene-drawer");
    assert.ok(drawer);
    assert.strictEqual(
      view.getByTestId("agent-scene-drawer-title").textContent,
      "S1A_IW_SLC__1SDV_20261015T101230_056112"
    );
    assert.strictEqual(view.getByTestId("agent-scene-drawer-orbit").textContent, "升轨");
    assert.strictEqual(view.getByTestId("agent-scene-drawer-polarization").textContent, "VV+VH");
    assert.ok((drawer.textContent || "").includes("入射角 38.4°"));
    assert.ok((drawer.textContent || "").includes("4.0 GB") || (drawer.textContent || "").includes("4 GB"));

    fireEvent.click(view.getByTestId("btn-drawer-toggle-quicklook"));
    fireEvent.click(view.getByTestId("btn-drawer-check-scene"));
    fireEvent.click(view.getByTestId("btn-drawer-download-scene"));
    fireEvent.click(view.getByTestId("btn-drawer-close"));

    assert.deepStrictEqual(events, [
      "ql:S1A_IW_SLC__1SDV_20261015T101230_056112:true",
      "chk:S1A_IW_SLC__1SDV_20261015T101230_056112",
      "dl:S1A_IW_SLC__1SDV_20261015T101230_056112",
      "close",
    ]);
  });

  await suite.test(
    "6. 升级卡片：AgentToolGroupCard（工具执行步骤徽章、折叠态胶囊 [data-tool-id]、定位地图/在地图查看）与 AgentConclusionCard / AgentErrorCard",
    () => {
      const locatedTools: string[] = [];
      const viewedTools: string[] = [];

      const toolGroup: StreamToolGroupItem = {
        kind: "tool_group",
        seq: 1,
        run_id: "run-9",
        status: "done",
        rounds: [
          {
            run_id: "run-9",
            call_id: "c-aoi",
            tool_id: "working_aoi_set",
            status: "done",
            summary_zh: "已绑定安徽省合肥市 (340100)",
          },
          {
            run_id: "run-9",
            call_id: "c-prog",
            tool_id: "search_progress",
            status: "done",
            result_count: 14,
            summary_zh: "Sentinel-1 检索完成",
          },
        ],
      };

      // 6.1 折叠态下，关键空间工具胶囊仍然暴露 [data-tool-id="working_aoi_set"] 与地图跳转按钮
      const collapsedView = render(
        <AgentToolGroupCard
          item={toolGroup}
          isExpanded={false}
          stepDetailMode="detailed"
          onLocateOnMap={(r) => locatedTools.push(r.tool_id)}
          onViewOnMap={(r) => viewedTools.push(r.tool_id)}
        />
      );
      assert.ok(collapsedView.getByText("工具执行步骤"));
      assert.strictEqual(collapsedView.queryByText("详细思考链"), null);
      const aoiPill = collapsedView.container.querySelector('[data-tool-id="working_aoi_set"]');
      assert.ok(aoiPill, "折叠态下必须存在 [data-tool-id='working_aoi_set'] 胶囊");
      assert.strictEqual(aoiPill?.getAttribute("data-tool-status"), "done");

      fireEvent.click(collapsedView.getByTestId("btn-tool-locate-map"));
      fireEvent.click(collapsedView.getByTestId("btn-tool-view-map"));
      assert.deepStrictEqual(locatedTools, ["working_aoi_set"]);
      assert.deepStrictEqual(viewedTools, ["search_progress"]);

      cleanup();

      // 6.2 AgentConclusionCard 整合思考折叠条 + Markdown 表格 + 纯净复制栏
      const concItem: StreamConclusionItem = {
        kind: "conclusion",
        seq: 5,
        run_id: "run-9",
        text: "<think>核验批次 b-1</think>\n### 检索结果\n- **14 景** Sentinel-1 SLC",
        claims: [],
        reasoning_off_refused: false,
        thinking_ms: 1500,
      };
      const concView = render(<AgentConclusionCard item={concItem} showActionBar />);
      assert.ok(concView.getByTestId("agent-thinking-toggle").textContent?.includes("已思考 1.5 秒"));
      assert.ok(concView.getByText("检索结果"));
      assert.strictEqual(concView.getByText("14 景").tagName, "STRONG");
      assert.ok(concView.getByTestId("agent-assistant-turn-actions"));

      cleanup();

      // 6.3 AgentErrorCard 支持一键重试且收起时不泄露 snake_case
      const retriedErrRuns: string[] = [];
      const errItem: StreamErrorItem = {
        kind: "error",
        seq: 6,
        run_id: "run-err-1",
        code: "model_timeout_error",
        status: "failed",
        plan_id: null,
      };
      const errView = render(
        <AgentErrorCard item={errItem} onRetry={(id) => retriedErrRuns.push(id)} />
      );
      assert.ok(!/[a-z]+_[a-z_]+/.test(errView.container.textContent || ""));
      fireEvent.click(errView.getByTestId("btn-error-retry"));
      assert.deepStrictEqual(retriedErrRuns, ["run-err-1"]);
    }
  );

  await suite.test(
    "7. AgentMarkdownRenderer 零依赖科学公式渲染：支持块级 \\[...\\]/$$...$$ 矩阵、行内 \\(...\\)/$...$ 分式根号希腊字母，且不误伤 $100 金额与景号下划线",
    () => {
      const mathMd = [
        "入射角 \\(\\theta_i\\) 与波长 $\\lambda = 5.6\\text{ cm}$ 满足相位关系：预算 $100 至 $200 不受影响。",
        "",
        "\\[\\Delta \\phi \\approx \\frac{4\\pi}{\\lambda} \\cdot \\sqrt{x^2 + y^2} \\times \\cos(\\theta_i)\\]",
        "",
        "协方差矩阵如下：",
        "\\[",
        "\\begin{bmatrix}",
        "\\sigma_{1}^2 & \\rho \\\\",
        "\\rho & \\sigma_{2}^2",
        "\\end{bmatrix}",
        "\\]",
      ].join("\n");

      const view = render(<AgentMarkdownRenderer content={mathMd} />);
      const root = view.getByTestId("agent-markdown-renderer");
      const text = root.textContent || "";

      // 不得裸露原始 LaTeX 反斜杠控制词
      assert.ok(!text.includes("\\theta"), "不应裸露 \\theta 源码");
      assert.ok(!text.includes("\\frac"), "不应裸露 \\frac 源码");
      assert.ok(!text.includes("\\begin{bmatrix}"), "不应裸露 \\begin{bmatrix} 源码");
      assert.ok(!text.includes("\\["), "不应裸露 \\[ 定界符");
      assert.ok(!text.includes("\\("), "不应裸露 \\( 定界符");

      // 渲染出对应希腊字母、运算符与结构组件
      assert.ok(text.includes("θ"), "应渲染希腊字母 θ");
      assert.ok(text.includes("λ"), "应渲染希腊字母 λ");
      assert.ok(text.includes("Δ"), "应渲染希腊字母 Δ");
      assert.ok(text.includes("≈"), "应渲染约等于号 ≈");
      assert.ok(text.includes("×"), "应渲染乘号 ×");
      assert.ok(text.includes("√"), "应渲染根号 √");
      assert.ok(text.includes("$100 至 $200"), "金额 $100 至 $200 必须保持原样不被误判为公式");

      assert.ok(view.getByTestId("agent-math-frac"), "应渲染分式结构组件");
      assert.ok(view.getByTestId("agent-math-sqrt"), "应渲染根号结构组件");
      assert.ok(view.getByTestId("agent-math-matrix"), "应渲染矩阵结构组件");
      assert.strictEqual(view.getAllByTestId("agent-math-block").length, 2);
    }
  );

  await suite.test(
    "8. AgentMarkdownRenderer rich harness：表格复制 Markdown / 下载 CSV / 放大预览弹窗、代码块换行切换、公式复制 LaTeX",
    async () => {
      let copiedText = "";
      Object.defineProperty(navigator, "clipboard", {
        configurable: true,
        value: {
          writeText: async (text: string) => {
            copiedText = text;
          },
        },
      });

      const richMd = [
        "| 参数 | 数值 |",
        "| :--- | ---: |",
        "| 波长, 波段 | 5.6 cm |",
        "| 阈值 | 250 |",
        "",
        "```python",
        "very_long_line_of_python_code = 'check_line_wrap_toggle_state'",
        "```",
        "",
        "$$\\Delta \\phi = \\frac{4\\pi}{\\lambda} d$$",
      ].join("\n");

      const view = render(<AgentMarkdownRenderer content={richMd} />);

      // 8.1 复制表格 Markdown
      await act(async () => {
        fireEvent.click(view.getByTestId("btn-copy-table-markdown"));
      });
      assert.ok(copiedText.includes("| 参数 | 数值 |"));
      assert.ok(copiedText.includes("| 波长, 波段 | 5.6 cm |"));

      // 8.2 CSV 序列化含 BOM 与引号转义
      const csv = serializeTableCsv(["参数", "数值"], [["波长, 波段", "5.6 cm"]]);
      assert.ok(csv.startsWith("\uFEFF"));
      assert.ok(csv.includes('"波长, 波段",5.6 cm'));
      assert.ok(serializeMarkdownTable(["A"], ["left"], [["1"]]).includes("| A |"));
      fireEvent.click(view.getByTestId("btn-download-table-csv"));

      // 8.3 放大预览表格弹窗（点击遮罩不可关闭，关闭按钮可关闭）
      fireEvent.click(view.getByTestId("btn-expand-table-preview"));
      const modalBackdrop = view.getByTestId("table-preview-modal");
      assert.ok(modalBackdrop);
      fireEvent.click(modalBackdrop);
      assert.ok(view.getByTestId("table-preview-modal"), "点击遮罩应阻止误关闭预览窗");
      fireEvent.click(view.getByTestId("btn-close-table-preview"));
      assert.strictEqual(view.queryByTestId("table-preview-modal"), null);

      // 8.4 代码块自动换行开关
      const codeBlock = view.getByTestId("agent-code-block");
      const pre = codeBlock.querySelector("pre")!;
      assert.ok(pre.className.includes("whitespace-pre"));
      assert.ok(!pre.className.includes("whitespace-pre-wrap"));
      fireEvent.click(view.getByTestId("btn-toggle-code-wrap"));
      assert.ok(pre.className.includes("whitespace-pre-wrap"));

      // 8.5 块级公式复制 LaTeX
      await act(async () => {
        fireEvent.click(view.getByTestId("btn-copy-math-latex"));
      });
      assert.strictEqual(copiedText, "\\Delta \\phi = \\frac{4\\pi}{\\lambda} d");
    }
  );
});
