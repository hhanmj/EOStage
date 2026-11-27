import { ExternalLink, LockKeyhole, X } from "lucide-react";

import { Button } from "@/components/ui/button";
import type { AsfRestrictedAccessPresentation } from "@/lib/asfRestrictedAccess";

export function AsfRestrictedAccessDialog({
  presentation,
  onClose,
  onOpenSupport,
}: {
  presentation: AsfRestrictedAccessPresentation;
  onClose(): void;
  onOpenSupport(url: string): void;
}) {
  return (
    <div className="fixed inset-0 z-[1400] grid place-items-center bg-black/25 p-4" data-asf-restricted-access-backdrop>
      <section
        role="dialog"
        aria-modal="true"
        aria-label={presentation.title}
        className="w-full max-w-lg overflow-hidden rounded-sp-modal border border-sp-line bg-background shadow-sp-modal"
        data-asf-restricted-access-dialog
      >
        <header className="flex items-center justify-between gap-3 border-b bg-muted/30 px-5 py-4">
          <div className="flex min-w-0 items-center gap-2.5">
            <LockKeyhole className="h-5 w-5 shrink-0 text-warning" />
            <h2 className="truncate text-base font-semibold">{presentation.title}</h2>
          </div>
          <Button type="button" size="icon" variant="ghost" aria-label="关闭" onClick={onClose}>
            <X className="h-4 w-4" />
          </Button>
        </header>
        <div className="space-y-4 px-5 py-5">
          <p className="text-sm leading-6 text-foreground">{presentation.message}</p>
          <div className="flex justify-end">
            <Button
              type="button"
              variant="outline"
              onClick={() => onOpenSupport(presentation.supportUrl)}
            >
              <ExternalLink className="h-4 w-4" />
              {presentation.supportLabel}
            </Button>
          </div>
        </div>
      </section>
    </div>
  );
}
