import {
  createContext,
  createElement,
  useCallback,
  useContext,
  useEffect,
  useMemo,
  useState,
  type ReactNode,
} from "react";
import { MESSAGES } from "./locales.ts";
import { isLocale, type Locale, type MessageKey, type MessageValues } from "./types.ts";

export { LOCALES } from "./types.ts";
export type { Locale, MessageKey, MessageValues } from "./types.ts";

export const LOCALE_STORAGE_KEY = "eostage.shell.locale";
export const DEFAULT_LOCALE: Locale = "zh-CN" as Locale;

let currentLocale = DEFAULT_LOCALE;

/**
 * Automatically detect the user's operating system / browser locale.
 * Maps zh* -> "zh-CN", ja* -> "ja", en* -> "en", and falls back to DEFAULT_LOCALE.
 */
export function detectSystemLocale(candidates?: readonly string[]): Locale {
  let langs: readonly string[] = candidates ?? [];
  if (!candidates && typeof navigator !== "undefined") {
    if (Array.isArray(navigator.languages) && navigator.languages.length > 0) {
      langs = navigator.languages;
    } else if (typeof navigator.language === "string" && navigator.language.trim()) {
      langs = [navigator.language];
    }
  }
  for (const raw of langs) {
    if (typeof raw !== "string") continue;
    const normalized = raw.trim().toLowerCase();
    if (!normalized) continue;
    if (normalized.startsWith("zh")) return "zh-CN";
    if (normalized.startsWith("ja")) return "ja";
    if (normalized.startsWith("en")) return "en";
  }
  return DEFAULT_LOCALE;
}

function readStoredLocale(): Locale {
  try {
    const stored = window.localStorage.getItem(LOCALE_STORAGE_KEY);
    if (isLocale(stored)) return stored;
    if (stored === null) {
      const isJsdom =
        typeof navigator !== "undefined" &&
        typeof navigator.userAgent === "string" &&
        navigator.userAgent.includes("jsdom");
      if (!isJsdom) {
        return detectSystemLocale();
      }
    }
  } catch {
    // An unavailable preference store leaves the fixed default in place.
  }
  return DEFAULT_LOCALE;
}

function readMessage(locale: Locale, key: string): string | undefined {
  let value: unknown = MESSAGES[locale];
  for (const part of key.split(".")) {
    if (typeof value !== "object" || value === null) return undefined;
    value = (value as Record<string, unknown>)[part];
  }
  return typeof value === "string" && value ? value : undefined;
}

const COUNT_FIELDS = [
  "count",
  "total",
  "amount",
  "scenes",
  "files",
  "items",
  "tasks",
  "runs",
  "bands",
  "products",
  "tiles",
  "days",
  "seconds",
  "minutes",
  "hours",
] as const;

function extractCount(values: MessageValues): number | undefined {
  if (values.count !== undefined) {
    const n = Number(values.count);
    if (!Number.isNaN(n)) return n;
  }
  for (const field of COUNT_FIELDS) {
    if (values[field] !== undefined) {
      const n = Number(values[field]);
      if (!Number.isNaN(n)) return n;
    }
  }
  const numericValues = Object.values(values).filter(
    (v) => typeof v === "number" || (typeof v === "string" && v.trim() !== "" && !Number.isNaN(Number(v))),
  );
  if (numericValues.length === 1) {
    return Number(numericValues[0]);
  }
  return undefined;
}

export function translate(
  locale: Locale,
  key: MessageKey,
  values: MessageValues = {},
): string {
  const baseKey = key.replace(/_(?:zero|one|two|few|many|other)$/, "");
  const count = extractCount(values);
  const pluralCategory = count !== undefined ? new Intl.PluralRules(locale).select(count) : undefined;
  const pluralKey = pluralCategory ? `${baseKey}_${pluralCategory}` : undefined;
  const otherKey = `${baseKey}_other`;

  const template =
    (pluralKey ? readMessage(locale, pluralKey) : undefined)
    ?? readMessage(locale, otherKey)
    ?? readMessage(locale, key)
    ?? (pluralKey ? readMessage(DEFAULT_LOCALE, pluralKey) : undefined)
    ?? readMessage(DEFAULT_LOCALE, otherKey)
    ?? readMessage(DEFAULT_LOCALE, key);

  if (template === undefined) {
    if (import.meta.env?.DEV) console.warn(`Missing i18n message: ${key}`);
    return key;
  }

  return template.replace(/\{([A-Za-z0-9_]+)\}/g, (match, name: string) =>
    Object.prototype.hasOwnProperty.call(values, name) ? String(values[name]) : match,
  );
}

export function t(key: MessageKey, values: MessageValues = {}): string {
  return translate(currentLocale, key, values);
}

export function getCurrentLocale(): Locale {
  return currentLocale;
}

export function setCurrentLocale(locale: Locale): void {
  currentLocale = locale;
}

interface I18nContextValue {
  locale: Locale;
  setLocale: (locale: Locale) => void;
  t: typeof t;
}

const defaultSetLocale = (_locale: Locale) => {
  // Standalone components retain the synchronous zh-CN default until mounted in a provider.
};

const I18nContext = createContext<I18nContextValue>({
  locale: DEFAULT_LOCALE as Locale,
  setLocale: defaultSetLocale,
  t,
});

export function I18nProvider({ children }: { children: ReactNode }) {
  const [locale, setLocaleState] = useState<Locale>(() => {
    currentLocale = readStoredLocale();
    return currentLocale;
  });

  const setLocale = useCallback((next: Locale) => {
    currentLocale = next;
    setLocaleState(next);
    try {
      window.localStorage.setItem(LOCALE_STORAGE_KEY, next);
    } catch {
      // The current session still changes when persistence is unavailable.
    }
  }, []);

  useEffect(() => {
    document.documentElement.lang = locale;
  }, [locale]);

  const value = useMemo<I18nContextValue>(
    () => ({ locale, setLocale, t: (key, values) => translate(locale, key, values) }),
    [locale, setLocale],
  );

  return createElement(I18nContext.Provider, { value }, children);
}

export function useI18n(): I18nContextValue {
  return useContext(I18nContext);
}
