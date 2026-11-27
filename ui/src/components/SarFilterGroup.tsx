import type { ReactNode } from "react";

import { cn } from "@/lib/utils";

type SarFilterGroupProps = {
  title: string;
  children: ReactNode;
  className?: string;
};

export function SarFilterGroup({ title, children, className }: SarFilterGroupProps) {
  return (
    <section
      data-sar-filter-group={title}
      className="space-y-1.5 border-t border-border/55 py-1.5 first:border-t-0"
    >
      <div className="text-[10px] font-semibold leading-4 text-foreground">{title}</div>
      <div className={cn("grid grid-cols-2 gap-x-2 gap-y-1.5", className)}>{children}</div>
    </section>
  );
}
