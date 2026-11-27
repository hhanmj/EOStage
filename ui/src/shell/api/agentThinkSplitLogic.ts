export interface SplitInlineThinkResult {
  thinkingText: string;
  answerMarkdown: string;
}

/**
 * Strips and extracts `<think>...</think>` or `<thinking>...</thinking>` blocks
 * (including multiple chunks, orphan closing `</think>` headers, and unclosed `<think>` tails)
 * while preserving `<think>` literals inside fenced code blocks (``` / ~~~) and inline code (`...`).
 *
 * When `raw` contains no think tags, returns `{ thinkingText: "", answerMarkdown: raw }` untouched.
 */
export function splitInlineThink(raw: string): SplitInlineThinkResult {
  if (!raw) {
    return { thinkingText: "", answerMarkdown: "" };
  }
  if (!/<\/?(?:think|thinking)\b/i.test(raw)) {
    return { thinkingText: "", answerMarkdown: raw };
  }

  const thinkingParts: string[] = [];
  const answerParts: string[] = [];

  let i = 0;
  const len = raw.length;
  let atLineStart = true;
  let inFence: { markerChar: "`" | "~"; markerLen: number } | null = null;
  let sawFenceBefore = false;
  let sawOpenThink = false;

  while (i < len) {
    if (inFence) {
      if (atLineStart) {
        let j = i;
        while (j < len && (raw[j] === " " || raw[j] === "\t")) j++;
        let count = 0;
        while (j + count < len && raw[j + count] === inFence.markerChar) count++;
        if (count >= inFence.markerLen) {
          let k = j + count;
          while (k < len && (raw[k] === " " || raw[k] === "\t")) k++;
          if (k >= len || raw[k] === "\n" || raw[k] === "\r") {
            let lineEnd = k;
            if (lineEnd < len && raw[lineEnd] === "\r") lineEnd++;
            if (lineEnd < len && raw[lineEnd] === "\n") lineEnd++;
            answerParts.push(raw.slice(i, lineEnd));
            i = lineEnd;
            inFence = null;
            atLineStart = true;
            continue;
          }
        }
      }
      const nextNl = raw.indexOf("\n", i);
      if (nextNl === -1) {
        answerParts.push(raw.slice(i));
        i = len;
        atLineStart = false;
      } else {
        answerParts.push(raw.slice(i, nextNl + 1));
        i = nextNl + 1;
        atLineStart = true;
      }
      continue;
    }

    // Check for start of fenced code block at line start (up to 3 leading spaces or tabs)
    if (atLineStart) {
      let j = i;
      while (j < len && (raw[j] === " " || raw[j] === "\t")) j++;
      const ch = raw[j];
      if (ch === "`" || ch === "~") {
        let count = 0;
        while (j + count < len && raw[j + count] === ch) count++;
        if (count >= 3) {
          const nextNl = raw.indexOf("\n", j + count);
          const infoStr = raw.slice(j + count, nextNl === -1 ? len : nextNl);
          // Backtick fence info string cannot contain backticks per CommonMark
          if (ch === "~" || !infoStr.includes("`")) {
            inFence = { markerChar: ch, markerLen: count };
            sawFenceBefore = true;
            if (nextNl === -1) {
              answerParts.push(raw.slice(i));
              i = len;
              atLineStart = false;
            } else {
              answerParts.push(raw.slice(i, nextNl + 1));
              i = nextNl + 1;
              atLineStart = true;
            }
            continue;
          }
        }
      }
    }

    // Check inline code span outside fences
    if (raw[i] === "`") {
      let tickCount = 0;
      while (i + tickCount < len && raw[i + tickCount] === "`") tickCount++;
      const ticks = "`".repeat(tickCount);
      const closeIdx = raw.indexOf(ticks, i + tickCount);
      if (closeIdx !== -1) {
        const span = raw.slice(i, closeIdx + tickCount);
        answerParts.push(span);
        i = closeIdx + tickCount;
        atLineStart = span.endsWith("\n");
        continue;
      }
      answerParts.push(ticks);
      i += tickCount;
      atLineStart = false;
      continue;
    }

    // Check <think> / <thinking> or orphan </think> / </thinking>
    if (raw[i] === "<") {
      const rest = raw.slice(i);
      const openMatch = /^<(think|thinking)(?:\s[^>]*)?>/i.exec(rest);
      if (openMatch) {
        sawOpenThink = true;
        const afterOpen = i + openMatch[0].length;
        const closeRe = /<\/(?:think|thinking)\s*>/i;
        const closeMatch = closeRe.exec(raw.slice(afterOpen));
        if (closeMatch) {
          const inner = raw.slice(afterOpen, afterOpen + closeMatch.index).trim();
          if (inner.length > 0) {
            thinkingParts.push(inner);
          }
          let afterClose = afterOpen + closeMatch.index + closeMatch[0].length;
          if (atLineStart) {
            while (afterClose < len && (raw[afterClose] === " " || raw[afterClose] === "\t")) {
              afterClose++;
            }
            if (afterClose < len && raw[afterClose] === "\r") afterClose++;
            if (afterClose < len && raw[afterClose] === "\n") afterClose++;
          }
          i = afterClose;
          atLineStart = i === 0 || raw[i - 1] === "\n";
          continue;
        }
        // Unclosed <think> tail: quarantine remainder into thinkingText so <think> never renders
        const trailingInner = raw.slice(afterOpen).trim();
        if (trailingInner.length > 0) {
          thinkingParts.push(trailingInner);
        }
        i = len;
        break;
      }

      const orphanCloseMatch = /^<\/(?:think|thinking)\s*>/i.exec(rest);
      if (orphanCloseMatch) {
        if (!sawOpenThink && !sawFenceBefore && i <= 8192) {
          const priorText = answerParts.join("").trim();
          if (priorText.length > 0) {
            thinkingParts.push(priorText);
          }
          answerParts.length = 0;
          sawOpenThink = true;
        }
        let afterOrphan = i + orphanCloseMatch[0].length;
        if (answerParts.length === 0) {
          while (
            afterOrphan < len &&
            (raw[afterOrphan] === " " ||
              raw[afterOrphan] === "\t" ||
              raw[afterOrphan] === "\r" ||
              raw[afterOrphan] === "\n")
          ) {
            afterOrphan++;
          }
        }
        i = afterOrphan;
        atLineStart = i === 0 || raw[i - 1] === "\n";
        continue;
      }
    }

    const ch = raw[i];
    answerParts.push(ch);
    atLineStart = ch === "\n";
    i++;
  }

  const joinedAnswer = answerParts.join("");
  if (thinkingParts.length === 0 && joinedAnswer === raw) {
    return { thinkingText: "", answerMarkdown: raw };
  }

  const thinkingText = thinkingParts.join("\n\n").trim();
  const answerMarkdown =
    joinedAnswer.trim().length === 0 ? "" : joinedAnswer.replace(/^\s+/, "").trimEnd();
  return { thinkingText, answerMarkdown };
}

export function tryUnwrapRawConclusionJson(text: string): string {
  const trimmed = text.trim();
  if (!trimmed.startsWith("{") || !trimmed.endsWith("}")) {
    return text;
  }
  try {
    const parsed = JSON.parse(trimmed) as Record<string, unknown>;
    if (
      parsed &&
      typeof parsed === "object" &&
      !Array.isArray(parsed) &&
      typeof parsed.text === "string" &&
      ("claims" in parsed ||
        Object.keys(parsed).every((k) => k === "text" || k === "claims" || k === "summary"))
    ) {
      return parsed.text;
    }
  } catch {
    // Keep original text when not valid JSON
  }
  return text;
}

/**
 * Produces clean copyable Markdown for clipboard copy actions, stripping any `<think>` blocks
 * and unwrapping accidental raw `{"text":...,"claims":[...]}` conclusion JSON payloads.
 */
export function cleanCopyableMarkdown(raw: string): string {
  const { answerMarkdown } = splitInlineThink(raw);
  return tryUnwrapRawConclusionJson(answerMarkdown).trim();
}
