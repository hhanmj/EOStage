import { reacquireConfirmationText } from "@/lib/localReusePresentation";
import { Popconfirm } from "@/components/ui/Popconfirm";
import type { ReactNode } from "react";

/** Downloader D3: confirms 重新下载 of a reused item via anchored Popconfirm; the original file is kept. */
export function ReacquireConfirmation({ displayName, anchorEl, busy, error, children, onCancel, onConfirm }: {
  displayName: string;
  anchorEl?: HTMLElement | null;
  busy: boolean;
  error: string | null;
  children?: ReactNode;
  onCancel: () => void;
  onConfirm: () => void;
}) {
  return (
    <Popconfirm
      open
      anchorEl={anchorEl}
      role="dialog"
      ariaLabel="重新下载"
      title={`重新下载 ${displayName}？`}
      description={reacquireConfirmationText(displayName)}
      error={error}
      extra={children}
      confirmLabel={busy ? "正在提交…" : "重新下载"}
      cancelLabel="取消"
      tone="primary"
      busy={busy}
      onCancel={onCancel}
      onConfirm={onConfirm}
    />
  );
}
