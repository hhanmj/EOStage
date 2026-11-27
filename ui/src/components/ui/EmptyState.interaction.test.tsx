import assert from "node:assert/strict";
import test from "node:test";
import React from "react";
import { cleanup, render } from "@/test-utils/render-harness";
import { EmptyState } from "./EmptyState";
import { EmptyStateIcon, EmptyStateWireframe } from "./EmptyStateVisuals";

test.afterEach(() => {
  cleanup();
});

test("EmptyState renders page, card, row, and menu sizes cleanly with structural and a11y rules", () => {
  const { container, getByRole, queryByText, getByTestId } = render(
    <div>
      <EmptyState
        kind="first-use"
        size="page"
        testId="es-page"
        visual={<EmptyStateWireframe variant="channel-setup" showAccentAction />}
        title="暂无自定义模型渠道"
        description="添加模型供应商与密钥后，即可在智能工作台选用对应模型。"
        primaryAction={{ label: "添加渠道", onClick: () => {} }}
        secondaryAction={{ label: "浏览预设", onClick: () => {} }}
      />
      <EmptyState
        kind="no-results"
        size="page"
        testId="es-no-results"
        visual={<EmptyStateWireframe variant="no-results" />}
        title="没有匹配的结果"
        description="请更换关键词或清除筛选后重试。"
        primaryAction={{ label: "清除筛选", onClick: () => {} }}
      />
      <EmptyState
        kind="all-done"
        size="card"
        testId="es-all-done"
        visual={<EmptyStateIcon name="inbox" size={22} neutral />}
        title="当前没有进行中的下载"
        description="不应在 card 尺寸下显示此说明"
        primaryAction={{ label: "不应出现的按钮", onClick: () => {} }}
      />
      <EmptyState
        kind="failed"
        size="row"
        testId="es-failed-row"
        title="快视图加载失败"
        primaryAction={{ label: "重试", onClick: () => {}, loading: true }}
      />
      <EmptyState
        kind="no-results"
        size="menu"
        testId="es-menu"
        title="未找到匹配模型"
      />
      <EmptyState
        kind="no-access"
        size="page"
        testId="es-no-access"
        visual={<EmptyStateWireframe variant="no-access" />}
        title="该页面需要桌面端权限"
        description="请在桌面客户端中打开查看。"
      />
    </div>
  );

  // no-results mounts polite live region(s)
  const liveNoResults = getByTestId("es-no-results");
  assert.equal(liveNoResults.getAttribute("aria-live"), "polite");
  assert.equal(liveNoResults.getAttribute("role"), "status");
  assert.equal(getByTestId("es-menu").getAttribute("aria-live"), "polite");

  // card size hides description, all-done suppresses buttons
  assert.equal(queryByText("不应在 card 尺寸下显示此说明"), null);
  assert.equal(queryByText("不应出现的按钮"), null);

  // failed loading shows spinning RefreshCw icon inside retry button
  const failedRow = getByTestId("es-failed-row");
  assert.ok(failedRow.querySelector("svg.animate-spin"));

  // SVG icons & wireframes obey strict purity rules
  const svgs = Array.from(container.querySelectorAll("svg"));
  assert.ok(svgs.length >= 4);
  for (const svg of svgs) {
    assert.equal(svg.getAttribute("aria-hidden"), "true");
  }
  assert.equal(container.querySelectorAll("polygon").length, 0);
  assert.equal(container.querySelectorAll("feDropShadow").length, 0);
  assert.equal(container.querySelectorAll("image").length, 0);
  assert.equal(container.querySelectorAll("use").length, 0);
  void getByRole;
});

test("EmptyStateIcon supports all icon variants at 48px and 22px", () => {
  const { container } = render(
    <div>
      <EmptyStateIcon name="folder" size={48} />
      <EmptyStateIcon name="layers" size={22} />
      <EmptyStateIcon name="plug" size={22} />
      <EmptyStateIcon name="history" size={48} />
      <EmptyStateIcon name="chart" size={22} />
      <EmptyStateIcon name="inbox" size={22} neutral />
      <EmptyStateIcon name="archive" size={22} neutral />
      <EmptyStateIcon name="bell" size={48} neutral />
      <EmptyStateIcon name="alert" size={48} />
      <EmptyStateIcon name="wifi-off" size={48} />
      <EmptyStateIcon name="search" size={48} />
      <EmptyStateIcon name="filter" size={22} />
      <EmptyStateIcon name="lock" size={48} neutral />
    </div>
  );
  assert.equal(container.querySelectorAll("svg").length, 13);
});
