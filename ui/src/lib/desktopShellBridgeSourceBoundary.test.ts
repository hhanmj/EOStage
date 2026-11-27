import assert from "node:assert/strict";
import { readFileSync, readdirSync } from "node:fs";
import path from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";

const uiRoot = fileURLToPath(new URL("../../", import.meta.url));
const libRoot = path.join(uiRoot, "src", "lib");

function source(relativePath: string): string {
  return readFileSync(path.join(uiRoot, relativePath), "utf8");
}

test("desktop shell modules expose no generic native or business command surface", () => {
  const moduleNames = [
    "desktopShellBridge.ts",
    "desktopShellBridgePywebview.ts",
    "desktopShellBridgeFixture.ts",
  ];
  const combined = moduleNames
    .map((name) => readFileSync(path.join(libRoot, name), "utf8"))
    .join("\n");

  assert.doesNotMatch(combined, /@tauri-apps\//);
  assert.doesNotMatch(combined, /\binvoke\s*\(/);
  // `credential:` is a typed capability input, not a native/business command surface.
  // Keep the boundary assertion focused on executable adapter fields.
  assert.doesNotMatch(combined, /\b(shell|sql|http|filesystem|provider|download)\s*:/i);
  assert.doesNotMatch(combined, /\b(products|selectedProductKeys|taskSnapshot|queue|demTasks)\s*:/);
});

test("migrated bridge exports delegate while business bridge detection stays behind the single host", () => {
  const bridge = source("src/lib/bridge.ts");

  assert.match(bridge, /export function hasBridge\(\): boolean \{[\s\S]*getDesktopShellBridge\(\)\.kind === "tauri"/);
  assert.match(bridge, /getDesktopShellBridge\(\)\.app\.getInfo\(\)/);
  assert.match(bridge, /getDesktopShellBridge\(\)\.dialog\.pickOpenFile\(title, filters\)/);
  assert.match(bridge, /getDesktopShellBridge\(\)\.dialog\.pickDirectory\(title\)/);
  assert.match(bridge, /getDesktopShellBridge\(\)\.window\.minimize\(\)/);
  assert.match(bridge, /getDesktopShellBridge\(\)\.window\.toggleMaximize\(\)/);
  assert.match(bridge, /getDesktopShellBridge\(\)\.window\.close\(\)/);
  assert.match(bridge, /getDesktopShellBridge\(\)\.window\.getSize\(\)/);
  assert.match(
    bridge,
    /getDesktopShellBridge\(\)\.window\.resizeFromEdge\([\s\S]*edge,[\s\S]*startWidth/,
  );
  assert.match(bridge, /isTauriDesktopHost/);
  assert.match(bridge, /compatibility\.callLegacy/);
  assert.doesNotMatch(bridge, /window\.pywebview|pywebviewready/);
  assert.doesNotMatch(bridge, /createPywebviewDesktopShellBridge/);
  assert.doesNotMatch(bridge, /route\(\{ \.\.\.settings, tianditu_token: "" \}\)/);
  assert.doesNotMatch(
    bridge,
    /return \{ ok: true, \.\.\.(?:settings|saved), tianditu_token: "" \}/,
  );
});

test("single-host application bridge uses named commands without a generic business escape hatch", () => {
  const bridge = source("src/lib/bridge.ts");
  const desktopBridge = source("src/lib/desktopShellBridge.ts");
  const tauri = source("src/lib/desktopShellBridgeTauri.ts");

  for (const [exportName, delegate] of [
    ["getApplicationProviderCapabilities", "providerCapabilities"],
    ["applicationSarSearchAndFreeze", "sarSearchAndFreeze"],
    ["applicationCatalogSearchAndFreeze", "catalogSearchAndFreeze"],
    ["getCdseSessionStatus", "cdseSessionStatus"],
    ["loginCdseSession", "cdseLogin"],
    ["logoutCdseSession", "cdseLogout"],
    ["getCdseNetworkPolicy", "cdseNetworkPolicy"],
    ["setCdseNetworkPolicy", "setCdseNetworkPolicy"],
    ["runCdseNetworkDiagnostic", "cdseNetworkDiagnostic"],
    ["getPreviewAsset", "previewAsset"],
    ["savePublicPreview", "savePreview"],
    ["prepareNisarSubsetIntentV2", "prepareNisarSubsetIntentV2"],
    ["confirmNisarSubsetTaskV2", "confirmNisarSubsetTaskV2"],
    ["getCdseQuicklook", "cdseQuicklook"],
    ["cancelApplicationRequest", "cancelRequest"],
    ["updateApplicationSearchWorkbench", "updateSearchWorkbench"],
    ["createApplicationDownloadTask", "createDownloadTask"],
    ["updateApplicationDownloadTask", "updateDownloadTask"],
    ["archiveApplicationDownloadTask", "archiveDownloadTask"],
    ["deleteApplicationDownloadHistory", "deleteDownloadHistory"],
  ]) {
    assert.match(bridge, new RegExp(`export async function ${exportName}[\\s\\S]*${delegate}`));
  }

  for (const command of [
    "application_provider_capabilities",
    "application_sar_search_and_freeze",
    "application_sar_background_count",
    "application_catalog_search_and_freeze",
    "application_cancel_request",
    "application_cdse_session_status",
    "application_cdse_login",
    "application_cdse_logout",
    "application_cdse_network_policy",
    "application_set_cdse_network_policy",
    "application_cdse_network_diagnostic",
    "application_preview_asset",
    "application_save_preview_asset",
    "application_prepare_nisar_subset_intent_v2",
    "application_confirm_nisar_subset_task_v2",
    "application_cdse_quicklook",
    "application_update_search_workbench",
    "application_create_download_task",
    "application_update_download_task",
    "application_archive_download_task",
    "application_delete_download_history",
    "application_resolve_dem_conversion_source",
  ]) {
    assert.match(tauri, new RegExp(`"${command}"`));
  }
  assert.match(
    desktopBridge,
    /import type \{[\s\S]*?CatalogSearchQuery,[\s\S]*?\} from "\.\/generated\/v3-contracts\.ts"/,
  );
  assert.match(
    desktopBridge,
    /export type DesktopCatalogSearchAndFreezeRequest = \{[\s\S]*search: CatalogSearchQuery;/,
  );
  assert.doesNotMatch(tauri, /application_dispatch|SidecarRequest|TaskStore|DownloadScheduler/);
  assert.doesNotMatch(tauri, /application_service_restore|application_download_snapshot|downloadSnapshot/);
  assert.doesNotMatch(desktopBridge, /restore\(includeTerminal|downloadSnapshot/);
});

test("normal renderer components never import Tauri adapters directly", () => {
  const componentRoot = path.join(uiRoot, "src", "components");
  const componentSources = readdirSync(componentRoot, { recursive: true })
    .filter((name) => name.endsWith(".tsx") && !name.endsWith(".test.tsx"))
    .map((name) => readFileSync(path.join(componentRoot, name), "utf8"));
  const normalEntry = [
    source("src/main.tsx"),
    source("src/App.tsx"),
    ...componentSources,
  ].join("\n");

  assert.doesNotMatch(normalEntry, /@tauri-apps\//);
  assert.doesNotMatch(normalEntry, /desktopShellBridge(?:Tauri|Pywebview|Fixture)/);
  assert.doesNotMatch(normalEntry, /tauriFoundation/i);
  assert.doesNotMatch(componentSources.join("\n"), /applicationCatalogSearchAndFreeze|catalogSearchAndFreeze/);
});
