import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";

const panelSource = readFileSync(
  new URL("../components/MapPreviewPanel.tsx", import.meta.url),
  "utf8",
);
const mapSource = readFileSync(
  new URL("../components/WorkbenchMap.tsx", import.meta.url),
  "utf8",
);

test("product Preview viewer is embedded in the bottom dock instead of a separate tab", () => {
  assert.match(panelSource, /data-map-preview-panel/);
  assert.match(panelSource, /preview\.status === "available"/);
  assert.match(panelSource, /preview\.status === "loading"/);
  assert.match(panelSource, /onClose/);
  assert.match(panelSource, /onCancel/);
  assert.doesNotMatch(panelSource, /onOpenOriginal|打开原图|ExternalLink/);
  assert.match(panelSource, /onSave/);
  assert.match(panelSource, /savedPath/);
  assert.match(panelSource, /onOpenSaveDirectory/);
  assert.match(panelSource, /保存到/);
  assert.match(panelSource, /aria-label="适合窗口"/);
  assert.match(panelSource, /aria-label="原始尺寸"/);
  assert.match(panelSource, /aria-label="放大预览"/);
  assert.match(panelSource, /aria-label="缩小预览"/);
  assert.match(panelSource, /<img/);
  assert.doesNotMatch(panelSource, /role="tab"|role="tablist"/);
  assert.doesNotMatch(panelSource, /非地理配准/);
  assert.doesNotMatch(panelSource, /transition-all|scale-\[|translate-/);
});

test("desktop-controlled Preview data URLs expose only the bounded save action", () => {
  assert.match(panelSource, /const publicActionAllowed = available && isPreviewActionAllowed\(preview\.url\)/);
  assert.match(panelSource, /isDesktopControlledPreviewDataUrl/);
  assert.match(panelSource, /桌面受控快视图/);
  assert.match(panelSource, /const saveTitle = publicActionAllowed/);
  assert.doesNotMatch(panelSource, /打开原图|onOpenOriginal|actionTitle/);
  assert.doesNotMatch(panelSource, /title=\{preview\.url \|\| undefined\}/);
});

test("WorkbenchMap projects only reviewed geographic Preview below vector layers", () => {
  assert.match(mapSource, /ImageOverlay/);
  assert.match(mapSource, /preview\?: PreviewRuntimeState/);
  assert.match(mapSource, /previewBbox/);
  assert.match(mapSource, /MAP_TILE_Z_INDEX\.preview/);
  assert.match(mapSource, /activePreviewProductKey/);
  assert.doesNotMatch(mapSource, /<MapPreviewPanel/);
  assert.doesNotMatch(mapSource, /onClosePreview/);
  assert.doesNotMatch(mapSource, /setSelectedDownloadSceneIds/);
});

test("map footprint activation uses canonical product identity and hover-only details", () => {
  assert.match(mapSource, /import \{ sceneProductKey \} from "@\/lib\/sarProduct"/);
  assert.match(
    mapSource,
    /function sceneIdentity\(scene: SceneRow\): string \{\s*return sceneProductKey\(scene\);\s*\}/,
  );
  assert.match(mapSource, /BoundedSceneTooltip/);
  assert.match(mapSource, /constrainMapTooltipPosition/);
  assert.doesNotMatch(mapSource, /<Tooltip/);
  assert.match(mapSource, /<ScenePopup scene=\{state\.scene\} index=\{state\.index\}/);
  assert.doesNotMatch(mapSource, /\bPopup\b/);
  assert.doesNotMatch(mapSource, /scenePopup|setScenePopup/);
});
