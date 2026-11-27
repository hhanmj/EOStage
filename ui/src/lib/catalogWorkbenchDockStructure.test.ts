import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";

const source = readFileSync(
  new URL("../components/CatalogWorkbenchDock.tsx", import.meta.url),
  "utf8",
);

test("bottom dock exposes one controlled result surface without duplicate primary tabs", () => {
  assert.match(source, /data-catalog-workbench-dock/);
  assert.match(source, /aria-label="目录与 DEM 结果工作坞"/);
  assert.doesNotMatch(source, /role="tablist"/);
  assert.doesNotMatch(source, /role="tab"/);
  assert.doesNotMatch(source, /function PreviewBody/);
  assert.match(source, /"折叠底部工作坞"/);
  assert.match(source, /"展开底部工作坞"/);
});

test("row activation and download selection remain different callbacks", () => {
  assert.match(source, /onActivateProduct\(row\.provider_product_key\)/);
  assert.match(source, /onToggleProduct\(row\.provider_product_key, event\.currentTarget\.checked\)/);
  assert.match(source, /event\.stopPropagation\(\)/);
  assert.match(source, /onDownloadSelected/);
  assert.match(source, /virtualRowWindow/);
  assert.match(source, /onPauseSelected/);
  assert.match(source, /onResumeSelected/);
  assert.match(source, /selectedPausableCount/);
  assert.match(source, /selectedResumableCount/);
  assert.match(source, /onPauseProduct\(row\.provider_product_key\)/);
  assert.match(source, /onResumeProduct\(row\.provider_product_key\)/);
  assert.match(source, /onDownloadProduct\(row\.provider_product_key\)/);
  assert.match(source, /activeDownloadProductKeys\.has\(row\.provider_product_key\)/);
  assert.match(source, /pausedDownloadProductKeys\.has\(row\.provider_product_key\)/);
  assert.match(source, /taskProductKeys\.has\(row\.provider_product_key\)/);
  assert.match(source, /onFilterQueryChange\(event\.currentTarget\.value\)/);
  assert.match(source, /rows\.length === 0/);
  assert.match(source, /scrollTopFromPageIndex\(pageIndex, pageSize, CATALOG_ROW_HEIGHT\)/);
  assert.match(source, /pageIndexFromScrollTop\(\s*nextScrollTop,\s*pageSize,\s*CATALOG_ROW_HEIGHT,?\s*\)/);
  assert.match(source, /onPageIndexChange\(nextPageIndex\)/);
  assert.match(source, /role="button"/);
  assert.match(source, /event\.key === "Enter" \|\| event\.key === " "/);
  assert.match(source, /CATALOG_ROW_HEIGHT = 72/);
});

test("metadata and asset details use the compact info interaction", () => {
  assert.match(source, /onShowProductInfo/);
  assert.match(source, /onHideProductInfo/);
  assert.match(source, /onCopyProductInfo/);
  assert.match(source, /悬浮查看元数据，点击复制/);
  assert.match(source, /row\.asset_summary/);
  assert.doesNotMatch(source, /\{row\.collection\}/);
  assert.match(source, /\{row\.platform\}/);
  assert.doesNotMatch(source, /MapPreviewPanel/);
  assert.doesNotMatch(source, /<MapPreviewPanel|preview=/);
  assert.doesNotMatch(source, /useState<\s*SarProduct\[\]/);
  assert.doesNotMatch(source, /useState<\s*DemDockProjection\[\]/);
  assert.doesNotMatch(source, /useState<\s*Set<string>/);
});

test("dock resizing and dense scientific rows keep explicit accessible controls", () => {
  assert.match(source, /searchPending && \([\s\S]{0,360}data-catalog-search-pending/);
  assert.doesNotMatch(source, /searchPending \? \([\s\S]{0,700}rows\.length === 0/);
  assert.doesNotMatch(source, /!searchPending && <div className="relative min-w-\[820px\]"/);
  assert.match(source, /!searchPending && rows\.length === 0/);
  assert.match(source, /role="separator"/);
  assert.match(source, /aria-orientation="horizontal"/);
  assert.match(source, /onPointerDown/);
  assert.match(source, /setPointerCapture/);
  assert.match(source, /onPreviewProduct/);
  assert.match(source, /Eye/);
  assert.match(source, /定位并预览/);
  assert.match(source, /row\.file_size_label/);
  assert.match(source, /h-\[72px\]/);
  assert.match(source, /border-b-2 border-border\/70/);
  assert.match(source, /variant="outline"[^>]*className="h-7"[^>]*onClick=\{onSelectAll\}/s);
  assert.match(source, /variant="outline"[^>]*className="h-7"[^>]*onClick=\{onClearSelection\}/s);
  assert.doesNotMatch(source, />已勾选</);
  assert.doesNotMatch(source, />有范围</);
  assert.doesNotMatch(source, /\$\{assets\.length\} assets/);
  assert.match(source, /筛选目录产品/);
  assert.match(source, /多关键词筛选/);
  assert.match(source, /清空筛选/);
  assert.match(source, /data-catalog-search-pending/);
  assert.match(source, /min-w-\[820px\]/);
  assert.match(source, /row\.kind === "sar"/);
  assert.match(source, /row\.kind === "optical" && "whitespace-nowrap text-center"/);
  assert.match(source, /row\.kind === "optical" && "max-w-none shrink-0 whitespace-nowrap text-center"/);
  assert.match(source, /row\.file_size_label \|\| row\.kind === "optical"/);
  assert.doesNotMatch(source, /row\.kind === "optical" && "min-w-\[(?:5|6\.5)rem\]/);
  assert.doesNotMatch(source, /"expanded"|Maximize2|Minimize2/);
  assert.match(source, /preferences\.heightMode === "collapsed"\s*\? "normal"\s*:\s*"collapsed"/);
});

test("bounded Store workbench exposes batch totals and explicit page controls", () => {
  assert.match(source, /boundedPageMode/);
  assert.match(source, /\n\s+全选\s*\n\s+<\/Button>/);
  assert.match(source, /\n\s+重置\s*\n\s+<\/Button>/);
  assert.doesNotMatch(source, /全选全部|清空全部选择/);
  assert.match(source, /筛选冻结结果/);
  assert.match(source, /aria-label="上一页"/);
  assert.match(source, /aria-label="下一页"/);
  assert.match(source, /aria-label="结果页码"/);
  assert.match(source, /if \(boundedPageMode\) return;/);
});

test("an authoritative zero-hit batch is distinct from an empty local filter", () => {
  assert.match(source, /products\.length === 0/);
  assert.match(source, /未找到符合当前条件的数据/);
  assert.match(source, /没有匹配当前筛选条件的产品；清空筛选可恢复完整结果/);
});

test("provider fallback provenance is read-only and appears only as trace details", () => {
  assert.match(source, /data-source-attempts/);
  assert.match(source, /来源切换/);
  assert.match(source, /本次检索来源轨迹/);
  assert.match(source, /sourceAttempts\.length > 1/);
  assert.doesNotMatch(source, /onSourceChange|onSourceSelect|sourceProviderOptions/);
});

test("DEM is rendered from projections without queue controls", () => {
  assert.match(source, /demTasks\.map/);
  assert.match(source, /task\.remote_source_summary/);
  assert.match(source, /task\.artifacts/);
  assert.match(source, /hasCatalog && hasDem/);
  assert.doesNotMatch(source, /onPauseDem|onResumeDem|onStopDem|onRetryDem/);
  assert.doesNotMatch(source, /SearchBatch/);
});
