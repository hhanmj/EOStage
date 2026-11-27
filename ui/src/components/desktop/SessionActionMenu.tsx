import React, { useCallback, useEffect, useLayoutEffect, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { ChevronRight, MoreHorizontal } from "lucide-react";
import { cn } from "@/lib/utils";
import type { MenuNode } from "../../shell/api/sessionOrganize.ts";

const MENU_GAP = 4;
const VIEWPORT_PAD = 8;

function placeMenu(
  anchor: DOMRect,
  size: { width: number; height: number },
  mode: "below" | "side"
): { left: number; top: number } {
  const vw = window.innerWidth || 1024;
  const vh = window.innerHeight || 768;
  let left: number;
  let top: number;
  if (mode === "side") {
    left = anchor.right + MENU_GAP;
    if (left + size.width > vw - VIEWPORT_PAD) left = anchor.left - MENU_GAP - size.width;
    top = anchor.top - 4;
  } else {
    left = anchor.left;
    top = anchor.bottom + MENU_GAP;
    if (top + size.height > vh - VIEWPORT_PAD) top = anchor.top - MENU_GAP - size.height;
  }
  left = Math.max(VIEWPORT_PAD, Math.min(left, vw - VIEWPORT_PAD - size.width));
  top = Math.max(VIEWPORT_PAD, Math.min(top, vh - VIEWPORT_PAD - size.height));
  return { left, top };
}

interface MenuPanelProps<A> {
  items: MenuNode<A>[];
  anchor: DOMRect;
  mode: "below" | "side";
  label: string;
  onSelect: (action: A) => void;
  /** Esc / 点击外部：关闭整棵菜单 */
  onDismiss: () => void;
  /** ArrowLeft：只关闭当前子菜单 */
  onBack?: () => void;
  panelRef?: React.RefObject<HTMLDivElement | null>;
}

function MenuPanel<A>({ items, anchor, mode, label, onSelect, onDismiss, onBack, panelRef }: MenuPanelProps<A>) {
  const localRef = useRef<HTMLDivElement | null>(null);
  const ref = panelRef ?? localRef;
  const [pos, setPos] = useState<{ left: number; top: number }>({ left: anchor.left, top: anchor.bottom + MENU_GAP });
  const [submenu, setSubmenu] = useState<{ id: string; rect: DOMRect; trigger: HTMLElement } | null>(null);
  const subRef = useRef<HTMLDivElement | null>(null);

  useLayoutEffect(() => {
    const el = ref.current;
    if (!el) return;
    const rect = el.getBoundingClientRect();
    setPos(placeMenu(anchor, { width: rect.width || 184, height: rect.height || items.length * 34 }, mode));
  }, [anchor, mode, items.length, ref]);

  useEffect(() => {
    const first = ref.current?.querySelector<HTMLElement>('[role="menuitem"]');
    first?.focus();
  }, [ref]);

  const menuItems = () =>
    Array.from(ref.current?.querySelectorAll<HTMLElement>(':scope > [role="menuitem"]') ?? []);

  const openSub = (id: string, target: HTMLElement) => {
    setSubmenu({ id, rect: target.getBoundingClientRect(), trigger: target });
  };

  const handleKeyDown = (e: React.KeyboardEvent<HTMLDivElement>) => {
    const list = menuItems();
    const idx = list.indexOf(document.activeElement as HTMLElement);
    if (e.key === "ArrowDown") {
      e.preventDefault();
      list[(idx + 1 + list.length) % list.length]?.focus();
    } else if (e.key === "ArrowUp") {
      e.preventDefault();
      list[(idx - 1 + list.length) % list.length]?.focus();
    } else if (e.key === "Home") {
      e.preventDefault();
      list[0]?.focus();
    } else if (e.key === "End") {
      e.preventDefault();
      list[list.length - 1]?.focus();
    } else if (e.key === "Escape") {
      e.preventDefault();
      e.stopPropagation();
      onDismiss();
    } else if (e.key === "ArrowLeft" && onBack) {
      e.preventDefault();
      e.stopPropagation();
      onBack();
    } else if (e.key === "Tab") {
      e.preventDefault();
      onDismiss();
    }
  };

  const activeSub = submenu ? items.find((i) => i.id === submenu.id) : null;

  return (
    <>
      <div
        ref={ref}
        role="menu"
        aria-label={label}
        onKeyDown={handleKeyDown}
        style={{ left: pos.left, top: pos.top }}
        className="fixed z-[1200] min-w-[176px] max-w-[280px] rounded-card bg-sp-surface p-1.5 shadow-overlay border border-sp-line dark:bg-sp-card"
      >
        {items.map((item) => {
          const hasSub = Boolean(item.items && item.items.length > 0);
          return (
            <button
              key={item.id}
              type="button"
              role="menuitem"
              tabIndex={-1}
              aria-haspopup={hasSub ? "menu" : undefined}
              aria-expanded={hasSub ? submenu?.id === item.id : undefined}
              onClick={(e) => {
                if (hasSub) {
                  openSub(item.id, e.currentTarget);
                } else if (item.action !== undefined) {
                  onSelect(item.action);
                }
              }}
              onMouseEnter={(e) => {
                if (hasSub) openSub(item.id, e.currentTarget);
                else if (submenu) setSubmenu(null);
              }}
              onKeyDown={(e) => {
                if (hasSub && (e.key === "ArrowRight" || e.key === "Enter" || e.key === " ")) {
                  e.preventDefault();
                  e.stopPropagation();
                  openSub(item.id, e.currentTarget);
                }
              }}
              className={cn(
                "flex h-8 w-full items-center gap-2 rounded-sp-sm px-2.5 text-left text-xs outline-none transition-colors duration-fast ease-sp cursor-pointer focus-visible:ring-1 focus-visible:ring-sp-accent",
                item.danger
                  ? "text-sp-danger-text hover:bg-sp-danger-bg focus-visible:bg-sp-danger-bg"
                  : "text-sp-text hover:bg-sp-hover focus-visible:bg-sp-hover",
                hasSub && submenu?.id === item.id && "bg-sp-hover"
              )}
            >
              <span className="min-w-0 flex-1 truncate" title={item.label}>
                {item.label}
              </span>
              {hasSub && <ChevronRight className="h-3.5 w-3.5 shrink-0 text-sp-text-muted" aria-hidden="true" />}
            </button>
          );
        })}
      </div>
      {activeSub && activeSub.items && submenu && (
        <MenuPanel
          items={activeSub.items}
          anchor={submenu.rect}
          mode="side"
          label={activeSub.label}
          onSelect={onSelect}
          onDismiss={onDismiss}
          onBack={() => {
            const trigger = submenu.trigger;
            setSubmenu(null);
            trigger.focus();
          }}
          panelRef={subRef}
        />
      )}
    </>
  );
}

export interface SessionActionMenuProps<A> {
  items: MenuNode<A>[];
  /** 触发按钮无障碍名称，如「会话操作」 */
  label: string;
  onSelect: (action: A) => void;
  disabled?: boolean;
  /** 父行在悬停/聚焦时才显示按钮；菜单打开时保持可见 */
  revealClassName?: string;
  className?: string;
  onOpenChange?: (open: boolean) => void;
}

/** 行内「⋯」菜单：portal 到 body，不受侧栏 overflow 裁剪；Esc 关闭并把焦点还给触发按钮。 */
export function SessionActionMenu<A>({
  items,
  label,
  onSelect,
  disabled = false,
  revealClassName = "opacity-0 group-hover:opacity-100 group-focus-within:opacity-100 focus-visible:opacity-100",
  className,
  onOpenChange,
}: SessionActionMenuProps<A>) {
  const triggerRef = useRef<HTMLButtonElement | null>(null);
  const panelRef = useRef<HTMLDivElement | null>(null);
  const [anchor, setAnchor] = useState<DOMRect | null>(null);
  const open = anchor !== null;

  const setOpen = useCallback(
    (next: DOMRect | null) => {
      setAnchor(next);
      onOpenChange?.(next !== null);
    },
    [onOpenChange]
  );

  const close = useCallback(
    (restoreFocus: boolean) => {
      setOpen(null);
      if (restoreFocus) triggerRef.current?.focus();
    },
    [setOpen]
  );

  useEffect(() => {
    if (!open) return;
    const onPointerDown = (e: MouseEvent) => {
      const target = e.target as Node | null;
      if (!target) return;
      if (triggerRef.current?.contains(target)) return;
      const menus = document.querySelectorAll('[role="menu"]');
      for (const menu of Array.from(menus)) {
        if (menu.contains(target)) return;
      }
      close(false);
    };
    const onResize = () => close(false);
    document.addEventListener("mousedown", onPointerDown);
    window.addEventListener("resize", onResize);
    return () => {
      document.removeEventListener("mousedown", onPointerDown);
      window.removeEventListener("resize", onResize);
    };
  }, [open, close]);

  return (
    <>
      <button
        ref={triggerRef}
        type="button"
        aria-label={label}
        title={label}
        aria-haspopup="menu"
        aria-expanded={open}
        disabled={disabled}
        onClick={(e) => {
          e.stopPropagation();
          if (open) close(false);
          else setOpen(e.currentTarget.getBoundingClientRect());
        }}
        onKeyDown={(e) => {
          e.stopPropagation();
          if (e.key === "ArrowDown" && !open) {
            e.preventDefault();
            setOpen(e.currentTarget.getBoundingClientRect());
          }
        }}
        className={cn(
          "flex h-6 w-6 shrink-0 items-center justify-center rounded-sp-sm text-sp-text-muted outline-none transition-[opacity,background-color,color] duration-fast ease-sp hover:bg-sp-hover hover:text-sp-text focus-visible:ring-1 focus-visible:ring-sp-accent cursor-pointer disabled:cursor-not-allowed disabled:opacity-40",
          !open && !disabled && revealClassName,
          open && "bg-sp-hover text-sp-text opacity-100",
          className
        )}
      >
        <MoreHorizontal className="h-4 w-4" aria-hidden="true" />
      </button>
      {open &&
        anchor &&
        typeof document !== "undefined" &&
        createPortal(
          <div onClick={(e) => e.stopPropagation()} onKeyDown={(e) => e.stopPropagation()}>
            <MenuPanel
              items={items}
              anchor={anchor}
              mode="below"
              label={label}
              panelRef={panelRef}
              onSelect={(action) => {
                close(true);
                onSelect(action);
              }}
              onDismiss={() => close(true)}
            />
          </div>,
          document.body
        )}
    </>
  );
}

export interface InlineRenameInputProps {
  initial: string;
  label: string;
  onCommit: (value: string) => void;
  onCancel: () => void;
  className?: string;
}

/** 行内重命名：Enter 保存，Esc 取消，失焦保存。 */
export function InlineRenameInput({ initial, label, onCommit, onCancel, className }: InlineRenameInputProps) {
  const [draft, setDraft] = useState(initial);
  const settled = useRef(false);
  const inputRef = useRef<HTMLInputElement | null>(null);

  useEffect(() => {
    const el = inputRef.current;
    if (!el) return;
    el.focus();
    el.select();
  }, []);

  const finish = (save: boolean) => {
    if (settled.current) return;
    settled.current = true;
    if (save) onCommit(draft);
    else onCancel();
  };

  return (
    <input
      ref={inputRef}
      type="text"
      aria-label={label}
      value={draft}
      maxLength={120}
      onChange={(e) => setDraft(e.target.value)}
      onClick={(e) => e.stopPropagation()}
      onKeyDown={(e) => {
        e.stopPropagation();
        if (e.key === "Enter") {
          e.preventDefault();
          finish(true);
        } else if (e.key === "Escape") {
          e.preventDefault();
          finish(false);
        }
      }}
      onBlur={() => finish(true)}
      className={cn(
        "h-7 min-w-0 flex-1 rounded-sp-sm bg-sp-surface px-2 text-xs text-sp-text outline-none ring-1.5 ring-sp-accent dark:bg-sp-card transition-shadow duration-fast ease-sp",
        className
      )}
    />
  );
}

export interface OrganizeNoteBarProps {
  note: { kind: "ok" | "error"; text: string; seq: number } | null;
  onDone: () => void;
  className?: string;
}

/** 短暂提示（导出文件名或操作失败），4 秒后自动消失。 */
export function OrganizeNoteBar({ note, onDone, className }: OrganizeNoteBarProps) {
  useEffect(() => {
    if (!note) return;
    const timer = window.setTimeout(onDone, 4000);
    return () => window.clearTimeout(timer);
  }, [note, onDone]);
  if (!note) return null;
  return (
    <div
      role={note.kind === "error" ? "alert" : "status"}
      title={note.text}
      className={cn(
        "truncate rounded-sp-sm px-2.5 py-1.5 text-xs",
        note.kind === "error" ? "bg-sp-danger-bg text-sp-danger-text" : "bg-sp-hover text-sp-text-secondary",
        className
      )}
    >
      {note.text}
    </div>
  );
}
