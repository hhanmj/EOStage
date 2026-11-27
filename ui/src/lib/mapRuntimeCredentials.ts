const BUILT_IN_TIANDITU_APPLICATION_TOKEN = "5e1557a5184dde180d7bf5c8cc4ca8e8";

/** Runtime-only resolution: persisted settings remain an optional user override. */
export function effectiveTiandituToken(userOverride: string | null | undefined): string {
  return userOverride?.trim() || BUILT_IN_TIANDITU_APPLICATION_TOKEN;
}

export function hasEffectiveTiandituToken(userOverride: string | null | undefined): boolean {
  return effectiveTiandituToken(userOverride).length > 0;
}
