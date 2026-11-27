import React from "react";
import { Info } from "lucide-react";
import { reasoningOffRefusedNote } from "../../api/agentChannelsLogic.ts";

export interface ChannelReasoningNoteProps {
  refused?: boolean;
}

export function ChannelReasoningNote({ refused }: ChannelReasoningNoteProps) {
  // 属性驱动提示组件；在后端暴露 reasoning_off_refused 字段前暂不接线
  const note = reasoningOffRefusedNote(refused);
  if (!note) {
    return null;
  }

  return (
    <div className="flex items-center gap-1 text-xs text-sp-text-muted">
      <Info className="h-3 w-3 shrink-0" aria-hidden="true" />
      <span>{note}</span>
    </div>
  );
}
