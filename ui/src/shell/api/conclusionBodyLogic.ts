import { splitInlineThink, tryUnwrapRawConclusionJson } from "./agentThinkSplitLogic.ts";

export function splitConclusionJsonBlock(text: string): {
  body: string;
  jsonBlocks: string[];
} {
  const { answerMarkdown } = splitInlineThink(text);
  const unwrapped = tryUnwrapRawConclusionJson(answerMarkdown);

  const jsonBlocks: string[] = [];
  const fenceRe = /^[ \t]*```json[ \t]*\r?\n(?:[\s\S]*?\r?\n)?[ \t]*```[ \t]*$/gim;
  let match: RegExpExecArray | null;
  while ((match = fenceRe.exec(unwrapped)) !== null) {
    jsonBlocks.push(match[0].trim());
  }
  if (jsonBlocks.length === 0) {
    return { body: unwrapped, jsonBlocks: [] };
  }
  const body = unwrapped.replace(fenceRe, "").trim();
  return { body, jsonBlocks };
}
