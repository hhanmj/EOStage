/**
 * PluginsAndSkillsPage.interaction.test.tsx
 * 「插件与技能」管理中心一级页面与 AgentComposer 技能注入交互测试套件
 */

import "../../test-utils/setup-dom";
import assert from "node:assert/strict";
import test from "node:test";
import React, { useState } from "react";
import { render, cleanup, fireEvent } from "../../test-utils/render-harness";
import { act } from "@testing-library/react";
import {
  PluginsAndSkillsPage,
  RuntimeToolsSettingsSection,
} from "./PluginsAndSkillsPage";
import { AgentComposer } from "./AgentComposer";
import {
  saveCustomSkills,
  loadCustomSkills,
  BUILTIN_SKILLS,
  SKILLS_STORAGE_KEY,
} from "@/lib/agentSkills";
import { GEOSCIENCE_SKILL_PACKAGES } from "@/lib/geoscienceSkillsCatalog";

if (typeof globalThis.localStorage === "undefined" && typeof window !== "undefined") {
  (globalThis as any).localStorage = window.localStorage;
}

test.afterEach(() => {
  cleanup();
  if (typeof localStorage !== "undefined") {
    localStorage.clear();
  }
});

test("Plugins & Skills Page - 完整交互与真实性测试套件", async (t) => {
  await t.test("1. 页面渲染与自解释能力页签（分析技能、扩展与工具连接）切换与运行组件剥离", async () => {
    const { getByTestId, queryByTestId } = render(<PluginsAndSkillsPage />);

    // 默认在「分析技能」页签；运行组件与旧系统设置按钮已从该页面移除
    assert.ok(getByTestId("skills-panel"));
    assert.equal(queryByTestId("tools-panel"), null);
    assert.equal(queryByTestId("plugins-panel"), null);
    assert.equal(queryByTestId("runtime-panel"), null);
    assert.equal(queryByTestId("tab-runtime"), null);
    assert.equal(queryByTestId("jump-to-settings-btn"), null);

    // 切换到「扩展与工具连接」页签 -> 默认进入插件包（目录 / .zip），并可切至 MCP 与外部工具
    fireEvent.click(getByTestId("tab-extensions"));
    assert.ok(getByTestId("plugins-panel"));
    assert.ok(getByTestId("tab-plugins"));
    assert.ok(getByTestId("tab-tools"));
    assert.equal(queryByTestId("skills-panel"), null);

    // 切换到「MCP 与外部工具」子页签：聚焦连接器管理，不再暴露内部测试算子台与工具契约清单一览
    fireEvent.click(getByTestId("tab-tools"));
    assert.ok(getByTestId("tools-panel"));
    assert.equal(queryByTestId("tools-truth-banner"), null);
    assert.equal(queryByTestId("geodata-native-workbench"), null);

    // 切换回「分析技能」页签
    fireEvent.click(getByTestId("tab-skills"));
    assert.ok(getByTestId("skills-panel"));
  });

  await t.test("2. 技能管理：内置模板归属规范、搜索、查看提示词展开、新建/编辑/删除自定义技能 (作者默认为 User)", async () => {
    localStorage.removeItem(SKILLS_STORAGE_KEY);

    const { getByTestId, queryByTestId } = render(<PluginsAndSkillsPage />);

    // 1) 验证内置全部 29 个权威地学技能展示，且已下线的薄工作流/核心规则不再作为用户技能暴露
    assert.equal(BUILTIN_SKILLS.length, 29);
    assert.ok(queryByTestId("skill-card-builtin-insar-copilot-core") === null);
    assert.ok(queryByTestId("skill-card-builtin-s1-insar-pair-workflow") === null);
    for (const builtin of BUILTIN_SKILLS) {
      assert.ok(
        getByTestId(`skill-card-${builtin.id}`),
        `内置技能 ${builtin.id} 应当存在`
      );
      assert.notEqual(builtin.author, "ScenePort", `${builtin.id} 不应再署名 ScenePort`);
    }

    // 2) 搜索过滤
    const searchInput = getByTestId("skills-search-input") as HTMLInputElement;
    fireEvent.change(searchInput, { target: { value: "ALOS-2" } });
    assert.ok(getByTestId("skill-card-radar-satellite-analysis"));
    assert.ok(queryByTestId("skill-card-dem-datum-conversion") === null);

    // 清空搜索
    fireEvent.change(searchInput, { target: { value: "" } });

    // 3) Owner 要求：将卡片「查看技能包 + 查看提示词」整合为「查看 + 复制」，在查看器内展示提示词与包目录以释放 3 列空间，且技能启停采用开关按钮 (role="switch")
    const skillSwitch = getByTestId("toggle-enable-skill-btn-radar-satellite-analysis");
    assert.equal(skillSwitch.getAttribute("role"), "switch");
    assert.equal(skillSwitch.getAttribute("aria-checked"), "true");
    fireEvent.click(skillSwitch);
    assert.equal(skillSwitch.getAttribute("aria-checked"), "false");

    const inspectBtn = getByTestId(
      "inspect-skill-btn-radar-satellite-analysis"
    );
    fireEvent.click(inspectBtn);
    assert.ok(getByTestId("skill-package-inspector"));
    assert.ok(getByTestId("prompt-content-radar-satellite-analysis"));
    assert.ok(getByTestId("inspector-copy-skill-btn"));
    const inspectorSwitch = getByTestId("inspector-toggle-enable-btn");
    assert.equal(inspectorSwitch.getAttribute("role"), "switch");
    assert.equal(inspectorSwitch.getAttribute("aria-checked"), "false");
    fireEvent.click(inspectorSwitch);
    assert.equal(inspectorSwitch.getAttribute("aria-checked"), "true");
    fireEvent.click(getByTestId("close-skill-inspector-btn"));
    assert.equal(
      getByTestId("toggle-enable-skill-btn-radar-satellite-analysis").getAttribute("aria-checked"),
      "true",
    );

    // 4) 新建自定义技能
    const createBtn = getByTestId("create-skill-btn");
    fireEvent.click(createBtn);

    const nameInput = getByTestId("skill-name-input") as HTMLInputElement;
    const descInput = getByTestId("skill-desc-input") as HTMLInputElement;
    const promptInput = getByTestId("skill-prompt-textarea") as HTMLTextAreaElement;
    const submitBtn = getByTestId("save-skill-submit-btn");

    fireEvent.change(nameInput, { target: { value: "滑坡形变监测定制技能" } });
    fireEvent.change(descInput, { target: { value: "针对滑坡灾害的自定义干涉处理指引" } });
    fireEvent.change(promptInput, {
      target: { value: "请重点识别时序形变梯度突变点并输出灾害风险等级。" },
    });
    fireEvent.click(submitBtn);

    // 验证自定义技能已保存到 localStorage，且默认作者署名为 User（非 EOStage）
    const savedCustom = loadCustomSkills();
    assert.equal(savedCustom.length, 1);
    assert.equal(savedCustom[0].name, "滑坡形变监测定制技能");
    assert.equal(savedCustom[0].author, "User");
    const createdId = savedCustom[0].id;

    // 验证页面上渲染了该自定义技能卡片并显示作者 User
    const createdCard = getByTestId(`skill-card-${createdId}`);
    assert.ok(createdCard);
    assert.ok(createdCard.textContent?.includes("User"));
    assert.ok(!createdCard.textContent?.includes("EOStage"));

    // 5) 编辑自定义技能
    const editBtn = getByTestId(`edit-skill-btn-${createdId}`);
    fireEvent.click(editBtn);

    const editDescInput = getByTestId("skill-desc-input") as HTMLInputElement;
    fireEvent.change(editDescInput, {
      target: { value: "更新后的滑坡灾害处理指引说明" },
    });
    fireEvent.click(getByTestId("save-skill-submit-btn"));

    const updatedCustom = loadCustomSkills();
    assert.equal(updatedCustom[0].description, "更新后的滑坡灾害处理指引说明");

    // 6) 删除自定义技能：点击删除弹出就地小气泡确认 (Popconfirm)，确认后删除
    const deleteBtn = getByTestId(`delete-skill-btn-${createdId}`);
    fireEvent.click(deleteBtn);
    assert.ok(getByTestId("sp-popconfirm"), "点击删除自定义技能应弹出就地确认气泡");
    assert.equal(loadCustomSkills().length, 1, "未点击确认前不应删除");
    fireEvent.click(getByTestId("popconfirm-confirm-btn"));

    const afterDelete = loadCustomSkills();
    assert.equal(afterDelete.length, 0);
    assert.equal(queryByTestId(`skill-card-${createdId}`), null);
  });

  await t.test("3. 连接器管理：自定义 MCP 连接器增删、启停 switch、用法弹窗闭环（无假内置连接器与开发调试算子台）", async () => {
    const { getByTestId, queryByTestId, getByText, getByRole } = render(
      <PluginsAndSkillsPage initialTab="tools" />
    );

    assert.ok(getByTestId("tools-panel"));
    // 移除了虚假内置连接器、内部算子执行台与注册工具契约开发清单
    assert.equal(getByTestId("connectors-total-count").textContent, "0");
    assert.ok(getByTestId("connectors-empty-state"));
    assert.equal(queryByTestId("tools-context-broker-card"), null);
    assert.equal(queryByTestId("geodata-native-workbench"), null);
    assert.equal(queryByTestId("tools-truth-banner"), null);

    // 添加自定义 MCP 连接器 (PostGIS Spatial MCP)
    fireEvent.click(getByTestId("add-custom-connector-btn"));
    assert.ok(getByTestId("custom-connector-form"));
    fireEvent.change(getByTestId("custom-connector-name-input"), {
      target: { value: "PostGIS Spatial MCP" },
    });
    fireEvent.click(getByTestId("custom-connector-transport-select"));
    fireEvent.click(getByRole("option", { name: /Stdio/ }));
    fireEvent.change(getByTestId("custom-connector-endpoint-input"), {
      target: { value: "uvx mcp-server-postgis --dsn postgresql://localhost/gis" },
    });
    fireEvent.change(getByTestId("custom-connector-desc-input"), {
      target: { value: "本地空间数据库矢量查询连接器" },
    });
    fireEvent.click(getByTestId("save-custom-connector-btn"));

    // 验证连接器计数变为 1，卡片出现在 connectors-grid 且使用统一 switch 开关与二级用法弹窗
    assert.equal(getByTestId("connectors-total-count").textContent, "1");
    assert.equal(queryByTestId("connectors-empty-state"), null);
    assert.ok(getByText("PostGIS Spatial MCP"));
    assert.ok(getByText("自定义 (STDIO)"));

    const customConnCard = document.querySelector("[data-testid^='custom-connector-card-']") as HTMLElement;
    const customConnId = customConnCard.getAttribute("data-testid")!.replace("custom-connector-card-", "");
    const connSwitch = getByTestId(`toggle-custom-connector-btn-${customConnId}`);
    assert.equal(connSwitch.getAttribute("role"), "switch");
    assert.equal(connSwitch.getAttribute("aria-checked"), "true");
    fireEvent.click(connSwitch);
    assert.equal(connSwitch.getAttribute("aria-checked"), "false");
    fireEvent.click(connSwitch);
    assert.equal(connSwitch.getAttribute("aria-checked"), "true");

    fireEvent.click(getByTestId(`view-connector-usage-btn-${customConnId}`));
    const connUsageModal = getByTestId("connector-usage-inspector");
    assert.ok(connUsageModal.textContent?.includes("PostGIS Spatial MCP"));
    assert.ok(connUsageModal.textContent?.includes("uvx mcp-server-postgis"));
    fireEvent.click(getByTestId("close-connector-usage-btn"));
    assert.equal(queryByTestId("connector-usage-inspector"), null);

    // 移除自定义连接器：点击移除弹出就地小气泡确认 (Popconfirm)，确认后恢复为 0
    const removeBtn = getByText("移除");
    fireEvent.click(removeBtn);
    assert.ok(getByTestId("sp-popconfirm"), "点击移除自定义连接器应弹出就地确认气泡");
    assert.equal(getByTestId("connectors-total-count").textContent, "1");
    fireEvent.click(getByTestId("popconfirm-confirm-btn"));
    assert.equal(getByTestId("connectors-total-count").textContent, "0");
    assert.ok(getByTestId("connectors-empty-state"));
  });

  await t.test("4. 设置 → 运行环境与工具：去除暴露镜像切换条、明确标注开发中组件、展示安装后可用功能及平台/安装闭环", async () => {
    const rendered = render(<RuntimeToolsSettingsSection />);

    await act(async () => {
      await new Promise((r) => setTimeout(r, 15));
    });

    assert.ok(rendered.getByTestId("runtime-panel"));
    assert.equal(
      rendered.queryByTestId("runtime-mirror-bar"),
      null,
      "扩展仓库镜像源切换条不得向普通用户直接暴露",
    );
    const recheckBtn = rendered.getByTestId("recheck-runtime-btn");
    assert.ok(recheckBtn);
    fireEvent.click(recheckBtn);

    // 未上架验证的组件必须诚实标注为「开发中」，不得展示吓人的缺失红叉与损坏安装按钮
    const devReportView = render(
      <RuntimeToolsSettingsSection
        getRuntimeComponentsOverride={async () => ({
          ok: true,
          install_root: "E:\\EOStage",
          manifest_found: true,
          app_version: "2.2.0",
          components: [
            {
              id: "whitebox-cli",
              name: "WhiteboxTools 地形水文",
              kind: "runtime",
              version: "2.4.0",
              install_root: "runtime/whitebox",
              status: "missing",
              file_count: 0,
              total_bytes: 0,
              verified: false,
              detail: "地形填洼与河网水文工具",
              capabilities: ["whitebox_run"],
            },
            {
              id: "duckdb-cli",
              name: "DuckDB Spatial 引擎",
              kind: "runtime",
              version: "1.5.6",
              install_root: "runtime/duckdb",
              status: "missing",
              file_count: 0,
              total_bytes: 0,
              verified: false,
              detail: "矢量 SQL 引擎",
              capabilities: ["duckdb_sql"],
            },
            {
              id: "rustgis-cli",
              name: "RustGIS 轻量核心",
              kind: "runtime",
              version: "0.1.0",
              install_root: "runtime/rustgis",
              status: "missing",
              file_count: 0,
              total_bytes: 0,
              verified: false,
              detail: "轻量 GIS 工具",
              capabilities: ["rustgis"],
            },
            {
              id: "eostage-core",
              name: "EOStage 主程序与核心服务",
              kind: "application",
              version: "2.2.0",
              install_root: ".",
              status: "ready",
              file_count: 1,
              total_bytes: 31457280,
              verified: true,
              detail: "核心服务",
              capabilities: ["desktop-ui"],
            },
          ],
        })}
      />
    );
    await act(async () => {
      await new Promise((r) => setTimeout(r, 15));
    });
    const devCards = Array.from(devReportView.container.querySelectorAll('[data-testid^="runtime-card-"]'));
    assert.equal(devCards[0].getAttribute("data-testid"), "runtime-card-eostage-core", "核心可用组件应排在开发中组件之前");
    for (const devId of ["whitebox-cli", "duckdb-cli", "rustgis-cli"]) {
      const devBadge = devReportView.getByTestId(`runtime-status-badge-${devId}`);
      assert.ok(devBadge.textContent?.includes("开发中"), `${devId} 应标注为开发中`);
      assert.equal(devReportView.queryByTestId(`runtime-dev-pill-${devId}`), null);
      assert.equal(devReportView.queryByTestId(`open-local-install-btn-${devId}`), null);
      assert.ok(
        devReportView.getByTestId(`runtime-capability-${devId}`).textContent?.includes("规划功能："),
      );
    }
    devReportView.unmount();

    // 真实组件应明确告知「安装后可用功能：」
    assert.ok(
      rendered.getByTestId("runtime-capability-vertical-grid-egm2008").textContent?.includes("安装后可用功能："),
    );

    // 平台范围标签与非当前宿主平台禁用保护
    const compatBadge = rendered.getByTestId("runtime-platform-badge-compat-worker");
    assert.ok(compatBadge.textContent?.includes("仅 Windows x64"));
    const compatMirrorBtn = rendered.getByTestId("mirror-install-btn-compat-worker") as HTMLButtonElement;
    if (process.platform !== "win32") {
      assert.equal(compatMirrorBtn.disabled, true);
      assert.ok(compatMirrorBtn.textContent?.includes("当前系统无需此组件"));
    }

    // 从本地包导入安装
    const openLocalBtn = rendered.getByTestId("open-local-install-btn-vertical-grid-egm2008");
    fireEvent.click(openLocalBtn);
    const localPathInput = rendered.getByTestId(
      "local-install-path-input-vertical-grid-egm2008"
    ) as HTMLInputElement;
    fireEvent.change(localPathInput, {
      target: { value: "D:\\EOStage\\_hub_dist\\components\\vertical-grid-egm2008-1.0.0.zip" },
    });
    fireEvent.click(rendered.getByTestId("confirm-local-install-btn-vertical-grid-egm2008"));
    await act(async () => {
      await new Promise((r) => setTimeout(r, 15));
    });
    assert.ok(rendered.getByTestId("runtime-install-banner"));

    // 从镜像源一键安装
    const mirrorInstallBtn = rendered.getByTestId("mirror-install-btn-vertical-grid-egm96");
    fireEvent.click(mirrorInstallBtn);
    await act(async () => {
      await new Promise((r) => setTimeout(r, 15));
    });
    assert.ok(rendered.getByTestId("runtime-install-banner"));
    rendered.unmount();

    // 验证当 platform_packages 仅包含其他系统包时，按钮诚实禁用为「当前系统包待上架」
    const pendingPlatformView = render(
      <RuntimeToolsSettingsSection
        fetchRegistryOverride={async () => ({
          ok: true,
          source_url: "https://raw.githubusercontent.com/hhanmj/eostage-hub/main/registry.json",
          updated_at: "2026-11-20T15:00:00Z",
          blocked_count: 0,
          entries: [
            {
              id: "native-gis-runtime",
              name: "Native GIS",
              kind: "component",
              version: "1.0.0",
              description: "Native GIS",
              package: "https://example.invalid/other.zip",
              sha256: "a".repeat(64),
              size: 1024,
              install_root: "runtime/gis",
              capabilities: ["dem-gdal-proj"],
              platforms: ["freebsd-x86_64"],
              platform_packages: {
                "freebsd-x86_64": {
                  package: "https://example.invalid/freebsd.zip",
                  sha256: "a".repeat(64),
                  size: 1024,
                },
              },
            },
          ],
        })}
      />
    );
    await act(async () => {
      await new Promise((r) => setTimeout(r, 15));
    });
    const pendingBtn = pendingPlatformView.getByTestId("mirror-install-btn-native-gis-runtime") as HTMLButtonElement;
    assert.equal(pendingBtn.disabled, true);
    assert.ok(pendingBtn.textContent?.includes("当前系统包待上架"));
    pendingPlatformView.unmount();
  });

  await t.test("5. 技能导入/导出与扩展仓库卡片网格：未安装排前、已安装排后，点击查看能力说明并安装", async () => {
    localStorage.removeItem(SKILLS_STORAGE_KEY);
    const { getByTestId, queryByTestId } = render(
      <PluginsAndSkillsPage
        initialTab="skills"
        fetchRegistryOverride={async () => ({
          ok: true,
          source_url: "https://api.gitcode.com/api/v5/repos/hhanmj/eostage-hub/raw/registry.json",
          updated_at: "2026-11-20T15:00:00Z",
          blocked_count: 0,
          entries: [
            {
              id: "skill-eo-workbench-concept",
              name: "遥感工作台任务状态与交付设计",
              kind: "skill",
              version: "1.0.0",
              description: "已包含在内置技能中的扩展条目示例",
              package: "https://example.invalid/concept.zip",
              sha256: "b".repeat(64),
              size: 1024,
              install_root: "skills",
              capabilities: ["skill-engineering"],
              skill: {
                id: "eo-workbench-concept",
                name_zh: "遥感工作台任务状态与交付设计",
                name_en: "EO Workbench Concept Design",
                category: "skill-engineering",
                description_zh: "已安装的遥感工作台状态与交付设计说明",
                description_en: "EO Workbench Concept Design",
                prompt_template: "# EO Workbench Concept",
              },
            },
            {
              id: "skill-alos2-stripmap-insar",
              name: "ALOS-2 PALSAR-2 L波段植被穿透形变监测工作流",
              kind: "skill",
              version: "1.0.0",
              description: "高植被覆盖山区滑坡与同震形变 L 波段工作流",
              package: "https://example.invalid/alos2.json",
              sha256: "c".repeat(64),
              size: 1420,
              install_root: "localStorage",
              capabilities: ["sar", "alos2"],
              skill: {
                id: "hub-skill-alos2-stripmap-insar",
                name_zh: "ALOS-2 PALSAR-2 L波段植被穿透形变监测工作流",
                name_en: "ALOS-2 PALSAR-2 L-band Workflow",
                category: "sar",
                description_zh: "用于密集植被覆盖区的 L 波段 InSAR 检索与配对指导",
                description_en: "ALOS-2 L-band InSAR workflow",
                prompt_template: "# ALOS-2 L-band InSAR Workflow\n1. 选择 Fine Beam SM1/SM3；",
              },
            },
          ],
        })}
      />
    );

    await act(async () => {
      await new Promise((r) => setTimeout(r, 15));
    });

    // 打开 JSON 导入面板并导入一条技能
    fireEvent.click(getByTestId("import-skills-btn"));
    const textarea = getByTestId("import-skills-textarea") as HTMLTextAreaElement;
    fireEvent.change(textarea, {
      target: {
        value: JSON.stringify([
          {
            id: "custom-imported-1",
            name: "外部导入的 SBAS-InSAR 技能",
            description: "用于城市沉降时序分析",
            prompt: "请执行小基线集（SBAS）时序干涉配对检查。",
          },
        ]),
      },
    });
    fireEvent.click(getByTestId("confirm-import-skills-btn"));
    assert.ok(getByTestId("skill-card-custom-imported-1"));

    // 扩展仓库卡片排序：未安装的 ALOS-2 必须排在已安装的 EO Workbench Concept 之前
    const hubSection = getByTestId("hub-skills-section");
    const hubCards = Array.from(hubSection.querySelectorAll('[data-testid^="hub-skill-card-"]'));
    assert.equal(hubCards.length, 2);
    assert.equal(hubCards[0].getAttribute("data-testid"), "hub-skill-card-skill-alos2-stripmap-insar");
    assert.equal(hubCards[1].getAttribute("data-testid"), "hub-skill-card-skill-eo-workbench-concept");
    assert.equal(getByTestId("hub-skill-status-skill-alos2-stripmap-insar").textContent, "未安装");
    assert.equal(getByTestId("hub-skill-status-skill-eo-workbench-concept").textContent, "已安装");

    // 点击「查看说明」打开能力说明详情弹窗
    fireEvent.click(getByTestId("inspect-hub-skill-btn-skill-alos2-stripmap-insar"));
    const hubInspector = getByTestId("hub-skill-inspector");
    assert.ok(hubInspector.textContent?.includes("用于密集植被覆盖区的 L 波段 InSAR 检索与配对指导"));
    assert.ok(getByTestId("hub-skill-inspector-prompt").textContent?.includes("Fine Beam SM1/SM3"));
    fireEvent.click(getByTestId("close-hub-skill-inspector-btn"));
    assert.equal(queryByTestId("hub-skill-inspector"), null);

    // 一键安装未安装技能后，状态更新为「已安装」，并出现在主技能网格中
    fireEvent.click(getByTestId("install-hub-skill-btn-skill-alos2-stripmap-insar"));
    assert.ok(getByTestId("skill-card-hub-skill-alos2-stripmap-insar"));
    assert.equal(getByTestId("hub-skill-status-skill-alos2-stripmap-insar").textContent, "已安装");
  });

  await t.test("6. 地球科学技能包目录树查看器、通过链接添加、连接器用法/启停与插件已添加/浏览插件双视图", async () => {
    localStorage.removeItem(SKILLS_STORAGE_KEY);
    localStorage.removeItem("eostage.plugins.installed.v1");
    localStorage.removeItem("eostage.plugins.custom.v1");
    const { getByTestId, queryByTestId, getByText } = render(<PluginsAndSkillsPage initialTab="skills" />);

    // 1) 验证左侧存在分类边栏（含计数），且运行组件卡片不再混杂在技能页内
    assert.ok(getByTestId("skills-category-sidebar"));
    const allCategoryBtn = getByTestId("skill-filter-all");
    assert.ok(allCategoryBtn.textContent?.includes("全部"));
    assert.ok(/\d+/.test(allCategoryBtn.textContent || ""), "分类边栏条目应包含数量统计");
    assert.equal(queryByTestId("skills-shared-components-section"), null);

    // 2) 点击「遥感影像分析 (remote-sensing-analysis)」的「查看技能包」按钮，验证作者归属与固定尺寸（w-[min(920px,92vw)] h-[min(640px,84vh)] rounded-[16px]）
    fireEvent.click(getByTestId("inspect-skill-btn-remote-sensing-analysis"));
    const inspector = getByTestId("skill-package-inspector");
    assert.ok(inspector);
    const initialClassName = inspector.className;
    assert.ok(initialClassName.includes("w-[min(920px,92vw)]"));
    assert.ok(initialClassName.includes("h-[min(640px,84vh)]"));
    assert.ok(initialClassName.includes("rounded-[16px]"));
    const dialog = inspector.closest('[role="dialog"]');
    assert.ok(dialog, "skill inspector must be rendered as a modal dialog with role='dialog'");
    assert.equal(dialog?.getAttribute("aria-modal"), "true");
    assert.ok(inspector.textContent?.includes("Muhammed Enes Duran"));
    assert.ok(!inspector.textContent?.includes("EOStage"));
    assert.ok(!inspector.textContent?.includes("作者：OpenAI"));
    assert.ok(inspector.textContent?.includes("/remote-sensing-analysis"));

    // 切换到 references/spatial-cv-protocol.md 文件节点，验证真实参考文件内容展示且外框尺寸类零变化
    fireEvent.click(getByTestId("inspector-file-references/spatial-cv-protocol.md"));
    const contentPane = getByTestId("inspector-content-pane");
    assert.ok(contentPane.textContent?.includes("Tobler") || contentPane.textContent?.includes("spatial"));
    assert.equal(getByTestId("skill-package-inspector").className, initialClassName);

    // 切换到 SKILL.md 文件节点
    fireEvent.click(getByTestId("inspector-file-SKILL.md"));
    assert.ok(contentPane.textContent?.includes("remote-sensing-analysis"));
    assert.equal(getByTestId("skill-package-inspector").className, initialClassName);

    // 切回「简介」并点击第一条示例任务胶囊
    fireEvent.click(getByTestId("inspector-node-overview"));
    fireEvent.click(getByTestId("inspector-example-task-0"));
    assert.ok(getByTestId("skills-feedback-banner"));

    // 关闭技能包查看器
    fireEvent.click(getByTestId("close-skill-inspector-btn"));
    assert.equal(queryByTestId("skill-package-inspector"), null);

    // 重新打开并验证受 ModalOverlay 统一蒙版保护：点击遮罩背景不会误关闭弹窗，底部操作栏正常闭环，Escape 键可关闭
    fireEvent.click(getByTestId("inspect-skill-btn-remote-sensing-analysis"));
    const overlay = getByTestId("skill-inspector-overlay");
    assert.equal(overlay.getAttribute("data-overlay-layer"), "base");
    assert.ok(getByTestId("skill-package-inspector"));
    assert.ok(getByTestId("skill-inspector-footer"));
    fireEvent.click(overlay);
    assert.ok(
      getByTestId("skill-package-inspector"),
      "ModalOverlay(closeOnBackdropClick=false) 必须防止点击背景误关闭查看器",
    );
    fireEvent.keyDown(window, { key: "Escape" });
    assert.equal(queryByTestId("skill-package-inspector"), null);

    // 验证底部「完成」按钮也能关闭查看器
    fireEvent.click(getByTestId("inspect-skill-btn-remote-sensing-analysis"));
    fireEvent.click(getByTestId("inspector-done-btn"));
    assert.equal(queryByTestId("skill-package-inspector"), null);

    // 3) 测试「+ 通过链接添加」按钮与真实联网 SKILL.md 拉取（失败闭环 + 成功导入）
    const origFetch = globalThis.fetch;
    try {
      (globalThis as any).fetch = async () =>
        new Response("<html>not a skill</html>", { status: 200 });
      fireEvent.click(getByTestId("add-skill-by-url-btn"));
      const urlInput = getByTestId("add-skill-by-url-input") as HTMLInputElement;
      fireEvent.change(urlInput, {
        target: { value: "https://example.invalid/not-a-skill.html" },
      });
      await act(async () => {
        fireEvent.click(getByTestId("confirm-add-skill-by-url-btn"));
        await new Promise((r) => setTimeout(r, 20));
      });
      assert.ok(getByTestId("skills-feedback-banner").textContent?.includes("有效的 SKILL.md"));

      (globalThis as any).fetch = async () =>
        new Response(
          "---\nname: earthengine-flood-skill\ndescription: 真实拉取的 GEE 洪水制图技能\n---\n\n# Flood Mapping\nUse Sentinel-1 SAR threshold.",
          { status: 200 },
        );
      fireEvent.change(urlInput, {
        target: { value: "https://github.com/google/earthengine-flood-skill.md" },
      });
      await act(async () => {
        fireEvent.click(getByTestId("confirm-add-skill-by-url-btn"));
        await new Promise((r) => setTimeout(r, 20));
      });
      assert.ok(getByTestId("skills-feedback-banner").textContent?.includes("已通过链接成功添加"));
      assert.ok(loadCustomSkills().some((s) => s.name === "earthengine-flood-skill"));
    } finally {
      globalThis.fetch = origFetch;
    }

    // 4) 直接导入标准 SKILL.md (含 YAML Frontmatter)
    fireEvent.click(getByTestId("import-skills-btn"));
    const textarea = getByTestId("import-skills-textarea") as HTMLTextAreaElement;
    fireEvent.change(textarea, {
      target: {
        value: `---\nname: ps-insar-urban-subsidence\ndescription: 城市地铁沿线永久散射体 PS-InSAR 沉降速率反演与相干点筛选规程\nlicense: MIT\n---\n\n# PS-InSAR 城市沉降分析\n\n1. 选取振幅离差指数 D_A < 0.25 的候选永久散射体点；\n2. 扣除外部 DEM 残差与大气相位屏幕 (APS)。`,
      },
    });
    fireEvent.click(getByTestId("confirm-import-skills-btn"));
    assert.ok(getByTestId("skill-card-custom-ps-insar-urban-subsidence"));

    // 5) 切换到「扩展与工具连接 → MCP 与外部工具」，验证无假内置 eostage:// 连接器，并可新建/查看/停用/启用自定义 MCP 连接器
    fireEvent.click(getByTestId("tab-extensions"));
    fireEvent.click(getByTestId("tab-tools"));
    assert.equal(queryByTestId("connector-card-connector-workbench-map-mcp"), null);
    assert.ok(getByTestId("connectors-empty-state"));
    fireEvent.click(getByTestId("add-custom-connector-btn"));
    fireEvent.change(getByTestId("custom-connector-name-input"), {
      target: { value: "PostGIS Local MCP" },
    });
    fireEvent.change(getByTestId("custom-connector-endpoint-input"), {
      target: { value: "http://127.0.0.1:8080/mcp" },
    });
    fireEvent.click(getByTestId("save-custom-connector-btn"));
    assert.equal(queryByTestId("connectors-empty-state"), null);
    assert.ok(getByText("PostGIS Local MCP"));

    // 6) 切换到「插件」页签，验证「已添加」初始为 0（不将内置技能伪装成默认已装插件），与「浏览插件」是两个完全不同的真实视图
    // Note: Per Owner feedback #5, prompt-only fake solver plugins (eostage-sbas-insar-plugin etc.) and unbundled CLI stubs were removed; only native/Rust-backed plugins remain.
    fireEvent.click(getByTestId("tab-plugins"));
    assert.ok(getByTestId("plugins-panel"));
    assert.ok(getByTestId("plugins-added-view"));
    assert.ok(getByTestId("plugins-added-empty"));
    assert.equal(getByTestId("plugins-subtab-added").textContent, "已添加 (0)");
    assert.equal(queryByTestId("plugin-suite-card-eostage-gdal-stream-plugin"), null);

    // 切换到「浏览插件」视图，验证仅展示有真实底层能力支撑的插件并可点击「添加插件」
    fireEvent.click(getByTestId("plugins-subtab-browse"));
    assert.ok(getByTestId("plugins-browse-view"));
    assert.equal(queryByTestId("browse-plugin-card-eostage-sbas-insar-plugin"), null);
    assert.equal(queryByTestId("browse-plugin-card-eostage-stac-harmonized-plugin"), null);
    assert.equal(queryByTestId("browse-plugin-card-eostage-whitebox-duckdb-plugin"), null);
    assert.ok(getByTestId("browse-plugin-card-eostage-gee-plugin"));
    assert.ok(getByTestId("browse-plugin-card-eostage-gdal-stream-plugin"));
    assert.ok(getByTestId("browse-plugin-card-eostage-open-geodata-plugin"));
    assert.ok(getByTestId("browse-plugin-card-eostage-mapgis-compat-plugin"));
    fireEvent.click(getByTestId("browse-install-plugin-btn-eostage-gdal-stream-plugin"));
    assert.equal(getByTestId("plugins-subtab-added").textContent, "已添加 (1)");

    // 切回「已添加」视图，验证刚添加的 GDAL 流式插件已真实出现且使用统一 switch 开关
    fireEvent.click(getByTestId("plugins-subtab-added"));
    assert.ok(getByTestId("plugin-suite-card-eostage-gdal-stream-plugin"));
    const pluginSwitch = getByTestId("plugin-toggle-btn-eostage-gdal-stream-plugin");
    assert.equal(pluginSwitch.getAttribute("role"), "switch");
    assert.equal(pluginSwitch.getAttribute("aria-checked"), "true");
    fireEvent.click(pluginSwitch);
    assert.equal(pluginSwitch.getAttribute("aria-checked"), "false");
    fireEvent.click(pluginSwitch);
    assert.equal(pluginSwitch.getAttribute("aria-checked"), "true");

    // 测试右上角「本地导入 .zip」按钮，确认弹窗圆角与蒙版保护，并导入自定义 plugin.json 持久化到「已添加」
    fireEvent.click(getByTestId("import-plugin-bundle-btn"));
    const importModal = getByTestId("import-plugin-bundle-modal");
    assert.ok(importModal.className.includes("rounded-[16px]"));
    fireEvent.click(getByTestId("import-plugin-bundle-overlay"));
    assert.ok(
      getByTestId("import-plugin-bundle-modal"),
      "closeOnBackdropClick=false prevents accidental discard during plugin import"
    );
    fireEvent.change(getByTestId("import-plugin-textarea"), {
      target: {
        value: JSON.stringify({
          id: "custom-hydro-plugin",
          name: "EOStage 流域水文分析插件",
          version: "1.2.0",
          author: "EOStage",
          description: "集成流域水文与地下水时间序列模拟工具集",
        }),
      },
    });
    fireEvent.click(getByTestId("confirm-import-plugin-btn"));
    assert.ok(getByTestId("plugin-suite-card-custom-hydro-plugin"));
    assert.equal(getByTestId("plugins-subtab-added").textContent, "已添加 (2)");
  });

  await t.test("7. 技能来源与协议准确归属、多源插件市场定位与自定义插件 Popconfirm 移除", async () => {
    // 1) 全量校验 32 个内置地球科学技能包作者与协议归属，杜绝 OpenAI / Google / ScenePort 与无协议技能包
    assert.equal(GEOSCIENCE_SKILL_PACKAGES.length, 29);
    for (const bundledId of [
      "eo-analysis-recipes",
      "eo-agent-capability-routing",
      "eo-model-explanation-review",
      "eo-model-transfer-evaluation",
      "eo-workbench-concept",
    ]) {
      const foundPkg = GEOSCIENCE_SKILL_PACKAGES.find((p) => p.id === bundledId);
      assert.ok(foundPkg, `应内置打包技能包 ${bundledId}`);
      assert.equal(foundPkg?.author, "EOStage");
    }
    for (const pkg of GEOSCIENCE_SKILL_PACKAGES) {
      assert.notEqual(pkg.author, "OpenAI", `${pkg.id} 不应被误标为 OpenAI`);
      assert.notEqual(pkg.author, "Google", `${pkg.id} 不应被误标为 Google`);
      assert.notEqual(pkg.author, "ScenePort", `${pkg.id} 不应遗留旧称 ScenePort`);
      assert.ok(pkg.license && !pkg.license.includes("未声明"), `${pkg.id} 必须声明有效开源协议`);
    }
    const rsaPkg = GEOSCIENCE_SKILL_PACKAGES.find((p) => p.id === "remote-sensing-analysis");
    const ecmwfPkg = GEOSCIENCE_SKILL_PACKAGES.find((p) => p.id === "ecmwf-open-data");
    const demPkg = GEOSCIENCE_SKILL_PACKAGES.find((p) => p.id === "dem-datum-conversion");
    const creatorPkg = GEOSCIENCE_SKILL_PACKAGES.find((p) => p.id === "skill-creator");
    const converterPkg = GEOSCIENCE_SKILL_PACKAGES.find((p) => p.id === "geospatial-converter");
    const radarPkg = GEOSCIENCE_SKILL_PACKAGES.find((p) => p.id === "radar-satellite-analysis");
    const geophysPkg = GEOSCIENCE_SKILL_PACKAGES.find((p) => p.id === "near-surface-geophysics");
    const geeTextbookPkg = GEOSCIENCE_SKILL_PACKAGES.find((p) => p.id === "gee-textbook-cookbook");
    const insarTextbookPkg = GEOSCIENCE_SKILL_PACKAGES.find((p) => p.id === "sar-insar-textbook");
    assert.equal(rsaPkg?.author, "Muhammed Enes Duran");
    assert.equal(ecmwfPkg?.author, "ECMWF");
    assert.equal(demPkg?.author, "EOStage");
    assert.equal(creatorPkg?.author, "Anthropic (anthropics/skills)");
    assert.equal(creatorPkg?.license, "Apache-2.0");
    assert.equal(converterPkg, undefined, "无上游开源协议的 geospatial-converter 不应被打包分发");
    assert.equal(radarPkg?.author, "EOStage");
    assert.equal(geophysPkg, undefined);
    assert.ok(geeTextbookPkg && geeTextbookPkg.fileCount >= 5);
    assert.ok(insarTextbookPkg && insarTextbookPkg.fileCount >= 5);

    // 2) 左侧栏一级工作区入口（workspace surface）：呈现自解释「插件与技能」，且在未连接远端注册表时诚实展示「本地内置目录（离线回退）」而非虚标已连接
    localStorage.removeItem("eostage.plugins.installed.v1");
    localStorage.removeItem("eostage.plugins.custom.v1");
    const workspaceView = render(<PluginsAndSkillsPage />);
    assert.equal(
      workspaceView.getByTestId("plugins-and-skills-page").getAttribute("data-surface"),
      "workspace"
    );
    assert.ok(workspaceView.getByText("插件与技能"));
    fireEvent.click(workspaceView.getByTestId("tab-extensions"));
    fireEvent.click(workspaceView.getByTestId("tab-plugins"));
    const sourcesBar = workspaceView.getByTestId("plugin-marketplace-sources-bar");
    assert.equal(sourcesBar.getAttribute("data-registry-state"), "offline-fallback");
    assert.ok(sourcesBar.textContent?.includes("本地内置目录（离线回退）"));
    assert.ok(
      !sourcesBar.textContent?.includes("已连接市场源"),
      "未完成真实远程注册表拉取时严禁虚标已连接市场源"
    );
    assert.ok(
      !workspaceView.getByTestId("plugins-panel").textContent?.includes("打包安装"),
      "插件市场页不应再将插件描述为本地打包器"
    );

    // 验证本地自定义插件移出时保留 Popconfirm 确认，内置市场插件直接移出
    fireEvent.click(workspaceView.getByTestId("plugins-subtab-browse"));
    fireEvent.click(
      workspaceView.getByTestId("browse-install-plugin-btn-eostage-gdal-stream-plugin")
    );
    fireEvent.click(workspaceView.getByTestId("import-plugin-bundle-btn"));
    fireEvent.change(workspaceView.getByTestId("import-plugin-textarea"), {
      target: {
        value: JSON.stringify({
          id: "custom-local-zip-plugin",
          name: "本地开发测试插件",
          version: "0.1.0",
          description: "验证自定义插件移出确认气泡",
        }),
      },
    });
    fireEvent.click(workspaceView.getByTestId("confirm-import-plugin-btn"));
    assert.equal(workspaceView.getByTestId("plugins-subtab-added").textContent, "已添加 (2)");

    // 内置市场插件直接移出，无需多余确认
    fireEvent.click(
      workspaceView.getByTestId("plugin-remove-btn-eostage-gdal-stream-plugin")
    );
    assert.equal(workspaceView.queryByTestId("sp-popconfirm"), null);
    assert.equal(workspaceView.getByTestId("plugins-subtab-added").textContent, "已添加 (1)");

    // 本地导入插件移出触发 Popconfirm 确认后移出
    fireEvent.click(workspaceView.getByTestId("plugin-remove-btn-custom-local-zip-plugin"));
    assert.ok(workspaceView.getByTestId("sp-popconfirm"));
    assert.equal(workspaceView.getByTestId("plugins-subtab-added").textContent, "已添加 (1)");
    fireEvent.click(workspaceView.getByTestId("popconfirm-confirm-btn"));
    assert.equal(workspaceView.getByTestId("plugins-subtab-added").textContent, "已添加 (0)");
    cleanup();

    // 2b) 验证远程注册表状态机三态真实跃迁：同步中 (syncing) -> 远端成功 (connected) -> 远端失败回退 (offline-fallback)
    let resolveRemote!: (val: any) => void;
    let shouldRejectNext = false;
    const liveView = render(
      <PluginsAndSkillsPage
        initialTab="plugins"
        fetchRegistryOverride={() => {
          if (shouldRejectNext) {
            return Promise.reject(new Error("network unreachable"));
          }
          return new Promise((res) => {
            resolveRemote = res;
          });
        }}
      />
    );
    const liveBar = liveView.getByTestId("plugin-marketplace-sources-bar");
    assert.equal(liveBar.getAttribute("data-registry-state"), "syncing");
    assert.ok(liveBar.textContent?.includes("同步中..."));
    assert.ok(!liveBar.textContent?.includes("已连接市场源"));

    await act(async () => {
      resolveRemote({
        ok: true,
        source_url: "https://raw.githubusercontent.com/hhanmj/eostage-hub/main/registry.json",
        updated_at: "2026-11-12T00:00:00Z",
        blocked_count: 0,
        entries: [],
      });
      await new Promise((r) => setTimeout(r, 5));
    });
    assert.equal(liveBar.getAttribute("data-registry-state"), "connected");
    assert.ok(liveBar.textContent?.includes("已连接市场源："));
    assert.ok(liveBar.textContent?.includes("EOStage 官方扩展源 (eostage-hub v2)"));

    shouldRejectNext = true;
    await act(async () => {
      fireEvent.click(liveView.getByTestId("sync-plugin-registry-btn"));
      await new Promise((r) => setTimeout(r, 5));
    });
    assert.equal(liveBar.getAttribute("data-registry-state"), "offline-fallback");
    assert.ok(liveBar.textContent?.includes("本地内置目录（离线回退）"));
    assert.ok(!liveBar.textContent?.includes("已连接市场源："));
    cleanup();

    // 3) 设置弹窗内嵌入口（.eo-settings-card 自动检测为 settings surface）：
    // 聚焦「MCP 与外部工具连接」，不重复堆叠整页技能卡片与运行组件（后者位于「设置 → 运行环境与工具」）
    const settingsView = render(
      <div className="eo-settings-card">
        <div className="eo-settings-card-content p-0 overflow-hidden">
          <PluginsAndSkillsPage />
        </div>
      </div>
    );
    assert.equal(
      settingsView.getByTestId("plugins-and-skills-page").getAttribute("data-surface"),
      "settings"
    );
    assert.ok(settingsView.getByText("MCP 与外部工具连接"));
    assert.ok(settingsView.getByTestId("tools-panel"));
    assert.equal(settingsView.queryByTestId("tab-skills"), null);
    assert.equal(settingsView.queryByTestId("tab-plugins"), null);
    assert.equal(settingsView.queryByTestId("tab-runtime"), null);
    assert.equal(settingsView.queryByTestId("jump-to-settings-btn"), null);
  });

  await t.test("7. 诚实 4 插件目录：后端 available_plugins 中的退役空壳插件绝不混入浏览市场", async () => {
    localStorage.removeItem("eostage.plugins.installed.v1");
    localStorage.removeItem("eostage.plugins.custom.v1");
    localStorage.removeItem("eostage.agent.installedPluginIds.v1");
    localStorage.removeItem("eostage.agent.customPlugins.v1");

    const { getByTestId, queryByTestId } = render(
      <PluginsAndSkillsPage
        initialTab="plugins"
        getPluginsStatusOverride={async () => ({
          ok: true,
          plugins_dir: "/tmp/plugins",
          installed_plugins: [],
          available_plugins: [
            {
              id: "eostage-sbas-insar-plugin",
              name: "EOStage SBAS-InSAR 时序形变解算与闭合相位质控插件",
              version: "1.0.0",
              description: "retired",
              author: "EOStage",
              license: "Apache-2.0",
              discipline: "insar",
              tags: [],
              enabled: false,
              install_path: "/tmp/sbas",
              sha256: "abc",
              files: [],
              bundled_skills: [],
            },
            {
              id: "eostage-grace-hydro-plugin",
              name: "GRACE",
              version: "1.0.0",
              description: "retired",
              author: "EOStage",
              license: "Apache-2.0",
              discipline: "hydro",
              tags: [],
              enabled: false,
              install_path: "/tmp/grace",
              sha256: "def",
              files: [],
              bundled_skills: [],
            },
          ],
          custom_connectors: [],
          custom_skills: [],
        })}
      />
    );

    await act(async () => {
      await new Promise((r) => setTimeout(r, 20));
    });

    fireEvent.click(getByTestId("plugins-subtab-browse"));
    assert.equal(document.querySelectorAll('[data-testid^="browse-plugin-card-"]').length, 4);
    assert.equal(queryByTestId("browse-plugin-card-eostage-sbas-insar-plugin"), null);
    assert.equal(queryByTestId("browse-plugin-card-eostage-grace-hydro-plugin"), null);
    const geeCard = getByTestId("browse-plugin-card-eostage-gee-plugin");
    assert.ok(geeCard);
    assert.ok(geeCard.textContent?.includes("Google Earth Engine 科学目录与云端计算插件"));

    // 点击「预览包含的技能 (1)」应在当前插件页弹出只读详情弹窗，而非盲跳走丢失上下文
    fireEvent.click(getByTestId("btn-preview-plugin-skills-eostage-gee-plugin"));
    const previewModal = getByTestId("plugin-skills-preview-modal");
    assert.ok(previewModal.textContent?.includes("未安装 · 仅预览内容"));
    assert.ok(previewModal.textContent?.includes("gee_search_catalog"));
    assert.ok(previewModal.textContent?.includes("eostage-gee-mcp"));
    assert.ok(getByTestId("plugins-browse-view"));
    fireEvent.click(getByTestId("plugin-skills-preview-close-btn"));
    assert.ok(queryByTestId("plugin-skills-preview-modal") === null);
  });

  await t.test("8. 移除过时的「镜像源设置」按钮，保留「设置 → 运行环境与工具」跳转并正确标注双平台组件", async () => {
    let runtimeNavCount = 0;
    const { getByTestId, queryByTestId } = render(
      <PluginsAndSkillsPage
        initialTab="plugins"
        onOpenRuntimeSettings={() => {
          runtimeNavCount += 1;
        }}
      />
    );
    await act(async () => {
      await new Promise((r) => setTimeout(r, 15));
    });

    fireEvent.click(getByTestId("plugins-subtab-browse"));
    assert.ok(queryByTestId("open-runtime-mirror-settings-btn") === null);
    assert.ok(!(getByTestId("plugin-marketplace-sources-bar").textContent || "").includes("镜像源设置"));

    const jumpBtn = getByTestId("open-runtime-tools-settings-btn");
    fireEvent.click(jumpBtn);
    assert.equal(runtimeNavCount, 1);
    cleanup();

    const rtView = render(
      <RuntimeToolsSettingsSection
        getRuntimeComponentsOverride={async () => ({
          ok: true,
          install_root: "/tmp/runtime",
          manifest_found: true,
          app_version: "2.2.0",
          components: [
            {
              id: "native-gis-runtime",
              name: "Native GIS 高级地理信息引擎 (GDAL / PROJ)",
              kind: "runtime",
              version: "1.0.0",
              install_root: "runtime/gis",
              status: "missing",
              file_count: 0,
              total_bytes: 0,
              verified: false,
              detail: "GDAL/PROJ",
              capabilities: ["dem-gdal-proj"],
            },
          ],
        })}
        fetchRegistryOverride={async () => ({
          ok: true,
          source: "https://gitcode.com/hhanmj/eostage-hub/raw/main/registry.json",
          version: 1,
          updated_at: "2026-11-23T09:52:00Z",
          blocked_count: 0,
          entries: [
            {
              id: "native-gis-runtime",
              name: "Native GIS",
              kind: "component",
              version: "1.0.0",
              description: "GDAL/PROJ",
              package: "https://gitcode.com/hhanmj/eostage-hub/releases/download/v1.1.0/native-gis-runtime-1.0.0.zip",
              sha256: "f".repeat(64),
              size: 14025059,
              install_root: "runtime/gis",
              capabilities: ["dem-gdal-proj"],
              platforms: ["macos-aarch64", "windows-x86_64"],
            },
          ],
        })}
      />
    );
    await act(async () => {
      await new Promise((r) => setTimeout(r, 15));
    });
    assert.equal(
      rtView.getByTestId("runtime-platform-badge-native-gis-runtime").textContent?.trim(),
      "Windows x64 / macOS Apple Silicon"
    );
  });
});

test("AgentComposer - 技能与工作流真实注入测试", async (t) => {
  const mockChannelReadiness = {
    ready: true,
    disabled: false,
    reason_zh: "",
  };

  await t.test("1. 点击「工作流」项弹出真实技能选择器，并把选定技能提示词注入输入框", async () => {
    let currentInput = "";
    function ComposerWrapper() {
      const [text, setText] = useState("");
      return (
        <AgentComposer
          value={text}
          onInputChange={(v) => {
            currentInput = v;
            setText(v);
          }}
          onSubmit={() => {}}
          channelReadiness={mockChannelReadiness}
          channels={[]}
          sessionId="test-session"
          permissionView={null}
          permissionStatus="ready"
          onPermissionSelect={() => {}}
          onPermissionSetGlobal={() => {}}
        />
      );
    }

    const { getByRole, getByTestId, queryByTestId, getByText } = render(
      <ComposerWrapper />
    );

    // 1) 打开「+」附加菜单
    const attachBtn = getByRole("button", { name: /附加/ });
    fireEvent.click(attachBtn);

    // 2) 找到并点击「工作流」选项
    const workflowMenuItem = getByText("工作流");
    fireEvent.click(workflowMenuItem);

    // 3) 验证技能选择弹层已展开
    const skillPicker = getByTestId("agent-skill-picker");
    assert.ok(skillPicker);

    // 4) 点击「SAR 与雷达卫星分析」
    const pairWorkflowItem = getByTestId(
      "skill-picker-item-radar-satellite-analysis"
    );
    fireEvent.click(pairWorkflowItem);

    // 5) 弹层自动关闭
    assert.ok(queryByTestId("agent-skill-picker") === null);

    // 6) 验证输入框已被真实注入该内置权威技能的英文规程提示词
    assert.ok(
      currentInput.includes("Radar & SAR Satellite Analysis"),
      "输入框应包含 SAR 权威技能英文提示词核心内容"
    );
  });

  await t.test("2. 已有文字时注入技能，提示词通过双换行无缝拼接", async () => {
    let currentInput = "请处理陕西省西安市区域";
    function ComposerWrapper() {
      const [text, setText] = useState("请处理陕西省西安市区域");
      return (
        <AgentComposer
          value={text}
          onInputChange={(v) => {
            currentInput = v;
            setText(v);
          }}
          onSubmit={() => {}}
          channelReadiness={mockChannelReadiness}
          channels={[]}
          sessionId="test-session"
          permissionView={null}
          permissionStatus="ready"
          onPermissionSelect={() => {}}
          onPermissionSetGlobal={() => {}}
        />
      );
    }

    const { getByRole, getByTestId, getByText } = render(<ComposerWrapper />);

    // 打开菜单并选择技能
    const attachBtn = getByRole("button", { name: /附加/ });
    fireEvent.click(attachBtn);

    const workflowMenuItem = getByText("工作流");
    fireEvent.click(workflowMenuItem);

    const pairWorkflowItem = getByTestId(
      "skill-picker-item-radar-satellite-analysis"
    );
    fireEvent.click(pairWorkflowItem);

    // 验证开头保留了原输入，后追加了完整提示词
    assert.ok(currentInput.startsWith("请处理陕西省西安市区域\n\n"));
    assert.ok(currentInput.includes("Radar & SAR Satellite Analysis"));
  });
});
