/**
 * ChatTokenizedText.tsx
 * Renders user prompt / composer entities (/skill, @dataset, #AOI) as compact chips
 * while preserving exact readable text semantics.
 */

import React from "react";
import { useI18n } from "@/i18n";
import { getAgentSkills, type AgentSkill } from "@/lib/agentSkills";
import { localizeAgentSkillForDisplay } from "@/lib/pluginsLocale";
import { cn } from "@/lib/utils";
import {
  extractSelectedSkillsFromPrompt,
  tokenizeChatEntities,
} from "./chatEntityTokens";

export interface ChatTokenizedTextProps {
  text: string;
  skills?: readonly AgentSkill[];
  className?: string;
}

export function ChatTokenizedText({
  text,
  skills,
  className,
}: ChatTokenizedTextProps) {
  const { locale } = useI18n();
  const availableSkills = (skills ?? getAgentSkills()).map((s) =>
    localizeAgentSkillForDisplay(s, locale)
  );
  const { selectedSkills, cleanText } = extractSelectedSkillsFromPrompt(
    text,
    availableSkills
  );
  const segments = tokenizeChatEntities(cleanText);

  return (
    <span className={cn("whitespace-pre-wrap break-words", className)}>
      {selectedSkills.length > 0 && (
        <span className="inline-flex flex-wrap items-center gap-1 mr-1.5 align-middle">
          {selectedSkills.map((skill) => (
            <span
              key={skill.id}
              data-token-kind="skill"
              className="sp-chat-token-chip sp-chat-token-chip--skill"
            >
              {skill.name}
            </span>
          ))}
        </span>
      )}
      {segments.map((seg, idx) =>
        seg.kind === "text" ? (
          <React.Fragment key={idx}>{seg.value}</React.Fragment>
        ) : (
          <span
            key={idx}
            data-token-kind={seg.kind}
            className={cn(
              "sp-chat-token-chip",
              seg.kind === "skill" && "sp-chat-token-chip--skill",
              seg.kind === "dataset" && "sp-chat-token-chip--dataset",
              seg.kind === "aoi" && "sp-chat-token-chip--aoi"
            )}
          >
            {seg.value}
          </span>
        )
      )}
    </span>
  );
}
