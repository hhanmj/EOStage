import { createBrowserFixtureDesktopShellBridge } from "@/lib/desktopShellBridgeFixture";
import type { DesktopShellBridge } from "@/lib/desktopShellBridge";

export function createWorkbenchTestDesktopShellBridge(): DesktopShellBridge {
  const base = createBrowserFixtureDesktopShellBridge();
  return { ...base, kind: "tauri" };
}
