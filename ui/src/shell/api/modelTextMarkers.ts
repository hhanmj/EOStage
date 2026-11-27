/**
 * 模型正文里的本机脱敏标记 → 中文占位（与模型自己写的「（消息中指定的目录）」同一种写法）。
 * 主机外发前把本机路径、凭据等换成 [local-reference]、[local-reference:message_dir]、[local]、[已省略]，
 * 模型回复会原样带回这些标记。只用于模型正文（在事件分类 classifyStreamEvents 一处替换）；用户自己的消息不替换。
 * 标记两侧原有的空格原样保留，不增不删。
 */

import { t } from "../../i18n/index.ts";

const MARKER = /\[(local-reference(?::([a-z_]+))?|local|已省略|redacted)\]/g; // i18n-exempt: matches host redaction marker token

export function localMarkersToZh(text: unknown): string {
  if (typeof text !== "string") return "";
  if (text.indexOf("[") < 0) return text;
  const replaceProse = (value: string) => value.replace(MARKER, (_match, marker: string, kind: string | undefined) => {
    if (marker === "已省略") return t("shared.modelTextMarkers.omitted"); // i18n-exempt: matches host redaction marker token
    if (marker === "redacted") return t("shared.modelTextMarkers.redacted");
    if (kind === "message_dir") return t("shared.modelTextMarkers.messageDir");
    return t("shared.modelTextMarkers.localContent");
  });

  // Track ordered container prefixes so continuation indentation is relative to
  // every enclosing list, not to the document margin. Unrecognized or incomplete
  // fence containers remain protected until a matching close is unambiguous.
  const lines = text.match(/[^\r\n]*(?:\r\n|\r|\n|$)/g)?.filter(Boolean) ?? [];
  type Container = { kind: "quote" } | { kind: "list"; indent: number };
  let containers: Container[] = [];
  let fence: { char: string; length: number; depth: number } | null = null;
  let inline = 0;
  let output = "";
  for (const line of lines) {
    const body = line.replace(/(?:\r\n|\r|\n)$/, "");
    let offset = 0;
    let depth = 0;
    if (!inline) {
      for (const container of containers) {
        const remaining = body.slice(offset);
        const match = container.kind === "quote"
          ? remaining.match(/^ {0,3}>[ \t]?/)
          : remaining.match(new RegExp(`^ {${container.indent}}`));
        if (!match) break;
        offset += match[0].length;
        depth++;
      }
      if (!fence) {
        containers = containers.slice(0, depth);
        while (true) {
          const remaining = body.slice(offset);
          const quote = remaining.match(/^ {0,3}>[ \t]?/);
          const list = remaining.match(/^ {0,3}(?:[-+*]|\d{1,9}[.)])(?:[ \t]{1,4})(?=[^ \t]|$)/);
          const match = quote ?? list;
          if (!match) break;
          containers.push(quote ? { kind: "quote" } : { kind: "list", indent: match[0].length });
          offset += match[0].length;
          depth++;
        }
      }
    }
    const content = body.slice(offset);
    if (fence) {
      output += line;
      const close = content.match(/^ {0,3}(`{3,}|~{3,})[ \t]*$/);
      if (depth >= fence.depth && close && close[1][0] === fence.char && close[1].length >= fence.length) fence = null;
      continue;
    }
    if (!inline) {
      const open = content.match(/^ {0,3}(`{3,}|~{3,})/);
      if (open) {
        fence = { char: open[1][0], length: open[1].length, depth };
        output += line;
        continue;
      }
      if (/^(?: {4}|\t)/.test(content)) {
        output += line;
        continue;
      }
    }
    output += line.slice(0, offset);
    const remainder = line.slice(offset);
    let pos = 0;
    for (const match of remainder.matchAll(/`+/g)) {
      const index = match.index;
      const segment = remainder.slice(pos, index);
      output += inline ? segment : replaceProse(segment);
      const ticks = match[0];
      output += ticks;
      let slashes = 0;
      for (let j = index - 1; j >= 0 && remainder[j] === "\\"; j--) slashes++;
      if (inline === 0 && slashes % 2 === 0) inline = ticks.length;
      else if (inline !== 0 && inline === ticks.length) inline = 0;
      pos = index + ticks.length;
    }
    const rest = remainder.slice(pos);
    output += inline ? rest : replaceProse(rest);
  }
  return output;
}
