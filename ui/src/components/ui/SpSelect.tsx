import React, { useState, useRef, useEffect, useId } from "react";
import { ChevronDown, Check } from "lucide-react";
import { cn } from "@/lib/utils";

export interface SpSelectOption {
  value: string;
  label: string;
  disabled?: boolean;
  description?: string;
}

export interface SpSelectProps {
  value: string;
  onChange: (value: string) => void;
  options: readonly SpSelectOption[];
  placeholder?: string;
  disabled?: boolean;
  className?: string;
  triggerClassName?: string;
  popupClassName?: string;
  "aria-label"?: string;
  "data-testid"?: string;
  size?: "sm" | "md";
}

export function SpSelect({
  value,
  onChange,
  options,
  placeholder = "请选择",
  disabled = false,
  className,
  triggerClassName,
  popupClassName,
  "aria-label": ariaLabel,
  "data-testid": dataTestId,
  size = "sm",
}: SpSelectProps) {
  const [isOpen, setIsOpen] = useState(false);
  const [highlightedIndex, setHighlightedIndex] = useState(-1);
  const containerRef = useRef<HTMLDivElement>(null);
  const triggerRef = useRef<HTMLButtonElement>(null);
  const listboxRef = useRef<HTMLUListElement>(null);
  const listboxId = useId();

  const selectedOption = options.find((opt) => opt.value === value);

  // 点击外部关闭
  useEffect(() => {
    if (!isOpen) return;
    const handleClickOutside = (e: MouseEvent) => {
      if (containerRef.current && !containerRef.current.contains(e.target as Node)) {
        setIsOpen(false);
      }
    };
    document.addEventListener("mousedown", handleClickOutside);
    return () => document.removeEventListener("mousedown", handleClickOutside);
  }, [isOpen]);

  // 打开时初始化高亮项为当前选中项
  useEffect(() => {
    if (isOpen) {
      const idx = options.findIndex((opt) => opt.value === value);
      setHighlightedIndex(idx >= 0 ? idx : 0);
    }
  }, [isOpen, value, options]);

  // 键盘导航
  const handleKeyDown = (e: React.KeyboardEvent) => {
    if (disabled) return;

    if (!isOpen) {
      if (e.key === "Enter" || e.key === " " || e.key === "ArrowDown" || e.key === "ArrowUp") {
        e.preventDefault();
        setIsOpen(true);
      }
      return;
    }

    switch (e.key) {
      case "Escape":
        e.preventDefault();
        setIsOpen(false);
        triggerRef.current?.focus();
        break;
      case "Tab":
        setIsOpen(false);
        break;
      case "ArrowDown": {
        e.preventDefault();
        let nextIdx = (highlightedIndex + 1) % options.length;
        while (options[nextIdx]?.disabled && nextIdx !== highlightedIndex) {
          nextIdx = (nextIdx + 1) % options.length;
        }
        setHighlightedIndex(nextIdx);
        break;
      }
      case "ArrowUp": {
        e.preventDefault();
        let prevIdx = (highlightedIndex - 1 + options.length) % options.length;
        while (options[prevIdx]?.disabled && prevIdx !== highlightedIndex) {
          prevIdx = (prevIdx - 1 + options.length) % options.length;
        }
        setHighlightedIndex(prevIdx);
        break;
      }
      case "Enter":
      case " ": {
        e.preventDefault();
        const opt = options[highlightedIndex];
        if (opt && !opt.disabled) {
          onChange(opt.value);
          setIsOpen(false);
          triggerRef.current?.focus();
        }
        break;
      }
    }
  };

  const handleSelect = (opt: SpSelectOption) => {
    if (opt.disabled) return;
    onChange(opt.value);
    setIsOpen(false);
    triggerRef.current?.focus();
  };

  const sizeClasses =
    size === "sm"
      ? "h-7 px-2 text-xs"
      : "h-8 px-2.5 text-xs";

  return (
    <div
      ref={containerRef}
      className={cn("relative inline-block w-full text-xs select-none", className)}
      onKeyDown={handleKeyDown}
    >
      <button
        ref={triggerRef}
        type="button"
        role="combobox"
        aria-expanded={isOpen}
        aria-haspopup="listbox"
        aria-controls={listboxId}
        aria-label={ariaLabel}
        data-testid={dataTestId}
        disabled={disabled}
        onClick={() => !disabled && setIsOpen((prev) => !prev)}
        className={cn(
          "w-full rounded-sp-sm border border-sp-line bg-sp-surface text-sp-text flex items-center justify-between gap-1.5 transition-colors duration-fast outline-none",
          "hover:bg-sp-hover focus-visible:border-sp-accent focus-visible:ring-1 focus-visible:ring-sp-accent",
          disabled && "opacity-50 cursor-not-allowed hover:bg-sp-surface",
          sizeClasses,
          triggerClassName
        )}
      >
        <span className={cn("truncate text-left flex-1", !selectedOption && "text-sp-text-muted")}>
          {selectedOption ? selectedOption.label : placeholder}
        </span>
        <ChevronDown
          className={cn(
            "w-3.5 h-3.5 text-sp-text-secondary shrink-0 transition-transform duration-fast",
            isOpen && "rotate-180 text-sp-text"
          )}
        />
      </button>

      {isOpen && (
        <ul
          id={listboxId}
          ref={listboxRef}
          role="listbox"
          tabIndex={-1}
          aria-label={ariaLabel}
          className={cn(
            "absolute left-0 min-w-full w-max max-w-[360px] z-50 mt-1 max-h-56 overflow-y-auto rounded-card border border-sp-line bg-sp-surface py-1 shadow-overlay animate-in fade-in-50 zoom-in-95 duration-fast outline-none",
            popupClassName
          )}
        >
          {options.map((opt, idx) => {
            const isSelected = opt.value === value;
            const isHighlighted = idx === highlightedIndex;

            return (
              <li
                key={opt.value}
                role="option"
                title={opt.description ? `${opt.label} — ${opt.description}` : opt.label}
                aria-selected={isSelected}
                aria-disabled={opt.disabled}
                onClick={() => handleSelect(opt)}
                onMouseEnter={() => !opt.disabled && setHighlightedIndex(idx)}
                className={cn(
                  "relative flex items-center justify-between gap-2 px-2.5 py-1.5 cursor-pointer text-xs transition-colors",
                  isHighlighted && !opt.disabled && "bg-sp-hover",
                  isSelected && "font-semibold text-sp-text",
                  !isSelected && "text-sp-text-secondary",
                  opt.disabled && "opacity-40 cursor-not-allowed hover:bg-transparent"
                )}
              >
                <div className="flex flex-col min-w-0 flex-1">
                  <span className="whitespace-nowrap">{opt.label}</span>
                  {opt.description && (
                    <span className="text-sp-text-muted text-xs whitespace-normal leading-tight">
                      {opt.description}
                    </span>
                  )}
                </div>
                {isSelected && (
                  <Check className="w-3.5 h-3.5 text-sp-accent shrink-0" />
                )}
              </li>
            );
          })}
        </ul>
      )}
    </div>
  );
}
