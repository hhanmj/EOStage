import React from "react";
import { cn } from "@/lib/utils";

export type EmptyStateIconName =
  | "folder"
  | "layers"
  | "plug"
  | "history"
  | "chart"
  | "inbox"
  | "archive"
  | "bell"
  | "alert"
  | "wifi-off"
  | "search"
  | "filter"
  | "lock";

export interface EmptyStateIconProps {
  name: EmptyStateIconName;
  size?: 48 | 22;
  /** 为「全部完成 / 无权限」准备的中性灰双色模式，不含任何强调色 */
  neutral?: boolean;
  className?: string;
}

/** 轻量双色空状态图标：纯内联 SVG，跟随浅深色 CSS 变量，零循环动画。 */
export function EmptyStateIcon({
  name,
  size = 48,
  neutral = false,
  className,
}: EmptyStateIconProps) {
  const strokeColor = neutral ? "var(--sp-text-muted)" : "var(--sp-empty-ink)";
  const bodyOpacity = neutral ? 0.22 : 0.34;

  const renderGlyph = () => {
    switch (name) {
      case "folder":
        return (
          <>
            <path
              d="M3 7.5A2 2 0 0 1 5 5.5h4.2l1.8 2H19a2 2 0 0 1 2 2v8a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-10Z"
              fill={strokeColor}
              fillOpacity={bodyOpacity}
              stroke={strokeColor}
            />
            <path d="M3 10.5h18" stroke={strokeColor} strokeOpacity={0.6} />
          </>
        );
      case "layers":
        return (
          <>
            <rect
              x="4"
              y="4.5"
              width="16"
              height="7"
              rx="2"
              fill={strokeColor}
              fillOpacity={bodyOpacity}
              stroke={strokeColor}
            />
            <path d="M4.5 14.5h15" stroke={strokeColor} />
            <path d="M4.5 18.5h15" stroke={strokeColor} />
          </>
        );
      case "plug":
        return (
          <>
            <path d="M12 22v-5" stroke={strokeColor} />
            <path d="M9 8V4" stroke={strokeColor} />
            <path d="M15 8V4" stroke={strokeColor} />
            <path
              d="M7 8h10v3.5a5 5 0 0 1-10 0V8Z"
              fill={strokeColor}
              fillOpacity={bodyOpacity}
              stroke={strokeColor}
            />
          </>
        );
      case "history":
        return (
          <>
            <circle
              cx="12.5"
              cy="12"
              r="7.5"
              fill={strokeColor}
              fillOpacity={bodyOpacity}
            />
            <path d="M4.5 12a8 8 0 1 0 2.3-5.6L4.5 8.5" stroke={strokeColor} />
            <path d="M4.5 4.5v4h4" stroke={strokeColor} />
            <path d="M12.5 8v4l2.8 1.6" stroke={strokeColor} />
          </>
        );
      case "chart":
        return (
          <>
            <rect
              x="4"
              y="12"
              width="4"
              height="7"
              rx="1.2"
              fill={strokeColor}
              fillOpacity={bodyOpacity}
              stroke={strokeColor}
            />
            <rect
              x="10"
              y="7"
              width="4"
              height="12"
              rx="1.2"
              fill={strokeColor}
              fillOpacity={bodyOpacity}
              stroke={strokeColor}
            />
            <rect
              x="16"
              y="10"
              width="4"
              height="9"
              rx="1.2"
              fill={strokeColor}
              fillOpacity={bodyOpacity}
              stroke={strokeColor}
            />
            <path d="M3 19.5h18" stroke={strokeColor} />
          </>
        );
      case "inbox":
        return (
          <>
            <path
              d="M4 13.5h4.2l1.4 2.2h4.8l1.4-2.2H20V18a2 2 0 0 1-2 2H6a2 2 0 0 1-2-2v-4.5Z"
              fill={strokeColor}
              fillOpacity={bodyOpacity}
              stroke={strokeColor}
            />
            <path
              d="M4 13.5 5.8 6A2 2 0 0 1 7.7 4.5h8.6A2 2 0 0 1 18.2 6l1.8 7.5"
              stroke={strokeColor}
            />
            <path d="m9.5 9.5 1.8 1.8 3.4-3.4" stroke={strokeColor} />
          </>
        );
      case "archive":
        return (
          <>
            <rect
              x="3.5"
              y="4.5"
              width="17"
              height="4"
              rx="1.2"
              fill={strokeColor}
              fillOpacity={bodyOpacity}
              stroke={strokeColor}
            />
            <path
              d="M5 8.5v9.5a2 2 0 0 0 2 2h10a2 2 0 0 0 2-2V8.5"
              fill={strokeColor}
              fillOpacity={bodyOpacity * 0.7}
              stroke={strokeColor}
            />
            <path d="M10 12.5h4" stroke={strokeColor} />
          </>
        );
      case "bell":
        return (
          <>
            <path
              d="M6.5 10a5.5 5.5 0 0 1 11 0c0 4 1.5 5.5 2.3 6.3H4.2C5 15.5 6.5 14 6.5 10Z"
              fill={strokeColor}
              fillOpacity={bodyOpacity}
              stroke={strokeColor}
            />
            <path d="M10.2 18.5a2 2 0 0 0 3.6 0" stroke={strokeColor} />
          </>
        );
      case "alert":
        return (
          <>
            <circle
              cx="12"
              cy="12"
              r="8.5"
              fill={strokeColor}
              fillOpacity={bodyOpacity}
              stroke={strokeColor}
            />
            <path d="M12 8.2v4.6" stroke={strokeColor} />
            <circle cx="12" cy="15.8" r="0.9" fill={strokeColor} stroke="none" />
          </>
        );
      case "wifi-off":
        return (
          <>
            <circle
              cx="12"
              cy="17.5"
              r="2.2"
              fill={strokeColor}
              fillOpacity={bodyOpacity}
              stroke={strokeColor}
            />
            <path d="M8.5 14a5.2 5.2 0 0 1 7 0" stroke={strokeColor} />
            <path d="M5.2 10.5a9.8 9.8 0 0 1 13.6 0" stroke={strokeColor} />
            <path d="M4.2 4.2 19.8 19.8" stroke={strokeColor} />
          </>
        );
      case "search":
        return (
          <>
            <circle
              cx="11"
              cy="11"
              r="6.5"
              fill={strokeColor}
              fillOpacity={bodyOpacity}
              stroke={strokeColor}
            />
            <path d="m15.8 15.8 3.7 3.7" stroke={strokeColor} />
          </>
        );
      case "filter":
        return (
          <>
            <path
              d="M4 5.5h16l-5.8 6.8v4.8l-4.4 2v-6.8L4 5.5Z"
              fill={strokeColor}
              fillOpacity={bodyOpacity}
              stroke={strokeColor}
            />
          </>
        );
      case "lock":
        return (
          <>
            <path d="M8 10.5V7.8a4 4 0 0 1 8 0v2.7" stroke={strokeColor} />
            <rect
              x="5.5"
              y="10.5"
              width="13"
              height="9"
              rx="2.5"
              fill={strokeColor}
              fillOpacity={bodyOpacity}
              stroke={strokeColor}
            />
            <circle cx="12" cy="14.8" r="1.2" fill={strokeColor} stroke="none" />
          </>
        );
    }
  };

  return (
    <svg
      width={size}
      height={size}
      viewBox="0 0 24 24"
      fill="none"
      strokeWidth={1.6}
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden="true"
      className={cn("shrink-0 select-none", className)}
    >
      {renderGlyph()}
    </svg>
  );
}

export type EmptyStateWireframeVariant =
  | "channel-setup"
  | "scene-search"
  | "scene-list"
  | "plugin-grid"
  | "no-results"
  | "no-access";

export interface EmptyStateWireframeProps {
  variant: EmptyStateWireframeVariant;
  /** 仅首次使用场景的下一步允许出现静谧强调圆钮，无结果/无权限/全部完成均不得使用强调色 */
  showAccentAction?: boolean;
  className?: string;
}

/**
 * 缩小的界面线框图示：
 * - 页面空态使用 viewBox 0 0 120 90，无结果使用 80 × 60
 * - 无底板/网点/描边外框，卡片白面 + --sp-line 边框 + 0 2px 3px 柔和投影
 * - 四周保留充足留白，最多 1 处低饱和强调圆钮用于下一步操作
 */
export function EmptyStateWireframe({
  variant,
  showAccentAction = false,
  className,
}: EmptyStateWireframeProps) {
  if (variant === "no-results") {
    return (
      <svg
        width={80}
        height={60}
        viewBox="0 0 80 60"
        fill="none"
        aria-hidden="true"
        className={cn("shrink-0 select-none overflow-visible", className)}
      >
        <g className="sp-empty-wireframe-card">
          <rect
            x="14"
            y="14"
            width="52"
            height="32"
            rx="10"
            fill="var(--sp-surface)"
            stroke="var(--sp-line)"
            strokeWidth="1"
          />
        </g>
        <rect
          x="21"
          y="21"
          width="38"
          height="10"
          rx="5"
          fill="var(--sp-text-muted)"
          fillOpacity="0.14"
        />
        <circle
          cx="27.5"
          cy="26"
          r="2.4"
          stroke="var(--sp-empty-ink)"
          strokeWidth="1.35"
        />
        <line
          x1="29.3"
          y1="27.8"
          x2="31"
          y2="29.5"
          stroke="var(--sp-empty-ink)"
          strokeWidth="1.35"
          strokeLinecap="round"
        />
        <rect
          x="34"
          y="24"
          width="18"
          height="4"
          rx="2"
          fill="var(--sp-text-muted)"
          fillOpacity="0.28"
        />
        <rect
          x="21"
          y="36"
          width="24"
          height="4"
          rx="2"
          fill="var(--sp-text-muted)"
          fillOpacity="0.28"
        />
        <line
          x1="51"
          y1="38"
          x2="58"
          y2="38"
          stroke="var(--sp-text-muted)"
          strokeOpacity="0.45"
          strokeWidth="1.5"
          strokeLinecap="round"
        />
      </svg>
    );
  }

  const allowAccent = showAccentAction && variant !== "no-access";

  return (
    <svg
      width={120}
      height={90}
      viewBox="0 0 120 90"
      fill="none"
      aria-hidden="true"
      className={cn("shrink-0 select-none overflow-visible", className)}
    >
      {variant === "channel-setup" && (
        <>
          <g className="sp-empty-wireframe-card">
            <rect
              x="16"
              y="18"
              width="66"
              height="52"
              rx="10"
              fill="var(--sp-surface)"
              stroke="var(--sp-line)"
              strokeWidth="1"
            />
          </g>
          <rect
            x="24"
            y="27"
            width="28"
            height="5"
            rx="2.5"
            fill="var(--sp-text-muted)"
            fillOpacity="0.52"
          />
          <rect
            x="24"
            y="37"
            width="48"
            height="10"
            rx="4"
            fill="var(--sp-text-muted)"
            fillOpacity="0.14"
          />
          <rect
            x="28"
            y="40"
            width="22"
            height="4"
            rx="2"
            fill="var(--sp-text-muted)"
            fillOpacity="0.28"
          />
          <rect
            x="24"
            y="52"
            width="48"
            height="10"
            rx="4"
            fill="var(--sp-text-muted)"
            fillOpacity="0.14"
          />
          <rect
            x="28"
            y="55"
            width="30"
            height="4"
            rx="2"
            fill="var(--sp-text-muted)"
            fillOpacity="0.28"
          />
          <g className="sp-empty-wireframe-card">
            <rect
              x="68"
              y="32"
              width="36"
              height="38"
              rx="10"
              fill="var(--sp-surface)"
              stroke="var(--sp-line)"
              strokeWidth="1"
            />
          </g>
          <rect
            x="76"
            y="41"
            width="20"
            height="5"
            rx="2.5"
            fill="var(--sp-text-muted)"
            fillOpacity="0.52"
          />
          <rect
            x="76"
            y="50"
            width="16"
            height="4"
            rx="2"
            fill="var(--sp-text-muted)"
            fillOpacity="0.28"
          />
          {allowAccent && (
            <g>
              <circle cx="96" cy="64" r="8" fill="var(--sp-empty-accent)" />
              <line
                x1="96"
                y1="60.8"
                x2="96"
                y2="67.2"
                stroke="#ffffff"
                strokeWidth="1.6"
                strokeLinecap="round"
              />
              <line
                x1="92.8"
                y1="64"
                x2="99.2"
                y2="64"
                stroke="#ffffff"
                strokeWidth="1.6"
                strokeLinecap="round"
              />
            </g>
          )}
        </>
      )}

      {variant === "scene-search" && (
        <>
          <g className="sp-empty-wireframe-card">
            <rect
              x="16"
              y="16"
              width="86"
              height="56"
              rx="10"
              fill="var(--sp-surface)"
              stroke="var(--sp-line)"
              strokeWidth="1"
            />
          </g>
          <rect
            x="24"
            y="24"
            width="18"
            height="18"
            rx="5"
            fill="var(--sp-text-muted)"
            fillOpacity="0.14"
          />
          <rect
            x="48"
            y="25"
            width="36"
            height="5"
            rx="2.5"
            fill="var(--sp-text-muted)"
            fillOpacity="0.52"
          />
          <rect
            x="48"
            y="34"
            width="26"
            height="4"
            rx="2"
            fill="var(--sp-text-muted)"
            fillOpacity="0.28"
          />
          <rect
            x="24"
            y="48"
            width="18"
            height="16"
            rx="5"
            fill="var(--sp-text-muted)"
            fillOpacity="0.14"
          />
          <rect
            x="48"
            y="50"
            width="32"
            height="5"
            rx="2.5"
            fill="var(--sp-text-muted)"
            fillOpacity="0.52"
          />
          <rect
            x="48"
            y="58"
            width="22"
            height="4"
            rx="2"
            fill="var(--sp-text-muted)"
            fillOpacity="0.28"
          />
          {allowAccent && (
            <g>
              <circle cx="96" cy="64" r="8" fill="var(--sp-empty-accent)" />
              <circle
                cx="95"
                cy="63"
                r="2.7"
                stroke="#ffffff"
                strokeWidth="1.5"
              />
              <line
                x1="97"
                y1="65"
                x2="99.2"
                y2="67.2"
                stroke="#ffffff"
                strokeWidth="1.5"
                strokeLinecap="round"
              />
            </g>
          )}
        </>
      )}

      {variant === "scene-list" && (
        <>
          <g className="sp-empty-wireframe-card">
            <rect
              x="16"
              y="16"
              width="88"
              height="24"
              rx="10"
              fill="var(--sp-surface)"
              stroke="var(--sp-line)"
              strokeWidth="1"
            />
          </g>
          <rect
            x="24"
            y="22"
            width="12"
            height="12"
            rx="3.5"
            fill="var(--sp-text-muted)"
            fillOpacity="0.14"
          />
          <rect
            x="42"
            y="23"
            width="38"
            height="5"
            rx="2.5"
            fill="var(--sp-text-muted)"
            fillOpacity="0.52"
          />
          <rect
            x="42"
            y="31"
            width="24"
            height="4"
            rx="2"
            fill="var(--sp-text-muted)"
            fillOpacity="0.28"
          />
          <g className="sp-empty-wireframe-card">
            <rect
              x="16"
              y="46"
              width="88"
              height="26"
              rx="10"
              fill="var(--sp-surface)"
              stroke="var(--sp-line)"
              strokeWidth="1"
            />
          </g>
          <rect
            x="24"
            y="53"
            width="12"
            height="12"
            rx="3.5"
            fill="var(--sp-text-muted)"
            fillOpacity="0.14"
          />
          <rect
            x="42"
            y="54"
            width="32"
            height="5"
            rx="2.5"
            fill="var(--sp-text-muted)"
            fillOpacity="0.52"
          />
          <rect
            x="42"
            y="62"
            width="28"
            height="4"
            rx="2"
            fill="var(--sp-text-muted)"
            fillOpacity="0.28"
          />
        </>
      )}

      {variant === "plugin-grid" && (
        <>
          <g className="sp-empty-wireframe-card">
            <rect
              x="18"
              y="16"
              width="38"
              height="26"
              rx="10"
              fill="var(--sp-surface)"
              stroke="var(--sp-line)"
              strokeWidth="1"
            />
          </g>
          <rect
            x="25"
            y="23"
            width="20"
            height="5"
            rx="2.5"
            fill="var(--sp-text-muted)"
            fillOpacity="0.52"
          />
          <rect
            x="25"
            y="31"
            width="24"
            height="4"
            rx="2"
            fill="var(--sp-text-muted)"
            fillOpacity="0.28"
          />

          <g className="sp-empty-wireframe-card">
            <rect
              x="62"
              y="16"
              width="38"
              height="26"
              rx="10"
              fill="var(--sp-surface)"
              stroke="var(--sp-line)"
              strokeWidth="1"
            />
          </g>
          <rect
            x="69"
            y="23"
            width="18"
            height="5"
            rx="2.5"
            fill="var(--sp-text-muted)"
            fillOpacity="0.52"
          />
          <rect
            x="69"
            y="31"
            width="22"
            height="4"
            rx="2"
            fill="var(--sp-text-muted)"
            fillOpacity="0.28"
          />

          <g className="sp-empty-wireframe-card">
            <rect
              x="18"
              y="48"
              width="38"
              height="26"
              rx="10"
              fill="var(--sp-surface)"
              stroke="var(--sp-line)"
              strokeWidth="1"
            />
          </g>
          <rect
            x="25"
            y="55"
            width="22"
            height="5"
            rx="2.5"
            fill="var(--sp-text-muted)"
            fillOpacity="0.52"
          />
          <rect
            x="25"
            y="63"
            width="16"
            height="4"
            rx="2"
            fill="var(--sp-text-muted)"
            fillOpacity="0.28"
          />

          <rect
            x="62"
            y="48"
            width="38"
            height="26"
            rx="10"
            fill="var(--sp-text-muted)"
            fillOpacity="0.14"
            stroke="var(--sp-line)"
            strokeWidth="1"
            strokeDasharray="3 3"
          />
          {allowAccent && (
            <g>
              <circle cx="96" cy="68" r="8" fill="var(--sp-empty-accent)" />
              <line
                x1="96"
                y1="64.8"
                x2="96"
                y2="71.2"
                stroke="#ffffff"
                strokeWidth="1.6"
                strokeLinecap="round"
              />
              <line
                x1="92.8"
                y1="68"
                x2="99.2"
                y2="68"
                stroke="#ffffff"
                strokeWidth="1.6"
                strokeLinecap="round"
              />
            </g>
          )}
        </>
      )}

      {variant === "no-access" && (
        <>
          <g className="sp-empty-wireframe-card">
            <rect
              x="18"
              y="18"
              width="82"
              height="52"
              rx="10"
              fill="var(--sp-surface)"
              stroke="var(--sp-line)"
              strokeWidth="1"
            />
          </g>
          <rect
            x="27"
            y="27"
            width="34"
            height="5"
            rx="2.5"
            fill="var(--sp-text-muted)"
            fillOpacity="0.52"
          />
          <rect
            x="27"
            y="37"
            width="52"
            height="4"
            rx="2"
            fill="var(--sp-text-muted)"
            fillOpacity="0.28"
          />
          <rect
            x="27"
            y="45"
            width="42"
            height="14"
            rx="4"
            fill="var(--sp-text-muted)"
            fillOpacity="0.14"
          />
          <g className="sp-empty-wireframe-card">
            <circle
              cx="92"
              cy="62"
              r="11"
              fill="var(--sp-surface)"
              stroke="var(--sp-line)"
              strokeWidth="1"
            />
          </g>
          <path
            d="M88.8 61v-1.8a3.2 3.2 0 0 1 6.4 0V61"
            stroke="var(--sp-empty-ink)"
            strokeWidth="1.5"
            strokeLinecap="round"
          />
          <rect
            x="86.8"
            y="61"
            width="10.4"
            height="7.5"
            rx="2"
            fill="var(--sp-empty-ink)"
            fillOpacity="0.28"
            stroke="var(--sp-empty-ink)"
            strokeWidth="1.5"
          />
        </>
      )}
    </svg>
  );
}
