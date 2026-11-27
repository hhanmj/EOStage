/**
 * Returns the active runtime localStorage instance, preferring an explicit
 * own-property override on globalThis before falling back to window/globalThis.
 * Callers wrap accesses in try/catch to preserve fail-closed behavior when
 * Web Storage is unavailable or throws.
 */
export function getStorage(): Storage {
  try {
    const desc = Object.getOwnPropertyDescriptor(globalThis, "localStorage");
    if (desc && "value" in desc && desc.value) return desc.value as Storage;
  } catch {
    // Ignore property descriptor errors and fall back to window/globalThis.
  }
  if (typeof window !== "undefined" && window.localStorage) return window.localStorage;
  return globalThis.localStorage;
}
