import { Loader2, MapPinned, Search } from "lucide-react";

import { Button } from "@/components/ui/button";

export function AdminBoundarySearchButton({
  busy,
  onSearch,
}: {
  busy: boolean;
  onSearch: () => void;
}) {
  return (
    <Button
      type="button"
      size="icon"
      onClick={onSearch}
      disabled={busy}
      aria-label="搜索行政边界候选"
      aria-busy={busy}
      data-state={busy ? "searching" : "idle"}
    >
      {busy ? <Loader2 className="h-4 w-4 animate-spin" /> : <Search className="h-4 w-4" />}
    </Button>
  );
}

export function AdminBoundaryBindButton({
  busy,
  onBind,
}: {
  busy: boolean;
  onBind: () => void;
}) {
  return (
    <Button
      type="button"
      onClick={onBind}
      disabled={busy}
      aria-busy={busy}
      data-state={busy ? "binding" : "idle"}
      className="min-w-0"
    >
      {busy ? <Loader2 className="h-4 w-4 animate-spin" /> : <MapPinned className="h-4 w-4" />}
      加载并绑定边界
    </Button>
  );
}
