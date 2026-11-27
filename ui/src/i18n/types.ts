import type { zhCN } from "./locales/zh-CN/index.ts";

export const LOCALES = ["zh-CN", "en", "ja"] as const;
export type Locale = (typeof LOCALES)[number];

type LocalizedNode<Value> = Value extends string
  ? string
  : Value extends object
  ? LocalizedMessages<Value>
  : never;

type PluralOneKey<K> = K extends `${infer Base}_other` ? `${Base}_one` : never;

type LocalizedMessages<Value> = {
  [Key in keyof Value]: LocalizedNode<Value[Key]>;
} & {
  [Key in keyof Value as PluralOneKey<Key>]?: string;
};

export type Messages = LocalizedMessages<typeof zhCN>;

type Join<Path extends string, Key extends string> = Path extends "" ? Key : `${Path}.${Key}`;
type LeafPaths<Value, Path extends string = ""> = {
  [Key in Extract<keyof Value, string>]: Value[Key] extends object
    ? LeafPaths<Value[Key], Join<Path, Key>>
    : Join<Path, Key>;
}[Extract<keyof Value, string>];

type ExpandPluralKeys<P extends string> = P extends `${infer Base}_other`
  ? Base | P | `${Base}_one`
  : P;

export type MessageKey = ExpandPluralKeys<LeafPaths<typeof zhCN>>;
export type MessageValues = Record<string, string | number>;

export function isLocale(value: string | null): value is Locale {
  return value !== null && (LOCALES as readonly string[]).includes(value);
}
