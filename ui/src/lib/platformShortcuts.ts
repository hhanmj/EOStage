/**
 * platformShortcuts.ts
 * 跨平台快捷键标注辅助：
 * - 在 macOS (`navigator.platform` 以 `Mac` 开头，含 Apple Silicon `MacIntel`) 显示 ⌘ / ⌥；
 * - 在 Windows / Linux / JSDOM 默认测试环境下保持 Ctrl+ / Alt+，与已有文案测试兼容。
 */

export function isMacPlatform(): boolean {
  if (typeof navigator === "undefined") return false;
  const nav = navigator as Navigator & { userAgentData?: { platform?: string } };
  const rawPlatform = nav.platform || nav.userAgentData?.platform || "";
  return /^mac/i.test(rawPlatform);
}

export function modShortcut(key: string): string {
  return isMacPlatform() ? `⌘${key}` : `Ctrl+${key}`;
}

export function altShortcut(key: string): string {
  return isMacPlatform() ? `⌥${key}` : `Alt+${key}`;
}

export function ctrlShortcut(key: string): string {
  return isMacPlatform() ? `⌃${key}` : `Ctrl+${key}`;
}

export function historyNavShortcut(direction: "back" | "forward"): string {
  if (isMacPlatform()) {
    return direction === "back" ? "⌘[" : "⌘]";
  }
  return direction === "back" ? "Alt+←" : "Alt+→";
}
