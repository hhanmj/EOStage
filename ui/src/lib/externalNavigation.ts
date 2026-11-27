export function approvedExternalUrl(href: string): string | null {
  try {
    const target = new URL(href);
    if (target.protocol !== "http:" && target.protocol !== "https:") return null;
    return target.href;
  } catch {
    return null;
  }
}

export function desktopExternalUrl(href: string, appHref: string): string | null {
  try {
    const targetHref = new URL(href, appHref).href;
    const approved = approvedExternalUrl(targetHref);
    if (!approved) return null;
    const target = new URL(approved);
    const app = new URL(appHref);
    if (
      (app.protocol === "http:" || app.protocol === "https:")
      && app.origin === target.origin
    ) {
      return null;
    }
    return approved;
  } catch {
    return null;
  }
}
