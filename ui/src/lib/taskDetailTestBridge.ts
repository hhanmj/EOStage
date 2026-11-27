import {
  clearTestDesktopShellBridge,
  registerTestDesktopShellBridge,
} from "./desktopShellBridge.ts";
import {
  createTauriDesktopShellBridge,
  type TauriBridgeDependencies,
} from "./desktopShellBridgeTauri.ts";

export type TaskDetailTestBridgeDependencies = TauriBridgeDependencies;

export function registerTaskDetailTestBridge(dependencies: TauriBridgeDependencies): void {
  registerTestDesktopShellBridge(
    createTauriDesktopShellBridge(dependencies, "tauri://localhost/", false),
  );
}

export function clearTaskDetailTestBridge(): void {
  clearTestDesktopShellBridge();
}
