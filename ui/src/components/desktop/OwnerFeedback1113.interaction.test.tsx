import "../../test-utils/setup-dom";
import assert from "node:assert/strict";
import test from "node:test";
import React from "react";
import { render, cleanup, fireEvent } from "../../test-utils/render-harness";
import { DesktopDownloadDrawer } from "./DesktopDownloadDrawer";
import { ChannelModelsEditor } from "../../shell/components/settings/ChannelModelsEditor";

test.afterEach(() => {
  cleanup();
});

test("Owner 11-13 反馈：下载抽屉完整页入口与模型测速按钮反馈", async (suite) => {
  await suite.test("1. 抽屉右上角提供「进入完整下载中心」按钮，点击回调一次", () => {
    let opened = 0;
    const view = render(
      <DesktopDownloadDrawer open tasks={[]} onOpenFullPage={() => { opened += 1; }} />,
    );
    const btn = view.getByTestId("download-drawer-open-full-page-btn");
    assert.equal(btn.getAttribute("aria-label"), "进入完整下载中心");
    fireEvent.click(btn);
    assert.equal(opened, 1);
  });

  await suite.test("2. 页面形态（variant=page）不显示该入口", () => {
    const view = render(
      <DesktopDownloadDrawer open variant="page" tasks={[]} onOpenFullPage={() => {}} />,
    );
    assert.equal(view.queryByTestId("download-drawer-open-full-page-btn"), null);
  });

  await suite.test("3. 正在测速的模型按钮转圈、标记忙碌，其它测速按钮禁用", () => {
    const models = [
      { id: "model-a", name: "Model A" },
      { id: "model-b", name: "Model B" },
    ] as any;
    const view = render(
      <ChannelModelsEditor
        models={models}
        protocol="openai"
        onFetch={() => {}}
        onChange={() => {}}
        onTestModel={() => {}}
        testingModelId="model-a"
      />,
    );
    const a = view.getByTestId("model-test-btn-0") as HTMLButtonElement;
    const b = view.getByTestId("model-test-btn-1") as HTMLButtonElement;
    assert.equal(a.getAttribute("aria-busy"), "true");
    assert.ok(a.querySelector(".animate-spin"), "测速中的按钮必须显示转圈图标");
    assert.equal(a.disabled, true);
    assert.equal(b.disabled, true);
    assert.equal(b.querySelector(".animate-spin"), null);
  });

  await suite.test("4. 未测速时按钮可点击并触发对应模型测速", () => {
    const calls: string[] = [];
    const view = render(
      <ChannelModelsEditor
        models={[{ id: "model-a", name: "Model A" }] as any}
        protocol="openai"
        onFetch={() => {}}
        onChange={() => {}}
        onTestModel={(id) => calls.push(id)}
        testingModelId={null}
      />,
    );
    const a = view.getByTestId("model-test-btn-0") as HTMLButtonElement;
    assert.equal(a.disabled, false);
    fireEvent.click(a);
    assert.deepEqual(calls, ["model-a"]);
  });
});
