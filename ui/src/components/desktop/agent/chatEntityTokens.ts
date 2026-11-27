/**
 * chatEntityTokens.ts
 * Lossless parser for inline /skill, @dataset, and #AOI tokens plus
 * selected builtin/custom skill prompt extraction for AgentComposer & chat bubbles.
 */

import type { AgentSkill } from "@/lib/agentSkills";

export type ChatEntityTokenKind = "text" | "skill" | "dataset" | "aoi";

export interface ChatEntitySegment {
  kind: ChatEntityTokenKind;
  value: string;
}

const ENTITY_TOKEN_PATTERN =
  /(?:^|(?<=[\s([（【]))((\/[a-zA-Z\u4e00-\u9fa5][a-zA-Z0-9\u4e00-\u9fa5._-]{1,31})|(@[a-zA-Z0-9\u4e00-\u9fa5][a-zA-Z0-9\u4e00-\u9fa5._-]{1,40})|(#[\p{L}\p{N}_-]{1,32}))(?=$|[\s)\]）】.,;:!?，。；：！？])/gu;

/**
 * Losslessly split message text into plain segments and typed entity chips.
 * Always preserves exact concatenation: segments.map((s) => s.value).join("") === text.
 */
export function tokenizeChatEntities(text: string): ChatEntitySegment[] {
  if (!text) return [];
  const segments: ChatEntitySegment[] = [];
  let cursor = 0;

  for (const match of text.matchAll(ENTITY_TOKEN_PATTERN)) {
    const token = match[1];
    const index = match.index ?? 0;
    if (!token) continue;

    if (index > cursor) {
      segments.push({ kind: "text", value: text.slice(cursor, index) });
    }

    const prefix = token[0];
    const kind: ChatEntityTokenKind =
      prefix === "/" ? "skill" : prefix === "@" ? "dataset" : "aoi";
    segments.push({ kind, value: token });
    cursor = index + token.length;
  }

  if (cursor < text.length) {
    segments.push({ kind: "text", value: text.slice(cursor) });
  }

  return segments;
}

/**
 * Extract selected skills whose prompt block appears verbatim in `rawValue`,
 * leaving clean editable user text without mutating external prompt contracts.
 */
export function extractSelectedSkillsFromPrompt(
  rawValue: string,
  skills: readonly AgentSkill[]
): { selectedSkills: AgentSkill[]; cleanText: string } {
  if (!rawValue || skills.length === 0) {
    return { selectedSkills: [], cleanText: rawValue };
  }

  const orderedSkills = [...skills]
    .filter((s) => s.prompt && s.prompt.trim().length >= 8)
    .sort((a, b) => b.prompt.length - a.prompt.length);

  let working = rawValue;
  const found: Array<{ pos: number; skill: AgentSkill }> = [];

  for (const skill of orderedSkills) {
    const idx = working.indexOf(skill.prompt);
    if (idx === -1) continue;
    found.push({ pos: idx, skill });
    const before = working.slice(0, idx);
    const after = working.slice(idx + skill.prompt.length);
    const trimmedBefore = before.replace(/\n{1,2}$/, "");
    const trimmedAfter = after.replace(/^\n{1,2}/, "");
    working =
      trimmedBefore && trimmedAfter
        ? `${trimmedBefore}\n\n${trimmedAfter}`
        : `${trimmedBefore}${trimmedAfter}`;
  }

  found.sort((a, b) => a.pos - b.pos);
  return {
    selectedSkills: found.map((entry) => entry.skill),
    cleanText: working,
  };
}

/**
 * Reconstruct the wire prompt string from user text + selected skill blocks.
 */
export function reconstructPromptWithSkills(
  cleanText: string,
  selectedSkills: readonly AgentSkill[]
): string {
  const promptBlocks = selectedSkills.map((s) => s.prompt).filter(Boolean);
  if (promptBlocks.length === 0) return cleanText;
  if (!cleanText.trim()) return promptBlocks.join("\n\n");
  return `${cleanText}\n\n${promptBlocks.join("\n\n")}`;
}
