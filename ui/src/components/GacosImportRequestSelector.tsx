import { useEffect } from "react";

import {
  reconcileGacosImportRequestKey,
  type GacosImportRequestChoice,
} from "@/lib/gacosImportRequest";

type GacosImportRequestSelectorProps = {
  choices: readonly GacosImportRequestChoice[];
  selectedKey: string;
  onSelectedKeyChange(nextKey: string): void;
};

/**
 * Keeps request-item selection in Workbench state while projecting the
 * durable GACOS request groups as an explicit import choice.
 */
export function GacosImportRequestSelector({
  choices,
  selectedKey,
  onSelectedKeyChange,
}: GacosImportRequestSelectorProps) {
  useEffect(() => {
    const nextKey = reconcileGacosImportRequestKey(selectedKey, choices);
    if (nextKey !== selectedKey) onSelectedKeyChange(nextKey);
  }, [choices, onSelectedKeyChange, selectedKey]);

  if (choices.length > 1) {
    return (
      <label className="grid gap-1 text-xs">
        <span className="font-medium">结果所属的 GACOS request 项目</span>
        <select
          value={selectedKey}
          onChange={(event) => onSelectedKeyChange(event.target.value)}
          className="flex h-9 w-full rounded-md border border-input bg-card px-3 text-sm shadow-sm focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
        >
          <option value="">请选择 task/item</option>
          {choices.map((choice) => (
            <option key={choice.key} value={choice.key}>{choice.label}</option>
          ))}
        </select>
      </label>
    );
  }
  if (choices.length === 1) {
    return (
      <div className="rounded-md border border-success/30 bg-success/10 px-3 py-2 text-[11px]">
        结果将绑定到 request：<span className="font-medium">{choices[0]!.label}</span>
      </div>
    );
  }
  return (
    <div className="rounded-md border border-warning/35 bg-warning/10 px-3 py-2 text-[11px] text-muted-foreground">
      暂无已验证的 request task/item；请先发送或刷新 GACOS 申请状态后再导入。
    </div>
  );
}
