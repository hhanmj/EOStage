# EOStage Design Guidelines

**Document Type:** Contributor UI & Interaction Guidelines
**Maintainers:** EOStage contributors

## Scope

This file describes visual design, interaction, layout, information density, components, maps, motion, and UI accessibility in EOStage. `PRODUCT.md` covers product scope and intent, while `contracts/` defines verified runtime schemas.

Design must not redefine provider behavior, scientific semantics, persistence, task transitions, or error truth. Review cross-domain conflicts at the affected boundary; do not halt unrelated authorized work.

## Current UI Stack

The inspected implementation uses React, TypeScript, Vite, Tailwind CSS v4, Lucide, and Leaflet. These are current implementation choices, not permanent product requirements. Introducing another UI kit, icon system, mapping engine, or motion library requires focused adoption review.

## Implemented Tokens

The canonical token vocabulary is the CSS variables currently implemented in `ui/src/index.css`. This document records semantic roles only; it does not create aliases or variables that do not exist in code.

| Implemented token | Semantic role |
| --- | --- |
| `--background`, `--foreground` | Application canvas and primary text |
| `--card`, `--card-foreground` | Framed item or tool surface and its text |
| `--popover`, `--popover-foreground` | Temporary overlay surface and text |
| `--primary`, `--primary-foreground` | Primary command or active emphasis |
| `--secondary`, `--secondary-foreground` | Secondary command surface and text |
| `--muted`, `--muted-foreground` | De-emphasized surface and supporting text |
| `--accent`, `--accent-foreground` | Local highlight and its text |
| `--destructive`, `--destructive-foreground` | Destructive action and its text |
| `--success`, `--warning` | Positive and caution status accents |
| `--border`, `--input`, `--ring` | Boundaries, input treatment, and focus indication |
| `--font-sans` | Interface text: Segoe UI, Chinese system faces, then Yu Gothic UI and Meiryo UI; under `:lang(ja)`, the Japanese faces take precedence for Japanese kanji glyphs |
| `--radius` | Current base radius input |

The Agent shell (`DesktopWorkbench` root, class `.sp-modern`) redefines these inside its own subtree only; the classic manual workbench (`#legacy`) keeps the defaults:

| Implemented token (inside `.sp-modern`) | Value | Semantic role |
| --- | --- | --- |
| `--text-xs` / `--text-xs--line-height` | 13px / 20px | Labels, metadata, secondary controls |
| `--text-sm` / `--text-sm--line-height` | 14px / 22px | Card body and primary controls |
| `--text-base` / `--text-base--line-height` | 15px / 26px | Conversation body (with `leading-[1.75]`) |
| `--text-lg`, `--text-xl` | 17px, 20px | Panel and page titles |
| `--sp-canvas`, `--sp-surface`, `--sp-card` | light `#f3f3f2` / `#ffffff` / `#f6f6f5`; dark `#0e0e0e` / `#171717` / `#1f1f1f` | Neutral shell canvas (sidebar, title bar), main panel, soft card fill |
| `--sp-line`, `--sp-divider` | light 8% / 6% black; dark 9% / 7% white | Separation is by soft fills and spacing first; lines are secondary |
| `--sp-hover`, `--sp-selected` | light 4% / 6% black; dark 5% / 8% white | Row hover and current item |
| `--sp-text`, `--sp-text-secondary`, `--sp-text-muted` | light `#0a0a0a` / `#525252` / `#737373`; dark `#fafafa` / `#a3a3a3` / `#8f8f8f` | Neutral text hierarchy |
| `--sp-accent`, `--sp-accent-strong` | light `#2563eb` / `#1d4ed8`; dark `#60a5fa` / `#93c5fd` | Links, focus, in-progress state (≥ 5:1 on the surface) |
| `--sp-accent-fill` | light `#18181b`, dark `#f4f4f5` | Primary carbon-black button fill in light (`#ffffff` ink) / high-contrast fill in dark (`#09090b` ink); blue is not used for primary buttons |
| `--sp-token-skill-bg`, `--sp-token-skill-border`, `--sp-token-skill-ink` | light `#f5f3ff` / `22%` violet / `#6d28d9`; dark `14%` `#8b5cf6` / `30%` `#a78bfa` / `#ddd6fe` | Violet semantic entity chip for `skill` commands & selected composer workflows |
| `--sp-token-dataset-bg`, `--sp-token-dataset-border`, `--sp-token-dataset-ink` | light `#eef4ff` / `22%` cobalt / `#1e40af`; dark `14%` `#60a5fa` / `28%` `#60a5fa` / `#bfdbfe` | Mineral cobalt entity chip for `@dataset` sensor & catalog references (≥ 5.5:1 contrast) |
| `--sp-token-aoi-bg`, `--sp-token-aoi-border`, `--sp-token-aoi-ink` | light `#f0fdfa` / `24%` teal / `#0f766e`; dark `13%` `#2dd4bf` / `28%` `#2dd4bf` / `#99f6e4` | Cartographic mineral-teal entity chip for `#AOI` geographic regions, kept visually distinct from `--sp-success-*` operational status badges |
| `--sp-sem-violet-dot`, `--sp-sem-cobalt-dot`, `--sp-sem-teal-dot`, `--sp-sem-amber-dot`, `--sp-sem-emerald-dot` | light `#7c3aed` / `#2563eb` / `#0d9488` / `#d97706` / `#059669`; dark `#a78bfa` / `#60a5fa` / `#2dd4bf` / `#fbbf24` / `#34d399` | Context capacity and telemetry bar/legend indicator dots (`AgentContextHud`) |
| `--sp-glass-rim`, `--sp-glass-inner`, `--sp-glass-edge`, `--sp-glass-contact`, `--sp-glass-shade` | light `94%` white / `45%` white / `7.5%` ink / `4%` ink / `7%` ink; dark `14%` white / `4%` white / `8%` white / `32%` black / `56%` black | Solid Glass tactile bevel system: 1px top specular rim, 1px inner ring glow, 1px outer hairline edge, and progressive ambient elevation |
| `--sp-well-shade`, `--sp-well-rim`, `--sp-primary-rim`, `--sp-primary-under` | light `6%` ink / `90%` white / `25%` white / `40%` black; dark `42%` black / `8%` white / `72%` white / `28%` black | Recessed groove/input well shadows (`.sp-field-well`, `.sp-meter-track`) and carbon-black primary button bevels |
| `--sp-shadow-card`, `--sp-shadow-overlay`, `--sp-shadow-modal` | 3-tier Solid Glass elevation (`shadow-card`, `shadow-overlay`, `shadow-modal` / `shadow-sp-*`) | Raised cards, anchored overlays/popovers, and top-level modals without changing surface colors |
| `--radius-sp-sm`, `--radius-sp-md`, `--radius-sp-card`, `--radius-sp-modal`, `--radius-sp-dialog`, `--radius-sp-full` | `6px` / `8px` / `10px` / `14px` / `14px` / `9999px` (also exposed via `--radius-sm/md/card/modal/dialog/full`) | Global radius scale for compact controls/chips, inputs/buttons, cards, dialogs/modals, and pills |
| `--sp-height-control`, `--sp-height-row`, `--sp-height-card-single`, `--sp-height-card-bundle` | `32px` / `36px` / `148px` / `220px` (also `--spacing-sp-*`) | Canonical fixed heights for standard controls, list rows, and catalog/bundle cards |

**Design Token & Component Continuity**: Primary actions keep EOStage's carbon-black fill (`--sp-accent-fill`) across all themes. Chromatic hues are strictly rationed to verified state truth (`--sp-success-*`, `--sp-warning-*`, `--sp-danger-*`), interactive links/focus (`--sp-accent`), and governed entity chips (`--sp-token-*`). Components must consume implemented `--sp-*` and `--radius-sp-*` CSS custom properties and reuse shared primitives in `ui/src/components/ui/` rather than hardcoding raw hex/`rgba(...)` colors, Tailwind raw palettes (`slate`/`zinc`/`blue`), or one-off button/card styles (Settings and Download Center enforce token/spacing/radius rules via `ui/src/shell/api/w545SettingsTokens.test.ts`).


When tokens change in code, update this table in the same reviewed change. Do not reference a planned token as though it were implemented.

## Layout And Workflow Structure

- Keep one primary workbench for a business workflow.
- Extend capability-driven controls rather than cloning workbenches for each provider or sensor.
- Keep search-state, map-state, selection-state, and task-snapshot ownership explicit where their lifecycles differ.
- Operational desktop screens use constrained, medium-high density layouts optimized for scanning and repeated action.
- Page sections are structural regions, not floating decorative cards.
- Cards are reserved for repeated items, modals, and genuinely framed tools; do not nest cards.
- **Global Radius Scale, Concentric Corner Geometry & Continuous-Corner Gate**:
  - Manage corner radii through the canonical `ui/src/index.css` token scale (`--radius-sp-sm: 6px`, `--radius-sp-md: 8px`, `--radius-sp-card: 10px`, `--radius-sp-modal` / `--radius-sp-dialog: 14px`, `--radius-sp-full: 9999px`). Do not introduce ad-hoc arbitrary pixel radii (`rounded-[5px]`, `rounded-[7px]`, etc.) in new or refactored code unless computing an exact concentric inner radius.
  - **Concentric Nesting (`r_inner = max(0, r_outer - gap)`)**: When an interactive control, input shell, or inner surface sits inside a rounded container, derive its radius from the outer container radius minus the padding/gap between them (`r_inner <= r_outer`) so nested curves stay concentric and never clash (for example, a `10px` card with `4px` inset uses `6px` `--radius-sp-sm` for inner rows).
  - **Continuous Corners (Squircle) — Progressive Enhancement Only**: Because the desktop ships on dual WebViews (macOS `WKWebView` and Windows `WebView2`), continuous corners (such as standardized CSS `corner-shape: squircle`) are permitted **only** as zero-runtime-cost progressive enhancement behind `@supports` on primary containers when verified not to break `1px` borders, Solid Glass inset shadows, focus rings, or cross-platform visual parity. Never emulate continuous corners via JS/SVG `clip-path` hacks on virtualized lists, map overlays, or high-frequency progress surfaces.
- **Global 4px Spacing System, Semantic Nesting & Symmetric Insets**:
  - Use multiples of `4px` (`4, 8, 12, 16, 20, 24, 32px` -> Tailwind `1, 2, 3, 4, 5, 6, 8`) as the default spacing scale for gaps, padding, and margins; do not write arbitrary pixel spacings (`gap-[7px]`, `p-[11px]`). Settings and Download Center enforce zero half-step spacing via `w545SettingsTokens.test.ts`; elsewhere, micro-steps (`2px` / `0.5` or `6px` / `1.5`) are narrow exceptions restricted to compact inline chips, badges, or icon-label pairs when `4px` clips or `8px` hurts `920 x 620` density.
  - **Semantic Nesting Spacing (`intra-unit < inter-group < inter-section`)**: Allocate spacing strictly by logical hierarchy — tightest (`4px`) inside one atomic item (icon + label, key + value), standard (`8px–12px`) between sibling controls/rows within a group, and wider (`16px–32px`) between distinct sections. Internal siblings must never have a wider gap than the separation between parent groups.
  - **Symmetric Container Insets**: When nesting elements inside a self-contained container (card, modal, drawer body, inset well, alert box), keep top, right, bottom, and left padding equal (`p-3`, `p-4`, `p-5`, or `p-1` concentric outer shells) so inner boundaries align evenly on all sides. Reserve asymmetric `px > py` padding for single-line horizontal controls (buttons, inputs, pills, badges, list rows, and slim header/footer bars).
- **Responsive `minChildWidth` Card Grids**: Multi-card collections (plugins, skills, data source cards, model/account cards) must use responsive auto-fill/auto-fit grids that adapt smoothly to panel and drawer width rather than ragged `flex-wrap` card rows or rigid viewport-only column jumps: `.sp-grid-cards-2col` (`184px` min) for paired 2-up cards inside split panes, `.sp-grid-cards-sm` (`216px` min) for compact presets, `.sp-grid-cards` (`232px` min) for standard 3-column desktop workbenches, `.sp-grid-cards-md` (`280px` min) for multi-action connector/harness cards, and `.sp-grid-cards-lg` (`320px` min) for rich plugin bundles. Scrollable vertical menus and flex lists using `.sp-row-compact` or `.sp-row-balanced` must carry `flex-shrink: 0` so multi-line rows are never crushed below their content height. Reserve linear `flex` rows/columns for toolbars, header bars, and vertical form stacks.
- **Desktop Shell Chrome, Platform Conventions & Layering**: Respect native platform affordances across macOS (`⌘`/`⌥`/`⌃` shortcuts, left traffic-light clearance) and Windows (`Ctrl`/`Alt` shortcuts); render modals and viewport-sensitive pickers through portals (`ModalOverlay` / `createPortal`) so `overflow-hidden` or scroll-fade masks never clip them and Leaflet panes never bleed above app overlays.
- Stable grids, row heights, aspect ratios, and control dimensions prevent state changes from shifting the layout.

One workbench is one coherent operational context, not one oversized component.
Linked map, catalogue, asset inspector and task views may coexist as projections
of the same owned objects. Extract components and controllers by lifecycle instead
of adding another provider-specific panel or another persistent selection store.

The desktop currently declares a 920 x 620 minimum window in code. Treat this as a current implementation constraint, not a permanent design standard. A different minimum requires runtime measurement, focused verification, and maintainer approval before this document is changed.

## Information Hierarchy

- Reserve large display text for true product or page identity, not compact tools.
- Keep headings inside panels small and aligned with their information density.
- Use labels, grouping, and whitespace to expose hierarchy before adding decoration.
- Preserve inspectable detail for long identifiers, filenames, paths, scientific metadata, and error messages.
- Truncation requires a reliable way to inspect the complete value.
- Technical identifiers and structured values may use monospace text where comparison benefits; use `tabular-nums` on dynamic numerical readouts (transfer bytes, percentages, speeds, coordinates, counts, elapsed time) so frequent updates never shift horizontally.

## UX Writing And Progressive Disclosure

- **Page Positioning & High-Signal Copy**: Define the page or panel's primary job before writing copy. Retain only the core title, concise section headings, control labels, and scannable **key-value facts** (e.g., `保存方式: 直接保存到所选文件夹`, `云量: <= 20%`). Remove decorative subtitles, introductory filler, and prose that merely narrates self-evident controls.
- **Formal Written Register**: All user-facing copy (`zh-CN`, `en`, `ja`) must follow objective, concise, formal written expression aligned with `GLOSSARY.md`. Avoid marketing hype, cheerleading, exclamation marks, or colloquial spoken phrasing.
- **Frequency-Based Copy Folding**: Match explanation visibility to user frequency instead of showing permanent walls of text:
  - **Always visible**: Page/section titles, field labels, active scientific/operational key-value state, irreversible-action consequences, and active error cause + recovery action.
  - **Tooltip (hover + keyboard focus)**: Brief, text-only hints for low-frequency icons, abbreviations, disabled reasons, or truncated full values.
  - **Info Popover (`ⓘ`) or Collapsible Disclosure**: Multi-line parameter definitions, delivery-mode explanations, format details, or attribution notes that regular users inspect only occasionally.
  - **Contextual Alert**: Shown **only** when an actionable runtime condition triggers (missing credentials, search-only dataset, quota/path conflict, or destructive risk); never use permanent alert banners to explain ordinary interface operation.
- **Actionable Empty States**: Distinguish "not searched yet", "0 matches under current filters", and "setup/credentials required" in one concise factual sentence plus a direct next action, without decorative marketing filler.
- **Scientific Trust Guardrail**: Trimming redundant UX copy must **never** remove units, resolution, orbit/polarization provenance, delivery-mode distinctions, destructive scope, or concrete failure diagnostics.

## Forms And Controls

- Use controls that match the data: checkboxes or toggles for binary settings, segmented controls for small modes, selects or menus for option sets, and numeric inputs for numeric ranges.
- Labels remain visible or programmatically associated; placeholders are examples or format hints, not the only label.
- Dependent fields clear stale values while new options load. Loading, empty, and failed states must be distinct.
- Validation explains the invalid field and the correction.
- Destructive commands are separated from primary commands and named by outcome.
- Icon-only buttons require an accessible name and a tooltip when the icon is not universally clear.
- **Sizing vs. Density Separation**: Fixed-height interactive controls (buttons, segmented pills, single-line inputs) must use three canonical size tiers (`sm: 28px`, `md: 32px`, `lg: 36px–40px`) rather than ad-hoc per-screen pixel heights. Variable-height containers (list rows, menu items, settings rows, collapsible sections) must use two vertical density presets (`compact` and `balanced`) controlled by vertical padding and gap.
- **Unified Custom Select Component (`<SpSelect>`) — Ban on Visible Native `<select>`**: Every visible dropdown select across desktop workbenches, panels, modals, maps, and settings views must use the project's custom `<SpSelect>` component (`ui/src/components/ui/SpSelect.tsx`, rounded-sp-sm, concentric trigger/listbox styling, keyboard navigation, theme-aware elevation). Visible raw HTML `<select>` elements are strictly prohibited because native WebView system dropdowns break corner geometry, density, and cross-theme visual unity (an `sr-only` accessibility proxy bound strictly to legacy compatibility is the sole narrow exception, verified by `ui/src/lib/noNativeSelectGuard.test.ts`).
- **Tokenized Chat Entities**: In Agent chat composers and message bubbles, structured domain references (`/skill` commands, `@dataset` sources, `#AOI` regions) should render as compact, interactive inline chips rather than pasting raw multi-line skill prompts into the input box or leaving structured references as plain unstyled text.
- **IME Composition Safety**: Text inputs and chat composers that bind `Enter` to submit must check IME composition state (`isComposing` / keyCode `229`) so confirming Chinese or Japanese candidates never triggers unintended submission or newline insertion.

## Component Semantics And Do / Don't Matrix

Manage UI through reusable primitives in `ui/src/components/ui/` (`Button`, `Card`, `Input`, `Textarea`, `Badge`, `Progress`, `SpSelect`, `ModalOverlay`, `Popconfirm`, `Skeleton`, `ProgressNotification`, `TopCenterToast`, `OptimisticInlineStatus`) before writing feature-local styled wrappers. Consult mature industry semantic conventions (WAI-ARIA APG, Radix UI, Apple HIG, Ant Design) strictly for **semantic selection and interaction contracts** — never import external UI kits or alien aesthetics without `component-adoption-review`.

| Scenario | Do (Canonical Semantic Choice) | Don't (Anti-Pattern To Avoid Or Remediate) |
| --- | --- | --- |
| **Action hierarchy** | Use `Primary` (at most one main CTA per visual region), `Secondary` / `Tertiary` / `Ghost` for supporting actions, `Destructive` separated and named by outcome, and semantic `<button>` / `<a>`. | Do not use clickable `<div>` for actions, stack multiple competing carbon-black primary buttons in one bar, or place destructive actions directly next to frequent safe actions. |
| **Binary settings** | Use `Switch` for immediate-effect on/off settings; use `Checkbox` for multi-item selection or form-submitted booleans. | Do not use a `Switch` for multi-select rows or unsubmitted form steps where toggling appears to apply immediately when it does not. |
| **Exclusive choice** | Use `Segmented Control` for 2–4 compact, always-visible modes; use `<SpSelect>` for ≥4 static options; use searchable combobox/palette for large dynamic sets (AOI, models, commands). | Do not render visible native `<select>`, use a dropdown for 2 simple modes, or cram 6+ wrapping choices into a single pill track. |
| **Short hints & help** | Use `Tooltip` (text-only, hover + focus) for icon labels, truncated IDs, or disabled reasons; use click-opened `Popover` if the hint includes links, copy actions, or multi-line guidance. | Do not put clickable buttons inside a hover-only `Tooltip` or print low-frequency explanations as permanent paragraph text. |
| **Confirmations** | Do reversible actions immediately with Undo where safe; use `Popconfirm` anchored to the trigger for single-item irreversible local actions; use `Modal Dialog` for bulk, high-impact, or decision-bearing confirmations. | Do not use a `Drawer` to confirm actions, open a full `Modal` for low-risk reversible toggles, or use vague "OK / Confirm" button labels. |
| **Forms & detail views** | Use `Modal Dialog` for short self-contained forms/decisions; use `Drawer` for long forms or inspection that needs the page visible alongside (e.g., download tasks, scene details); use a `Full Page` for primary workflows. | Do not trap long multi-section editors in tiny modals or split one workflow state into competing side-by-side panels. |
| **Feedback & progress** | Use inline status for local saves/errors; `TopCenterToast` for brief off-eye confirmations (copy, sync); `ProgressNotification` for cross-page long work; contextual `Alert` for active blocking conditions. | Do not use auto-dismissing toasts for errors the user must fix, or stack multiple competing loaders for one wait. |
| **Layout containers** | Use whitespace, headings, and subtle dividers for page sections; reserve `Card` for repeated collection items, modals, and framed tools. | Do not nest cards inside cards or wrap an entire settings/workbench page in a decorative outer card. |

**Scoped Remediation**: Replace semantically mismatched components incrementally when modifying an affected workflow, preserving scientific state ownership and regression tests rather than triggering an unscoped global rewrite.

## Tables, Lists, And Scientific Metadata

- Product lists prioritize stable scanning, comparison, selection, and repeated actions.
- Keep identifiers, acquisition time, path, frame, orbit direction, polarization, status, and other relevant scientific fields available when the provider supplies them.
- Use virtualization or equivalent techniques for large collections without changing selection truth.
- Row selection, task state, and download state must not be visually or behaviorally conflated.
- Counts are derived from the current state source and update with state transitions.
- Empty values use explicit state language instead of guessed substitutions.

Expose Collection, Item and Asset information at their appropriate levels. A
selected band's format, native resolution, unit, scale/offset and quality role
belong in the asset inspector, not in a provider-specific download page. Grouping
by scientific track/frame/time or comparing candidates must preserve exact item
selection rather than replace it with a presentation-only group identifier.

## Maps And AOIs

- The map is an operational inspection surface, not decoration.
- AOI geometry used for search or topology must preserve its true geometry; a display summary may be shorter but must not replace the stored geometry.
- Imported, administrative, and drawn AOIs need clear identity and lifecycle.
- Fit the map to a newly bound AOI without losing user control afterward.
- Thematic map colors do not inherit success, warning, or failure meaning unless a visible legend says so.
- Map controls, attribution, tooltips, and overlays must remain usable at the current window constraint.

Use map-first inspection with a shared layer lifecycle: source/item binding,
visibility, ordering, opacity, loading, error, cancellation and removal. Basemaps,
footprints and raster previews are separate layers, not separate product stores.
Adding a layer or receiving TileJSON acknowledges a request; "loaded" requires
actual successful image/tile loading. Switching product/context invalidates late
events without changing download selection.

Preview rendering and scientific delivery use different controls and labels.
Color stretching, overviews and display resampling may improve a preview but
cannot silently modify a downloaded band, categorical mask or DEM. Show the
chosen delivery explicitly: original archive, assets/bands, scientific subset,
or rendered picture. An unavailable archive does not explain away other actions
whose capabilities can be supported independently.

## Tasks And Long-Running Work

- A command that starts search, download, conversion, or preparation receives immediate visible acknowledgement.
- Progress is determinate only when backed by a meaningful total; otherwise use an honest indeterminate state.
- Show durable task identity, current state, useful counts, output location, failure reason, and next action.
- Pause, resume, retry, cancel, delete, and open-output commands must map to their real task transitions.
- Queue cards and task snapshots may expose different selection scope, but controls must state that scope and use the same underlying task truth.
- Do not animate frequent progress updates in a way that makes values harder to read.

## Status, Errors, And Logs

- Status never relies on color alone; pair color with text, icon, or shape.
- **Actor-Driven State Taxonomy & Single Visual Per State**: Group UI state indicators by the actor driving the transition — **User** (hover, focus, selected, pressed), **System** (loading, background transfer, syncing, queued), and **AI Agent** (thinking, tool execution, awaiting confirmation) — and reuse **one canonical visual indicator per state** across the entire workbench. Never stack competing animations (such as spinner + text shimmer + pulsing border + wave dots) on the same state.
- Unknown, not applicable, not fetched, unavailable, queued, running, paused, failed, cancelled, skipped, and completed remain distinguishable.
- Error messages start with the affected operation, retain a concise technical cause, and offer a practical next action when available.
- Logs are concise, chronological, selectable, and able to display complete details without overlapping neighboring content.
- Success is shown only after state or output verification.

## Icons

- Lucide is the normal action icon system.
- Use familiar symbols for familiar commands and text only when meaning would otherwise be ambiguous.
- Do not mix icon families for ordinary actions.
- Brand marks may use their official asset after a focused licensing, accessibility, and visual-fit review.
- Icons must remain legible at their rendered size and must not be cropped by control bounds.
- **Monochrome Agent Timeline Tool Icons**: Tool execution steps inside Agent thinking and process timelines use bare monochrome stroke icons (`#52525b` on light surfaces, `#a1a1aa` on dark surfaces) with no colored tints or colored background boxes, distinguishing tools strictly by icon silhouette.
- **No Generic AI Sparkle Icon (`Sparkles`)**: Never use Lucide `Sparkles` (or `✨` / `✦` star clichés) or blue `text-sp-accent` / `text-primary` tints on decorative icons; use semantic, monochrome stroke icons (`Tag`, `MessageSquare`, `Cloud`, `Cpu`, `SlidersHorizontal`, `Plug`, `Info`, etc.) in neutral ink (`text-sp-text-secondary` / `text-sp-text-muted`).

## Motion

- Motion communicates spatial relationship, causality, or state change; it is not ambient decoration.
- Use canonical easing curves in CSS tokens without external motion runtime dependencies:
  - `--sp-ease-out-expo: cubic-bezier(0.16, 1, 0.3, 1)` for anchored popovers, drawers, and chip entrances (`150ms–180ms` micro-chip & directional popover entrance without text blur so CJK typography remains razor-sharp).
  - `--sp-ease-spring: cubic-bezier(0.22, 1, 0.36, 1)` for fluid highlights and tactile release (`120ms` press compression & spring release).
  - `--sp-ease: cubic-bezier(0.2, 0, 0, 1)` for standard color, border, and panel transitions (`200ms–240ms`).
- **Unified Spring Expansion for Popovers, Pills & Floating Mini-Cards**: Anchored dropdowns, expandable status pills, and floating task progress cards (such as titlebar popovers and the Agent top-right task checklist card) must use the shared zero-dependency 4-substep 1D spring engine (`ui/src/lib/springDropdownMotion.ts`: open `k=360, c=24`, pill-to-card width/height `k=340, c=21` / `k=280, c=23`, close `k=420, c=30`, fade `k=420, c=32`, blur-free `45ms` row stagger, anchored `transform-origin`, and `0%` idle CPU once settled) instead of abrupt conditional DOM swaps or linear jumps.
- **Anti-Flicker Animation Persistence & Live-Transition-Only Triggers**:
  - Collapsible headers hosting continuous animations (such as soft character wave `sp-ai-char-wave` or live elapsed timers) must preserve a stable React wrapper and `key` across collapsed and expanded states and anchor periodic wave phase to wall-clock time so expanding or collapsing never unmounts or resets the animation.
  - One-shot completion animations (such as checkmark stroke draw `sp-check-draw`) must trigger strictly on live `running/pending -> done` state transitions, never when mounting already-completed rows during card or timeline expansion.
- Keep frequent controls and progress updates restrained.
- Do not delay a command or hide state truth behind an animation.
- Focus, keyboard order, cancellation, and pointer behavior remain correct during overlays and transitions.
- Respect `@media (prefers-reduced-motion: reduce)` with full parity across every entrance, press, highlight, and progress animation. Essential state changes remain understandable with motion removed.
- Adding a motion library requires component adoption review and explicit justification.

## Loading And Waiting States (Blur-Free CJK Rules)

- **300ms Delay / 500ms Minimum Hold**: Sub-`300ms` actions render no separate spinner, skeleton, or progress card so fast local responses never flicker. Once a delayed indicator appears after `300ms`, hold it at least `500ms`. Keep button labels and widths locked while busy.
- **Full-Turn Action Icons**: Buttons whose own Lucide icon already represents the action (such as `RefreshCw`) rotate immediately on press with smooth accel/decel (`>=640ms` per turn) and stop strictly on integer `360deg` turns.
- **Layout-Matched Skeletons**: When initial content layout is predictable (search results, history sessions, usage metrics, fetched model lists), render non-data chrome immediately, show layout-matched `Skeleton` rows (`rgb(240 240 242)` light / `rgba(255,255,255,0.08)` dark, `6px` line radius, single synchronized `1.2s` `320px` list-wide sweep band) after `300ms` instead of a lone centered spinner or fake `0`/empty flash, and keep existing known data visible during refreshes.
- **Counted Progress & Cross-Page Notification**: Counted long operations (`>3s`) display `"估算中…"` during the first second and estimated time remaining once `>=1s` of rate data exists; uncountable long waits name the current stage and surface `>10s` reassurance with background/cancel continuation. Project cross-page long work from existing single-source stores into a `344px` (`24px` radius) blur-free top-right notification after `300–500ms` that morphs in place to a solid-green check on completion; surface quick off-eye confirmations (like map coordinate copy) via a `40px` (`16px` radius) blur-free top-center toast.
- **Optimistic Reversible Edits & Rollback**: Low-risk reversible edits (session organize, skill/plugin toggles, saved-channel enables) update immediately with a quiet inline `保存中 → 已保存` status and zero silent `.catch(() => {})` failures. Failures shake `6px` (`360ms`, 2 cycles), restore prior state, preserve typed input, and offer inline Retry.
- **AI Streamed Steps & Accessibility**: Tool/thinking steps show explicit `16px` spinner-to-check transitions with one active run indicator; streamed prose keeps a downward-only `min-height` and a `2px` `--sp-accent-fill` trailing caret without blur-fading small CJK text. Loading regions expose `aria-busy`, polite `aria-live` status updates, and full `prefers-reduced-motion: reduce` parity.

## Accessibility

- Preserve visible keyboard focus and logical tab order.
- Controls have accessible names; labels and error text are associated with their fields.
- Maintain readable contrast in light and dark themes.
- Do not require hover, color, animation, or precise pointer movement to complete a core workflow.
- Test Simplified Chinese, English, and Japanese labels, scientific identifiers, long unbroken strings, zoom, and current window constraints.
- A visible language change updates the document language immediately. Inspect English and Japanese long copy at 920 x 620 and with the sidebar collapsed; text must remain readable or inspectable without overlap.
- Japanese uses Japanese system font faces ahead of Chinese faces. Do not distribute a new font solely for language support.

## Modals And Overlays

- **Modal Geometry, Fixed Outer Bounds & Backdrop Mask Law**:
  - Every modal dialog, detail inspector, confirmation dialog, or editing overlay MUST use the canonical modal radius token (`--radius-sp-modal` / `rounded-sp-modal` / `rounded-modal`, `14px`; existing `16px` full-frame inspector shells remain tracked visual debt to converge under a scoped pass) and crisp border (`border border-sp-line`) over a high-contrast dim backdrop (`bg-black/45 dark:bg-black/65 backdrop-blur-[2px]`) via `ModalOverlay` so modals stay clearly separated from zinc canvas/card surfaces (`--sp-canvas` / `--sp-card`).
  - Any modal, dialog, or drawer where the user edits text, configures provider/model parameters, searches/filters a catalog, or reviews a multi-step confirmation **MUST NOT close when clicking the backdrop or outside area** (`closeOnBackdropClick={false}`). Dismissal uses explicit Cancel/Close actions or intentional `Escape` when unmodified. Lightweight read-only popovers or action menus may close on outside click and `Escape`.
  - Multi-tab or file-tree inspector modals (such as Skill Package Inspector, Global Command Palette, and Catalog/Plugins modals) MUST lock their outer width and height (`w-[min(...)] h-[min(...)]`, `flex flex-col overflow-hidden`) and scroll internally so switching tabs, selecting files, or showing empty states produces `0px` outer width/height jitter.
- **Zero Orphan-Wrap & Blind-Truncation Law**: Never allow 1-2 orphan CJK or Latin characters to wrap awkwardly onto an extra line, and never blindly `...`-truncate critical labels, metadata pills, or headings at minimum panel or card width: tighten copy to fit cleanly on one line (`whitespace-nowrap shrink-0` for action/badge clusters) or structure content into an intentional, balanced two-line hierarchy.

## Known Visual Debt

The inspected UI still contains legacy translucent glass/gradients on `#legacy` surfaces, incomplete reduced-motion/accessible naming on older controls, and token-hygiene drift outside Settings and Download Center (such as ad-hoc `rounded-[...]` radii, `14px` `--radius-sp-modal` vs. `16px` inspector-modal divergence, mixed half-step spacing like `gap-1.5`/`px-2.5`, and feature-local button/card classes duplicating `ui/src/components/ui/` primitives). These are explicit audit targets, not permission for an unscoped global rewrite.

Any cleanup must begin with an evidence-based audit, identify affected workflows, and land in small reversible changes with runtime inspection.

## Anti-Patterns

Do not introduce:

- a marketing hero as the primary application screen;
- duplicate workbenches or result panels for the same state source;
- cards inside cards or decorative cards around whole page sections;
- invented CSS tokens that are absent from code, or arbitrary pixel radii/spacing bypassing the global `--radius-sp-*` and `4px` scales;
- semantically mismatched components (e.g., clickable `<div>` actions, visible native `<select>`, `Drawer` for confirmation, or auto-dismissing `Toast` for blocking errors);
- status-only color, fake progress, fake success, or guessed scientific values;
- a second general icon system or UI kit for a single component;
- broad gradients, glass, animation, or radius changes without an approved audit scope;
- permanent instructional walls of text, colloquial/marketing UX copy, or copy that merely narrates obvious controls.

## Design Acceptance

- Scientific fields and state distinctions remain intact.
- The change uses existing state ownership, canonical `ui/src/components/ui/` primitives, and semantically appropriate components per the Do / Don't matrix.
- Styling uses implemented `--sp-*` / `--radius-sp-*` tokens, concentric corner geometry, `4px`-grid semantic spacing, and symmetric container insets.
- Copy matches the page's positioning in concise formal written language, with low-frequency explanations folded into tooltips, popovers, or contextual alerts.
- Layout remains stable at the current documented window constraint.
- Long text is readable or inspectable without overlap, and dynamic numeric values use `tabular-nums`.
- Keyboard focus, IME safety, accessible names, contrast, and reduced motion are addressed.
- Loading, empty, error, disabled, paused, and completed states are covered.
- New dependencies and external components have passed adoption review.
- Runtime inspection confirms the intended interface rather than relying only on static code review.

## Governance Update Triggers

Review this file when the project:

- changes UI framework, map engine, icon system, or styling architecture;
- implements a new token or removes an existing canonical token;
- changes the primary workbench structure or documented minimum window;
- adds a materially different data visualization or interaction model;
- introduces a motion library, component kit, or second desktop platform;
- resolves visual debt in a way that establishes a new reusable pattern.
