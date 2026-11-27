import type { ReactNode } from "react";
import { Loader2 } from "lucide-react";
import { Button, type ButtonProps } from "@/components/ui/button";
import { cn } from "@/lib/utils";

export type CredentialAction = {
  key: string;
  label: string;
  onClick?: () => void;
  icon?: ReactNode;
  busy?: boolean;
  disabled?: boolean;
  type?: "button" | "submit";
  variant?: ButtonProps["variant"];
};

export function CredentialActionBar({
  actions,
  id,
  className,
}: {
  actions: CredentialAction[];
  id: string;
  className?: string;
}) {
  return (
    <div
      className={cn("grid grid-cols-[repeat(auto-fit,minmax(min(100%,6rem),1fr))] gap-1.5", className)}
      data-credential-action-bar={id}
    >
      {actions.map((action) => (
        <Button
          key={action.key}
          type={action.type ?? "button"}
          variant={action.variant ?? "outline"}
          size="sm"
          className="h-8 min-w-0 w-full px-2"
          disabled={action.disabled || action.busy}
          aria-busy={action.busy ?? false}
          onClick={action.onClick}
        >
          {action.busy ? <Loader2 className="h-3.5 w-3.5 animate-spin" /> : action.icon}
          <span className="whitespace-nowrap">{action.label}</span>
        </Button>
      ))}
    </div>
  );
}
