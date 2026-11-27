import "../../test-utils/setup-dom";
import assert from "node:assert/strict";
import test from "node:test";
import React from "react";
import { cleanup, fireEvent, render } from "../../test-utils/render-harness";
import { I18nProvider, LOCALE_STORAGE_KEY, useI18n, type Locale } from "../../i18n";
import type { StreamConfirmCardItem } from "../../shell/api/agentStreamLogic";
import { AgentComposer } from "./AgentComposer";
import { AgentConfirmCard } from "./AgentConfirmCard";
import { AgentModelPicker } from "./AgentModelPicker";
import { AgentPermissionMenu } from "./AgentPermissionMenu";
import { AgentContextHud } from "./AgentContextHud";
import { FullAccessRiskDialog } from "./FullAccessRiskDialog";

const permissionView = {
  effective: "confirm_changes",
  session_override: null,
  global_default: "confirm_changes",
} as const;

const confirmItem: StreamConfirmCardItem = {
  kind: "confirm_card",
  seq: 1,
  run_id: "run-locale",
  plan_id: "plan-locale",
  plan_hash: "hash-locale",
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
};

const expectations = {
  en: {
    placeholder: "Describe the area, time, satellite, and purpose",
    permission: "Permission mode",
    modes: ["Confirm changes", "Auto-approve", "Full access"],
    enqueue: "Add to download queue",
    reject: "Reject",
    dialog: "Switch to full access?",
    acknowledge: "I understand the risks",
  },
  ja: {
    placeholder: "範囲、期間、衛星、用途を入力",
    permission: "権限モード",
    modes: ["変更前に確認", "自動承認", "フルアクセス"],
    enqueue: "ダウンロードキューに追加",
    reject: "拒否",
    dialog: "フルアクセスに切り替えますか？",
    acknowledge: "リスクを理解しました",
  },
} as const;

function LanguageControl({ locale }: { locale: Locale }) {
  const { setLocale } = useI18n();
  return <button type="button" onClick={() => setLocale(locale)}>Switch language</button>;
}

function AgentLabels({ locale }: { locale: Locale }) {
  return (
    <I18nProvider>
      <LanguageControl locale={locale} />
      <AgentComposer
        value=""
        onInputChange={() => {}}
        onSubmit={() => {}}
        channelReadiness={{ ready: true, disabled: false, reason_zh: "" }}
        channels={[]}
        sessionId={null}
        permissionView={permissionView}
        permissionStatus="ready"
        onPermissionSelect={() => {}}
        onPermissionSetGlobal={() => {}}
      />
      <AgentPermissionMenu view={permissionView} status="ready" onSelect={() => {}} />
      <AgentConfirmCard item={confirmItem} />
      <FullAccessRiskDialog open onConfirm={() => {}} onCancel={() => {}} />
    </I18nProvider>
  );
}

test.afterEach(() => {
  cleanup();
  window.localStorage.removeItem(LOCALE_STORAGE_KEY);
});

for (const locale of ["en", "ja"] as const) {
  test(`${locale} switches Agent composer, permission, confirmation, and full-access labels`, () => {
    const expected = expectations[locale];
    const view = render(<AgentLabels locale={locale} />);
    assert.equal(view.getByRole("textbox", { name: "科学需求输入框" }).getAttribute("placeholder"), "描述范围、时间、卫星与用途");

    fireEvent.click(view.getByRole("button", { name: "Switch language" }));

    assert.equal(view.getByRole("textbox", { name: locale === "en" ? "Scientific request input" : "科学的な要求の入力欄" }).getAttribute("placeholder"), expected.placeholder);
    const permissionButtons = view.getAllByRole("button", { name: expected.permission });
    fireEvent.click(permissionButtons[0]);
    const menu = view.getAllByRole("menu", { name: expected.permission })[0];
    assert.deepEqual(
      Array.from(menu.querySelectorAll("[data-mode-label]"), (element) => element.textContent),
      expected.modes,
    );
    const autoApproveItem = menu.querySelector("[data-testid='permission-menu-item-auto_approve']")?.textContent || "";
    if (locale === "en") {
      assert.equal(/[\u4e00-\u9fff]/.test(autoApproveItem), false, `English permission auto_approve leaked CJK: ${autoApproveItem}`);
    } else {
      assert.equal(/自动执行检索|已验证目录下载|超大容量|仍需确认/.test(autoApproveItem), false, `Japanese permission auto_approve leaked Simplified Chinese: ${autoApproveItem}`);
    }

    // Open slash Skills & Workflows picker in Composer and confirm every skill renders cleanly in target locale
    const skillBtnName = locale === "en" ? "Select Skill or Workflow" : "スキルとワークフローを選択";
    fireEvent.click(view.getByRole("button", { name: skillBtnName }));
    const skillPicker = view.getByTestId("agent-skill-picker");
    const skillItemsText = skillPicker.textContent || "";
    assert.ok(skillPicker.querySelectorAll("[data-testid^='skill-picker-item-']").length >= 10, "expected builtin and geoscience skills in picker");
    if (locale === "en") {
      assert.equal(/[\u4e00-\u9fff]/.test(skillItemsText), false, `English skill picker leaked CJK: ${skillItemsText}`);
    } else {
      assert.equal(
        /遥感影像分析|遥感变化检测|地形与水文分析|激光点云|数据准备与任务编排核心规程|时空基线配对工作流|高程基准自动化对齐|批量匹配工作流/.test(skillItemsText),
        false,
        `Japanese skill picker leaked Simplified Chinese: ${skillItemsText}`,
      );
    }
    assert.ok(view.getByRole("button", { name: expected.enqueue }));
    assert.ok(view.getByRole("button", { name: expected.reject }));
    assert.ok(view.getByRole("dialog", { name: expected.dialog }));
    const checkbox = view.getByRole("checkbox", { name: expected.acknowledge });
    fireEvent.click(checkbox);
    assert.equal((view.getByRole("button", { name: locale === "en" ? "Confirm switch" : "切り替えを確認" }) as HTMLButtonElement).disabled, false);
    assert.ok(view.getByText(/大同盆地/), "AOI content must retain its original language");
    assert.ok(view.getByText("SLC"), "scientific identifiers must retain their original spelling");
    assert.equal(document.documentElement.lang, locale);
  });
}

test("AgentContextHud and AgentModelPicker render English with 0 CJK and Japanese without Simplified Chinese leaks", () => {
  const sampleChannels = [
    {
      id: "eostage-official",
      label: "EOStage",
      provider: "openai",
      enabled: true,
      key_ready: true,
      model: "glm-5.3-flash",
      models: [
        {
          id: "glm-5.3-flash",
          name: "GLM-5.3 Flash",
          input_types: ["text"],
          reasoning_levels: [
            { level: "off", value: "off" },
            { level: "low", value: "low" },
            { level: "medium", value: "medium" },
            { level: "high", value: "high" },
          ],
        },
      ],
    },
  ] as any;

  for (const locale of ["en", "ja"] as const) {
    window.localStorage.setItem(LOCALE_STORAGE_KEY, locale);
    const view = render(
      <I18nProvider>
        <AgentContextHud runs={[]} events={[]} modelLabel="GLM-5.3 Flash" />
        <AgentModelPicker
          channels={sampleChannels}
          sessionId={null}
          ready={true}
        />
      </I18nProvider>,
    );

    const pickerTrigger = view.getByTestId("agent-model-picker-trigger");
    fireEvent.click(pickerTrigger);
    const pulsePill = view.getByTestId("pulse-pill-context");
    fireEvent.click(pulsePill);
    const bodyText = document.body.textContent || "";
    const allAttrs = Array.from(document.body.querySelectorAll("[title], [aria-label], [placeholder]"))
      .map((el) => `${el.getAttribute("title") || ""} ${el.getAttribute("aria-label") || ""} ${el.getAttribute("placeholder") || ""}`)
      .join("\n");

    if (locale === "en") {
      assert.equal(/[\u4e00-\u9fff]/.test(bodyText), false, `English HUD/ModelPicker leaked CJK: ${bodyText}`);
      assert.equal(/[\u4e00-\u9fff]/.test(allAttrs), false, `English HUD/ModelPicker attrs leaked CJK: ${allAttrs}`);
      assert.ok(bodyText.includes("EOStage Official"));
      assert.ok(bodyText.includes("Reasoning"));
      assert.ok(bodyText.includes("Medium"));
      assert.ok(allAttrs.includes("Cycle reasoning intensity"));
      assert.ok(bodyText.includes("Context usage"));
      assert.ok(bodyText.includes("Minimal"));
    } else {
      assert.ok(bodyText.includes("EOStage 公式"), "Japanese ModelPicker must show EOStage 公式");
      assert.ok(bodyText.includes("思考強度"), "Japanese ReasoningIntensitySlider must show 思考強度");
      assert.ok(allAttrs.includes("思考強度を切替"), "Japanese ReasoningIntensitySlider shortcut title must be localized");
      assert.ok(bodyText.includes("コンテキスト容量"), "Japanese AgentContextHud must show コンテキスト容量");
      assert.ok(bodyText.includes("シンプル"), "Japanese AgentContextHud must show シンプル");
      assert.equal(
        /EOStage 官方|思考强度|切换思考强度|当前模型未配置思考强度档位|默认（未配置档位）|关闭|未配置|渠道列表|同步最新官方模型|搜索模型|管理模型|当前模型|上下文容量/.test(
          `${bodyText}\n${allAttrs}`,
        ),
        false,
        `Japanese HUD/ModelPicker leaked Simplified Chinese:\n${bodyText}\n${allAttrs}`,
      );
    }
    cleanup();
  }
});
