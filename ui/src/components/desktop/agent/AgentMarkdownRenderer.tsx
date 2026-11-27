/**
 * AgentMarkdownRenderer.tsx
 * Zero-dependency safe tokenizer/AST Markdown renderer for Agent 2.0 prose, conclusions, and cards.
 * Supports headings, **bold**, *italic*, inline `code`, ordered/unordered lists, blockquotes,
 * GFM pipe tables, and fenced code blocks with language badge + copy feedback.
 * Raw `<think>` / `<thinking>` blocks are defensively quarantined and never rendered in prose.
 */

import React, { useCallback, useEffect, useRef, useState } from "react";
import { Check, Copy, Download, Maximize2, WrapText, X } from "lucide-react";
import { cn } from "@/lib/utils";
import { useI18n } from "@/i18n";
import { ModalOverlay } from "@/components/ui/ModalOverlay";
import {
  cleanCopyableMarkdown,
  splitInlineThink,
  type SplitInlineThinkResult,
} from "@/shell/api/agentThinkSplitLogic";

export { splitInlineThink, cleanCopyableMarkdown };
export type { SplitInlineThinkResult };

export interface AgentMarkdownRendererProps {
  content: string;
  className?: string;
  compact?: boolean;
  streaming?: boolean;
}

type TableAlignment = "left" | "center" | "right";

type BlockNode =
  | { type: "heading"; level: 1 | 2 | 3 | 4; text: string }
  | { type: "code_block"; language: string; code: string }
  | { type: "math_block"; latex: string }
  | { type: "blockquote"; lines: string[] }
  | { type: "ul"; items: string[] }
  | { type: "ol"; start: number; items: string[] }
  | { type: "table"; headers: string[]; alignments: TableAlignment[]; rows: string[][] }
  | { type: "hr" }
  | { type: "paragraph"; lines: string[] };

type InlineNode =
  | { type: "text"; value: string }
  | { type: "code"; value: string }
  | { type: "math_inline"; latex: string }
  | { type: "math_block"; latex: string }
  | { type: "bold"; children: InlineNode[] }
  | { type: "italic"; children: InlineNode[] }
  | { type: "link"; label: InlineNode[]; href: string };

const LATEX_SYMBOLS: Record<string, string> = {
  // Lowercase Greek
  alpha: "α",
  beta: "β",
  gamma: "γ",
  delta: "δ",
  epsilon: "ϵ",
  varepsilon: "ε",
  zeta: "ζ",
  eta: "η",
  theta: "θ",
  vartheta: "ϑ",
  iota: "ι",
  kappa: "κ",
  lambda: "λ",
  mu: "μ",
  nu: "ν",
  xi: "ξ",
  pi: "π",
  varpi: "ϖ",
  rho: "ρ",
  varrho: "ϱ",
  sigma: "σ",
  varsigma: "ς",
  tau: "τ",
  upsilon: "υ",
  phi: "ϕ",
  varphi: "φ",
  chi: "χ",
  psi: "ψ",
  omega: "ω",
  // Uppercase Greek
  Gamma: "Γ",
  Delta: "Δ",
  Theta: "Θ",
  Lambda: "Λ",
  Xi: "Ξ",
  Pi: "Π",
  Sigma: "Σ",
  Upsilon: "Υ",
  Phi: "Φ",
  Psi: "Ψ",
  Omega: "Ω",
  // Operators & Relations
  times: "×",
  cdot: "·",
  cdots: "⋯",
  ldots: "…",
  vdots: "⋮",
  ddots: "⋱",
  div: "÷",
  pm: "±",
  mp: "∓",
  le: "≤",
  leq: "≤",
  ge: "≥",
  geq: "≥",
  neq: "≠",
  ne: "≠",
  approx: "≈",
  sim: "∼",
  simeq: "≃",
  cong: "≅",
  equiv: "≡",
  propto: "∝",
  ll: "≪",
  gg: "≫",
  subset: "⊂",
  supset: "⊃",
  subseteq: "⊆",
  supseteq: "⊇",
  in: "∈",
  notin: "∉",
  ni: "∋",
  cup: "∪",
  cap: "∩",
  setminus: "∖",
  land: "∧",
  lor: "∨",
  neg: "¬",
  forall: "∀",
  exists: "∃",
  nexists: "∄",
  emptyset: "∅",
  varnothing: "∅",
  infty: "∞",
  partial: "∂",
  nabla: "∇",
  perp: "⊥",
  parallel: "∥",
  angle: "∠",
  triangle: "△",
  circ: "∘",
  degree: "°",
  prime: "′",
  ell: "ℓ",
  hbar: "ℏ",
  Re: "ℜ",
  Im: "ℑ",
  aleph: "ℵ",
  wp: "℘",
  oplus: "⊕",
  otimes: "⊗",
  odot: "⊙",
  star: "⋆",
  ast: "∗",
  bullet: "•",
  dagger: "†",
  ddagger: "‡",
  // Big operators
  sum: "∑",
  prod: "∏",
  coprod: "∐",
  int: "∫",
  iint: "∬",
  iiint: "∭",
  oint: "∮",
  bigcup: "⋃",
  bigcap: "⋂",
  // Arrows
  to: "→",
  rightarrow: "→",
  leftarrow: "←",
  leftrightarrow: "↔",
  Rightarrow: "⇒",
  Leftarrow: "⇐",
  Leftrightarrow: "⇔",
  iff: "⇔",
  mapsto: "↦",
  uparrow: "↑",
  downarrow: "↓",
  nearrow: "↗",
  searrow: "↘",
  longrightarrow: "⟶",
  longleftarrow: "⟵",
  Longrightarrow: "⟹",
  Longleftrightarrow: "⟺",
  // Delimiters
  langle: "⟨",
  rangle: "⟩",
  lceil: "⌈",
  rceil: "⌉",
  lfloor: "⌊",
  rfloor: "⌋",
  lvert: "|",
  rvert: "|",
  lVert: "‖",
  rVert: "‖",
  vert: "|",
  Vert: "‖",
  mid: "∣",
};

const LATEX_FUNCTIONS = new Set([
  "sin",
  "cos",
  "tan",
  "cot",
  "sec",
  "csc",
  "arcsin",
  "arccos",
  "arctan",
  "sinh",
  "cosh",
  "tanh",
  "log",
  "ln",
  "lg",
  "exp",
  "lim",
  "sup",
  "inf",
  "max",
  "min",
  "arg",
  "det",
  "dim",
  "gcd",
  "hom",
  "ker",
  "Pr",
  "deg",
  "tr",
  "rank",
  "cov",
  "var",
  "mod",
]);

function readBracedGroup(src: string, startIdx: number): { content: string; nextIdx: number } | null {
  let i = startIdx;
  while (i < src.length && /\s/.test(src[i])) i++;
  if (i >= src.length || src[i] !== "{") return null;
  let depth = 0;
  const contentStart = i + 1;
  while (i < src.length) {
    if (src[i] === "\\" && i + 1 < src.length && (src[i + 1] === "{" || src[i + 1] === "}")) {
      i += 2;
      continue;
    }
    if (src[i] === "{") depth++;
    else if (src[i] === "}") {
      depth--;
      if (depth === 0) {
        return {
          content: src.slice(contentStart, i),
          nextIdx: i + 1,
        };
      }
    }
    i++;
  }
  return null;
}

function readBracketGroup(src: string, startIdx: number): { content: string; nextIdx: number } | null {
  let i = startIdx;
  while (i < src.length && /\s/.test(src[i])) i++;
  if (i >= src.length || src[i] !== "[") return null;
  let depth = 0;
  const contentStart = i + 1;
  while (i < src.length) {
    if (src[i] === "[") depth++;
    else if (src[i] === "]") {
      depth--;
      if (depth === 0) {
        return {
          content: src.slice(contentStart, i),
          nextIdx: i + 1,
        };
      }
    }
    i++;
  }
  return null;
}

function readMathAtom(src: string, startIdx: number): { content: string; nextIdx: number } {
  let i = startIdx;
  while (i < src.length && /\s/.test(src[i])) i++;
  if (i >= src.length) return { content: "", nextIdx: i };
  if (src[i] === "{") {
    const grp = readBracedGroup(src, i);
    if (grp) return grp;
  }
  if (src[i] === "\\") {
    let j = i + 1;
    while (j < src.length && /[A-Za-z]/.test(src[j])) j++;
    if (j === i + 1 && j < src.length) j++;
    return { content: src.slice(i, j), nextIdx: j };
  }
  return { content: src[i], nextIdx: i + 1 };
}

function splitLatexMatrixRows(body: string): string[][] {
  const rows: string[] = [];
  let cur = "";
  let braceDepth = 0;
  let envDepth = 0;
  let i = 0;
  while (i < body.length) {
    if (body.startsWith("\\begin{", i)) {
      envDepth++;
    } else if (body.startsWith("\\end{", i) && envDepth > 0) {
      envDepth--;
    }
    if (body[i] === "{" && (i === 0 || body[i - 1] !== "\\")) {
      braceDepth++;
    } else if (body[i] === "}" && (i === 0 || body[i - 1] !== "\\") && braceDepth > 0) {
      braceDepth--;
    }
    if (
      braceDepth === 0 &&
      envDepth === 0 &&
      body[i] === "\\" &&
      i + 1 < body.length &&
      body[i + 1] === "\\"
    ) {
      rows.push(cur.trim());
      cur = "";
      i += 2;
      // Optional row spacing like \\[4pt]
      if (i < body.length && body[i] === "[") {
        const opt = readBracketGroup(body, i);
        if (opt) i = opt.nextIdx;
      }
      continue;
    }
    cur += body[i];
    i++;
  }
  if (cur.trim().length > 0 || rows.length === 0) {
    rows.push(cur.trim());
  }
  return rows.map((row) => {
    const cells: string[] = [];
    let cellCur = "";
    let d = 0;
    let e = 0;
    for (let k = 0; k < row.length; k++) {
      if (row.startsWith("\\begin{", k)) e++;
      else if (row.startsWith("\\end{", k) && e > 0) e--;
      if (row[k] === "{" && (k === 0 || row[k - 1] !== "\\")) d++;
      else if (row[k] === "}" && (k === 0 || row[k - 1] !== "\\") && d > 0) d--;
      if (d === 0 && e === 0 && row[k] === "&" && (k === 0 || row[k - 1] !== "\\")) {
        cells.push(cellCur.trim());
        cellCur = "";
      } else {
        cellCur += row[k];
      }
    }
    cells.push(cellCur.trim());
    return cells;
  });
}

function renderLatexNodes(latex: string, keyPrefix: string): React.ReactNode[] {
  const out: React.ReactNode[] = [];
  let i = 0;
  const len = latex.length;
  let textBuf = "";
  let nodeSeq = 0;

  const flushText = () => {
    if (textBuf.length > 0) {
      // Normalize basic math operators in plain segments (- -> − when flanked by spaces/operands)
      const formatted = textBuf.replace(/\s-\s/g, " − ");
      out.push(<React.Fragment key={`${keyPrefix}-t-${nodeSeq++}`}>{formatted}</React.Fragment>);
      textBuf = "";
    }
  };

  while (i < len) {
    const ch = latex[i];

    // Grouping braces {...}
    if (ch === "{") {
      const grp = readBracedGroup(latex, i);
      if (grp) {
        flushText();
        out.push(
          <React.Fragment key={`${keyPrefix}-g-${nodeSeq++}`}>
            {renderLatexNodes(grp.content, `${keyPrefix}-g-${nodeSeq}`)}
          </React.Fragment>
        );
        i = grp.nextIdx;
        continue;
      }
    }

    // Superscript ^ or Subscript _
    if (ch === "^" || ch === "_") {
      const isSup = ch === "^";
      const atom = readMathAtom(latex, i + 1);
      if (atom.content.length > 0) {
        flushText();
        const k = `${keyPrefix}-${isSup ? "sup" : "sub"}-${nodeSeq++}`;
        const children = renderLatexNodes(atom.content, k);
        out.push(
          isSup ? (
            <sup key={k} className="text-[0.75em] leading-none align-super ml-[0.5px]">
              {children}
            </sup>
          ) : (
            <sub key={k} className="text-[0.75em] leading-none align-baseline ml-[0.5px]">
              {children}
            </sub>
          )
        );
        i = atom.nextIdx;
        continue;
      }
    }

    // Backslash command
    if (ch === "\\") {
      if (i + 1 >= len) {
        i++;
        continue;
      }
      const nextCh = latex[i + 1];

      // Escaped punctuation or spacing: \{, \}, \%, \_, \&, \#, \$, \|, \,, \;, \:, \!, \ , \\
      if (!/[A-Za-z]/.test(nextCh)) {
        if (nextCh === "{" || nextCh === "}" || nextCh === "%" || nextCh === "_" || nextCh === "&" || nextCh === "#" || nextCh === "$") {
          textBuf += nextCh;
          i += 2;
          continue;
        }
        if (nextCh === "|") {
          textBuf += "‖";
          i += 2;
          continue;
        }
        if (nextCh === "," || nextCh === ";" || nextCh === ":" || nextCh === " ") {
          textBuf += "\u2009";
          i += 2;
          continue;
        }
        if (nextCh === "!") {
          i += 2;
          continue;
        }
        if (nextCh === "\\") {
          flushText();
          out.push(<br key={`${keyPrefix}-br-${nodeSeq++}`} />);
          i += 2;
          continue;
        }
        textBuf += nextCh;
        i += 2;
        continue;
      }

      // Read alphabetic command name
      let cmdEnd = i + 1;
      while (cmdEnd < len && /[A-Za-z]/.test(latex[cmdEnd])) cmdEnd++;
      const cmd = latex.slice(i + 1, cmdEnd);

      // Environment: \begin{env}...\end{env}
      if (cmd === "begin") {
        const envGrp = readBracedGroup(latex, cmdEnd);
        if (envGrp) {
          const envName = envGrp.content.trim();
          const endTag = `\\end{${envName}}`;
          const closeIdx = latex.indexOf(endTag, envGrp.nextIdx);
          if (closeIdx !== -1) {
            flushText();
            const body = latex.slice(envGrp.nextIdx, closeIdx);
            const rows = splitLatexMatrixRows(body);
            const k = `${keyPrefix}-env-${nodeSeq++}`;
            const leftDelim =
              envName === "bmatrix"
                ? "["
                : envName === "pmatrix"
                  ? "("
                  : envName === "Bmatrix" || envName === "cases"
                    ? "{"
                    : envName === "vmatrix"
                      ? "|"
                      : envName === "Vmatrix"
                        ? "‖"
                        : "";
            const rightDelim =
              envName === "bmatrix"
                ? "]"
                : envName === "pmatrix"
                  ? ")"
                  : envName === "Bmatrix"
                    ? "}"
                    : envName === "vmatrix"
                      ? "|"
                      : envName === "Vmatrix"
                        ? "‖"
                        : "";

            out.push(
              <span
                key={k}
                data-testid="agent-math-matrix"
                className="inline-flex items-center align-middle mx-1 my-0.5"
              >
                {leftDelim && (
                  <span className="text-[1.25em] font-light leading-none select-none pr-0.5 text-sp-text">
                    {leftDelim}
                  </span>
                )}
                <span className="inline-table border-collapse align-middle">
                  <span className="table-row-group">
                    {rows.map((r, rIdx) => (
                      <span key={`${k}-r-${rIdx}`} className="table-row">
                        {r.map((cell, cIdx) => (
                          <span
                            key={`${k}-c-${rIdx}-${cIdx}`}
                            className={cn(
                              "table-cell px-1.5 py-0.5 align-middle",
                              envName === "cases" || envName === "aligned" || envName === "align"
                                ? "text-left"
                                : "text-center"
                            )}
                          >
                            {renderLatexNodes(cell, `${k}-c-${rIdx}-${cIdx}`)}
                          </span>
                        ))}
                      </span>
                    ))}
                  </span>
                </span>
                {rightDelim && (
                  <span className="text-[1.25em] font-light leading-none select-none pl-0.5 text-sp-text">
                    {rightDelim}
                  </span>
                )}
              </span>
            );
            i = closeIdx + endTag.length;
            continue;
          }
        }
      }

      // Fraction: \frac{num}{den}, \dfrac{num}{den}, \tfrac{num}{den}
      if (cmd === "frac" || cmd === "dfrac" || cmd === "tfrac") {
        const numGrp = readBracedGroup(latex, cmdEnd);
        if (numGrp) {
          const denGrp = readBracedGroup(latex, numGrp.nextIdx);
          if (denGrp) {
            flushText();
            const k = `${keyPrefix}-frac-${nodeSeq++}`;
            out.push(
              <span
                key={k}
                data-testid="agent-math-frac"
                className="inline-flex flex-col items-center align-middle mx-1 leading-tight"
              >
                <span className="border-b border-current px-1 pb-[1px] text-[0.9em] text-center">
                  {renderLatexNodes(numGrp.content, `${k}-n`)}
                </span>
                <span className="px-1 pt-[1px] text-[0.9em] text-center">
                  {renderLatexNodes(denGrp.content, `${k}-d`)}
                </span>
              </span>
            );
            i = denGrp.nextIdx;
            continue;
          }
        }
      }

      // Square / N-th root: \sqrt[n]{x} or \sqrt{x}
      if (cmd === "sqrt") {
        let afterCmd = cmdEnd;
        const optRoot = readBracketGroup(latex, afterCmd);
        if (optRoot) afterCmd = optRoot.nextIdx;
        const radAtom = readMathAtom(latex, afterCmd);
        if (radAtom.content.length > 0) {
          flushText();
          const k = `${keyPrefix}-sqrt-${nodeSeq++}`;
          out.push(
            <span
              key={k}
              data-testid="agent-math-sqrt"
              className="inline-flex items-baseline align-baseline mx-0.5"
            >
              {optRoot ? (
                <sup className="text-[0.65em] leading-none -mr-0.5">
                  {renderLatexNodes(optRoot.content, `${k}-rt`)}
                </sup>
              ) : null}
              <span className="text-[1.05em] leading-none select-none">√</span>
              <span className="border-t border-current px-0.5 pt-[1px]">
                {renderLatexNodes(radAtom.content, `${k}-rad`)}
              </span>
            </span>
          );
          i = radAtom.nextIdx;
          continue;
        }
      }

      // Text / font style commands: \text{...}, \mathrm{...}, \mathbf{...}, etc.
      if (
        cmd === "text" ||
        cmd === "textrm" ||
        cmd === "mathrm" ||
        cmd === "operatorname" ||
        cmd === "mathbf" ||
        cmd === "boldsymbol" ||
        cmd === "bm" ||
        cmd === "mathit" ||
        cmd === "mathcal" ||
        cmd === "mathbb" ||
        cmd === "mathtt"
      ) {
        const grp = readBracedGroup(latex, cmdEnd);
        if (grp) {
          flushText();
          const k = `${keyPrefix}-fn-${nodeSeq++}`;
          const isBold = cmd === "mathbf" || cmd === "boldsymbol" || cmd === "bm";
          const isItalic = cmd === "mathit";
          const isMono = cmd === "mathtt";
          out.push(
            <span
              key={k}
              className={cn(
                isBold && "font-semibold",
                isItalic ? "italic" : "not-italic",
                isMono && "font-mono text-[0.92em]"
              )}
            >
              {cmd === "text" || cmd === "textrm"
                ? grp.content
                : renderLatexNodes(grp.content, k)}
            </span>
          );
          i = grp.nextIdx;
          continue;
        }
      }

      // Accents / decorations: \hat, \bar, \overline, \vec, \tilde, \dot, \ddot, \underline
      if (
        cmd === "hat" ||
        cmd === "widehat" ||
        cmd === "bar" ||
        cmd === "overline" ||
        cmd === "vec" ||
        cmd === "overrightarrow" ||
        cmd === "tilde" ||
        cmd === "widetilde" ||
        cmd === "dot" ||
        cmd === "ddot" ||
        cmd === "underline"
      ) {
        const atom = readMathAtom(latex, cmdEnd);
        if (atom.content.length > 0) {
          flushText();
          const k = `${keyPrefix}-acc-${nodeSeq++}`;
          if (cmd === "overline" || cmd === "bar") {
            out.push(
              <span key={k} className="border-t border-current pt-[1px] inline-block leading-none">
                {renderLatexNodes(atom.content, k)}
              </span>
            );
          } else if (cmd === "underline") {
            out.push(
              <span key={k} className="border-b border-current pb-[1px] inline-block leading-none">
                {renderLatexNodes(atom.content, k)}
              </span>
            );
          } else {
            const combining =
              cmd === "hat" || cmd === "widehat"
                ? "\u0302"
                : cmd === "vec" || cmd === "overrightarrow"
                  ? "\u20D7"
                  : cmd === "tilde" || cmd === "widetilde"
                    ? "\u0303"
                    : cmd === "ddot"
                      ? "\u0308"
                      : "\u0307";
            out.push(
              <span key={k} className="inline-block">
                {renderLatexNodes(atom.content, k)}
                {combining}
              </span>
            );
          }
          i = atom.nextIdx;
          continue;
        }
      }

      // Delimiter sizing commands: \left, \right, \big, \Big, \bigg, \Bigg
      if (
        cmd === "left" ||
        cmd === "right" ||
        cmd === "big" ||
        cmd === "Big" ||
        cmd === "bigg" ||
        cmd === "Bigg" ||
        cmd === "bigl" ||
        cmd === "bigr" ||
        cmd === "Bigl" ||
        cmd === "Bigr"
      ) {
        let j = cmdEnd;
        while (j < len && /\s/.test(latex[j])) j++;
        if (j < len && latex[j] === ".") {
          // Invisible delimiter \left. or \right.
          i = j + 1;
          continue;
        }
        i = cmdEnd;
        continue;
      }

      // Display style switches
      if (
        cmd === "displaystyle" ||
        cmd === "textstyle" ||
        cmd === "scriptstyle" ||
        cmd === "scriptscriptstyle" ||
        cmd === "limits" ||
        cmd === "nolimits"
      ) {
        i = cmdEnd;
        continue;
      }

      // Spacing commands
      if (cmd === "quad") {
        textBuf += "\u2003";
        i = cmdEnd;
        continue;
      }
      if (cmd === "qquad") {
        textBuf += "\u2003\u2003";
        i = cmdEnd;
        continue;
      }
      if (cmd === "thinspace" || cmd === "enspace" || cmd === "space") {
        textBuf += "\u2009";
        i = cmdEnd;
        continue;
      }

      // Known Greek letter or math symbol
      if (cmd in LATEX_SYMBOLS) {
        textBuf += LATEX_SYMBOLS[cmd];
        // Eat a single trailing space after a control word unless followed by another control word
        i = cmdEnd;
        if (i < len && latex[i] === " ") i++;
        continue;
      }

      // Standard upright math functions (\sin, \cos, \ln, \log, \max, etc.)
      if (LATEX_FUNCTIONS.has(cmd)) {
        flushText();
        out.push(
          <span key={`${keyPrefix}-op-${nodeSeq++}`} className="not-italic font-normal mr-[1px]">
            {cmd}
          </span>
        );
        i = cmdEnd;
        continue;
      }

      // Unknown command fallback: render command name without raw backslash
      textBuf += cmd;
      i = cmdEnd;
      continue;
    }

    textBuf += ch;
    i++;
  }

  flushText();
  return out;
}

function DisplayMathBlockView({ latex, keyId }: { latex: string; keyId: string }) {
  const [copied, setCopied] = useState(false);
  const timerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const cleaned = latex.trim();
  const nodes = renderLatexNodes(cleaned, keyId);

  useEffect(() => {
    return () => {
      if (timerRef.current) clearTimeout(timerRef.current);
    };
  }, []);

  const handleCopyLatex = useCallback(async () => {
    await writeClipboardSafe(cleaned);
    setCopied(true);
    if (timerRef.current) clearTimeout(timerRef.current);
    timerRef.current = setTimeout(() => setCopied(false), 1800);
  }, [cleaned]);

  return (
    <span
      data-testid="agent-math-block"
      className="group/math relative block my-2 px-3 py-2.5 pr-16 rounded-lg bg-sp-surface-muted/70 border border-sp-line/70 overflow-x-auto text-center font-serif text-[1.02em] leading-relaxed text-sp-text"
    >
      <button
        type="button"
        data-testid="btn-copy-math-latex"
        onClick={() => void handleCopyLatex()}
        title="复制 LaTeX"
        aria-label={copied ? "已复制 LaTeX" : "复制 LaTeX"}
        className="absolute right-2 top-2 inline-flex items-center gap-1 rounded px-1.5 py-0.5 text-xs font-sans text-sp-text-muted bg-sp-surface/85 border border-sp-line/80 hover:text-sp-text hover:bg-sp-hover transition-colors cursor-pointer select-none"
      >
        {copied ? (
          <>
            <Check className="h-3 w-3 text-sp-status-success" />
            <span className="text-sp-success-text">已复制</span>
          </>
        ) : (
          <>
            <Copy className="h-3 w-3" />
            <span>LaTeX</span>
          </>
        )}
      </button>
      <span className="inline-flex items-center justify-center flex-wrap gap-x-0.5">
        {nodes}
      </span>
    </span>
  );
}

function renderLatexFormula(latex: string, displayMode: boolean, key: string): React.ReactNode {
  const cleaned = latex.trim();
  if (displayMode) {
    return <DisplayMathBlockView key={key} latex={cleaned} keyId={key} />;
  }
  const nodes = renderLatexNodes(cleaned, key);
  return (
    <span
      key={key}
      data-testid="agent-math-inline"
      className="inline-flex items-baseline font-serif text-[1.02em] px-0.5 text-sp-text"
    >
      {nodes}
    </span>
  );
}

function splitPipeRow(line: string): string[] {
  let trimmed = line.trim();
  if (trimmed.startsWith("|")) trimmed = trimmed.slice(1);
  if (trimmed.endsWith("|") && !trimmed.endsWith("\\|")) {
    trimmed = trimmed.slice(0, -1);
  }
  const cells: string[] = [];
  let cur = "";
  for (let i = 0; i < trimmed.length; i++) {
    if (trimmed[i] === "\\" && i + 1 < trimmed.length && trimmed[i + 1] === "|") {
      cur += "|";
      i++;
    } else if (trimmed[i] === "|") {
      cells.push(cur.trim());
      cur = "";
    } else {
      cur += trimmed[i];
    }
  }
  cells.push(cur.trim());
  return cells;
}

function parseTableSeparator(line: string, expectedCols: number): TableAlignment[] | null {
  const trimmed = line.trim();
  if (!trimmed.includes("|") && expectedCols > 1) return null;
  const cells = splitPipeRow(trimmed);
  if (cells.length !== expectedCols || cells.length === 0) return null;
  const alignments: TableAlignment[] = [];
  for (const cell of cells) {
    const c = cell.trim();
    if (!/^:?-{3,}:?$/.test(c)) return null;
    if (c.startsWith(":") && c.endsWith(":")) {
      alignments.push("center");
    } else if (c.endsWith(":")) {
      alignments.push("right");
    } else {
      alignments.push("left");
    }
  }
  return alignments;
}

function parseMarkdownBlocks(markdown: string): BlockNode[] {
  const normalized = markdown.replace(/\r\n/g, "\n");
  const lines = normalized.split("\n");
  const blocks: BlockNode[] = [];
  let i = 0;

  while (i < lines.length) {
    const line = lines[i];
    const trimmed = line.trim();

    if (trimmed.length === 0) {
      i++;
      continue;
    }

    // Fenced code block: ```lang or ~~~lang
    const fenceMatch = /^[ \t]*(`{3,}|~{3,})([^\r\n]*)$/.exec(line);
    if (fenceMatch && (!fenceMatch[1].startsWith("`") || !fenceMatch[2].includes("`"))) {
      const fenceChars = fenceMatch[1];
      const fenceChar = fenceChars[0];
      const fenceLen = fenceChars.length;
      const language = fenceMatch[2].trim().split(/\s+/)[0] || "";
      const codeLines: string[] = [];
      i++;
      while (i < lines.length) {
        const closeRe = new RegExp(`^[ \\t]*${fenceChar}{${fenceLen},}[ \\t]*$`);
        if (closeRe.test(lines[i])) {
          i++;
          break;
        }
        codeLines.push(lines[i]);
        i++;
      }
      blocks.push({
        type: "code_block",
        language,
        code: codeLines.join("\n"),
      });
      continue;
    }

    // Display math block: \[ ... \] or $$ ... $$
    if (trimmed.startsWith("\\[") || trimmed.startsWith("$$")) {
      const isDollar = trimmed.startsWith("$$");
      const openMarker = isDollar ? "$$" : "\\[";
      const closeMarker = isDollar ? "$$" : "\\]";
      const afterOpen = trimmed.slice(openMarker.length);
      const sameLineClose = afterOpen.indexOf(closeMarker);
      if (sameLineClose !== -1 && afterOpen.slice(sameLineClose + closeMarker.length).trim().length === 0) {
        blocks.push({
          type: "math_block",
          latex: afterOpen.slice(0, sameLineClose).trim(),
        });
        i++;
        continue;
      }
      if (sameLineClose === -1) {
        const mathLines: string[] = [];
        if (afterOpen.trim().length > 0) {
          mathLines.push(afterOpen);
        }
        i++;
        while (i < lines.length) {
          const curLine = lines[i];
          const closePos = curLine.indexOf(closeMarker);
          if (closePos !== -1) {
            const beforeClose = curLine.slice(0, closePos);
            if (beforeClose.trim().length > 0) {
              mathLines.push(beforeClose);
            }
            i++;
            break;
          }
          mathLines.push(curLine);
          i++;
        }
        blocks.push({
          type: "math_block",
          latex: mathLines.join("\n").trim(),
        });
        continue;
      }
    }

    // Heading: # .. ####
    const headingMatch = /^[ \t]*(#{1,6})\s+(.+?)\s*#*$/.exec(line);
    if (headingMatch) {
      const rawLevel = headingMatch[1].length;
      const level = (Math.min(4, Math.max(1, rawLevel)) as 1 | 2 | 3 | 4);
      blocks.push({
        type: "heading",
        level,
        text: headingMatch[2],
      });
      i++;
      continue;
    }

    // Horizontal rule
    if (/^[ \t]*(?:-\s*-\s*-[-\s]*|\*\s*\*\s*\*[\*\s]*)$/.test(line)) {
      blocks.push({ type: "hr" });
      i++;
      continue;
    }

    // GFM pipe table
    if (
      line.includes("|") &&
      i + 1 < lines.length &&
      lines[i + 1].includes("-") &&
      lines[i + 1].includes("|")
    ) {
      const headers = splitPipeRow(line);
      const alignments = parseTableSeparator(lines[i + 1], headers.length);
      if (alignments) {
        const rows: string[][] = [];
        i += 2;
        while (i < lines.length) {
          const rowLine = lines[i];
          if (rowLine.trim().length === 0 || !rowLine.includes("|")) break;
          if (/^[ \t]*(`{3,}|~{3,})/.test(rowLine)) break;
          const parsedCells = splitPipeRow(rowLine);
          const normalizedRow = headers.map((_, colIdx) => parsedCells[colIdx] ?? "");
          rows.push(normalizedRow);
          i++;
        }
        blocks.push({
          type: "table",
          headers,
          alignments,
          rows,
        });
        continue;
      }
    }

    // Blockquote: > ...
    if (/^[ \t]*>\s?/.test(line)) {
      const quoteLines: string[] = [];
      while (i < lines.length && /^[ \t]*>\s?/.test(lines[i])) {
        quoteLines.push(lines[i].replace(/^[ \t]*>\s?/, ""));
        i++;
      }
      blocks.push({ type: "blockquote", lines: quoteLines });
      continue;
    }

    // Unordered list: - item or * item
    const ulMatch = /^[ \t]*[-*+]\s+(.*)$/.exec(line);
    if (ulMatch) {
      const items: string[] = [];
      while (i < lines.length) {
        const m = /^[ \t]*[-*+]\s+(.*)$/.exec(lines[i]);
        if (!m) break;
        items.push(m[1]);
        i++;
      }
      blocks.push({ type: "ul", items });
      continue;
    }

    // Ordered list: 1. item
    const olMatch = /^[ \t]*(\d+)\.\s+(.*)$/.exec(line);
    if (olMatch) {
      const start = Number.parseInt(olMatch[1], 10) || 1;
      const items: string[] = [];
      while (i < lines.length) {
        const m = /^[ \t]*\d+\.\s+(.*)$/.exec(lines[i]);
        if (!m) break;
        items.push(m[1]);
        i++;
      }
      blocks.push({ type: "ol", start, items });
      continue;
    }

    // Paragraph: collect contiguous normal text lines
    const paraLines: string[] = [line];
    i++;
    while (i < lines.length) {
      const next = lines[i];
      const nextTrimmed = next.trim();
      if (nextTrimmed.length === 0) break;
      if (/^[ \t]*(`{3,}|~{3,})/.test(next)) break;
      if (nextTrimmed.startsWith("\\[") || nextTrimmed.startsWith("$$")) break;
      if (/^[ \t]*#{1,6}\s+/.test(next)) break;
      if (/^[ \t]*>\s?/.test(next)) break;
      if (/^[ \t]*[-*+]\s+/.test(next)) break;
      if (/^[ \t]*\d+\.\s+/.test(next)) break;
      if (/^[ \t]*(?:-\s*-\s*-[-\s]*|\*\s*\*\s*\*[\*\s]*)$/.test(next)) break;
      if (
        next.includes("|") &&
        i + 1 < lines.length &&
        parseTableSeparator(lines[i + 1], splitPipeRow(next).length) !== null
      ) {
        break;
      }
      paraLines.push(next);
      i++;
    }
    blocks.push({ type: "paragraph", lines: paraLines });
  }

  return blocks;
}

function parseInlineNodes(text: string): InlineNode[] {
  const nodes: InlineNode[] = [];
  let i = 0;
  const len = text.length;
  let plainBuf = "";

  const flushPlain = () => {
    if (plainBuf.length > 0) {
      nodes.push({ type: "text", value: plainBuf });
      plainBuf = "";
    }
  };

  while (i < len) {
    // Inline code span: `...`
    if (text[i] === "`") {
      let tickCount = 0;
      while (i + tickCount < len && text[i + tickCount] === "`") tickCount++;
      const ticks = "`".repeat(tickCount);
      const closeIdx = text.indexOf(ticks, i + tickCount);
      if (closeIdx !== -1) {
        flushPlain();
        const codeContent = text.slice(i + tickCount, closeIdx);
        nodes.push({ type: "code", value: codeContent });
        i = closeIdx + tickCount;
        continue;
      }
      plainBuf += ticks;
      i += tickCount;
      continue;
    }

    // LaTeX display math: \[ ... \]
    if (text[i] === "\\" && text[i + 1] === "[") {
      const closeIdx = text.indexOf("\\]", i + 2);
      if (closeIdx !== -1 && closeIdx > i + 2) {
        flushPlain();
        nodes.push({
          type: "math_block",
          latex: text.slice(i + 2, closeIdx),
        });
        i = closeIdx + 2;
        continue;
      }
    }

    // LaTeX inline math: \( ... \)
    if (text[i] === "\\" && text[i + 1] === "(") {
      const closeIdx = text.indexOf("\\)", i + 2);
      if (closeIdx !== -1 && closeIdx > i + 2) {
        flushPlain();
        nodes.push({
          type: "math_inline",
          latex: text.slice(i + 2, closeIdx),
        });
        i = closeIdx + 2;
        continue;
      }
    }

    // Display math: $$ ... $$
    if (text[i] === "$" && text[i + 1] === "$") {
      const closeIdx = text.indexOf("$$", i + 2);
      if (closeIdx !== -1 && closeIdx > i + 2) {
        flushPlain();
        nodes.push({
          type: "math_block",
          latex: text.slice(i + 2, closeIdx),
        });
        i = closeIdx + 2;
        continue;
      }
    }

    // Safe single-$ inline math: $...$ (guards against $100 currency)
    if (
      text[i] === "$" &&
      text[i + 1] !== "$" &&
      (i === 0 || (text[i - 1] !== "\\" && !/[A-Za-z0-9$]/.test(text[i - 1]))) &&
      i + 1 < len &&
      !/\s/.test(text[i + 1])
    ) {
      let closeIdx = -1;
      for (let k = i + 1; k < len; k++) {
        if (text[k] === "\n") break;
        if (text[k] === "$" && text[k - 1] !== "\\") {
          closeIdx = k;
          break;
        }
      }
      if (
        closeIdx !== -1 &&
        closeIdx > i + 1 &&
        !/\s/.test(text[closeIdx - 1]) &&
        (closeIdx + 1 >= len || !/[A-Za-z0-9$]/.test(text[closeIdx + 1]))
      ) {
        const candidate = text.slice(i + 1, closeIdx);
        // Reject pure numbers/currency like $100$ or $1,200.50$ or currency ranges like $50 USD
        const isPureCurrency =
          /^\d+(?:[.,]\d+)*$/.test(candidate) || /^\d[\d.,]*\s+[A-Za-z]{2,}/.test(candidate);
        if (!isPureCurrency) {
          flushPlain();
          nodes.push({
            type: "math_inline",
            latex: candidate,
          });
          i = closeIdx + 1;
          continue;
        }
      }
    }

    // Bare LaTeX commands in prose: \begin{...}, \frac{...}{...}, \sqrt{...}, or known symbols like \theta_i, \times, \Delta
    if (text[i] === "\\" && i + 1 < len && /[A-Za-z]/.test(text[i + 1]) && (i === 0 || text[i - 1] !== ":")) {
      let cmdEnd = i + 1;
      while (cmdEnd < len && /[A-Za-z]/.test(text[cmdEnd])) cmdEnd++;
      const cmd = text.slice(i + 1, cmdEnd);

      if (cmd === "begin") {
        const envGrp = readBracedGroup(text, cmdEnd);
        if (envGrp) {
          const envName = envGrp.content.trim();
          const endTag = `\\end{${envName}}`;
          const closeIdx = text.indexOf(endTag, envGrp.nextIdx);
          if (closeIdx !== -1) {
            flushPlain();
            nodes.push({
              type: "math_block",
              latex: text.slice(i, closeIdx + endTag.length),
            });
            i = closeIdx + endTag.length;
            continue;
          }
        }
      }

      if (cmd === "frac" || cmd === "dfrac" || cmd === "tfrac") {
        const numGrp = readBracedGroup(text, cmdEnd);
        if (numGrp) {
          const denGrp = readBracedGroup(text, numGrp.nextIdx);
          if (denGrp) {
            flushPlain();
            nodes.push({
              type: "math_inline",
              latex: text.slice(i, denGrp.nextIdx),
            });
            i = denGrp.nextIdx;
            continue;
          }
        }
      }

      if (cmd === "sqrt") {
        let afterCmd = cmdEnd;
        const optRoot = readBracketGroup(text, afterCmd);
        if (optRoot) afterCmd = optRoot.nextIdx;
        const radGrp = readBracedGroup(text, afterCmd);
        if (radGrp) {
          flushPlain();
          nodes.push({
            type: "math_inline",
            latex: text.slice(i, radGrp.nextIdx),
          });
          i = radGrp.nextIdx;
          continue;
        }
      }

      if (cmd in LATEX_SYMBOLS) {
        let endIdx = cmdEnd;
        // Consume immediate subscript/superscript attached to this symbol, e.g. \theta_i, \sigma_{0}^{2}
        while (endIdx < len && (text[endIdx] === "_" || text[endIdx] === "^")) {
          const atom = readMathAtom(text, endIdx + 1);
          if (atom.content.length > 0 && atom.nextIdx > endIdx + 1) {
            endIdx = atom.nextIdx;
          } else {
            break;
          }
        }
        flushPlain();
        nodes.push({
          type: "math_inline",
          latex: text.slice(i, endIdx),
        });
        i = endIdx;
        continue;
      }
    }

    // Bare braced superscript / subscript in prose: e.g. 10^{-3}, m^{2}, H_{0}
    // (Never triggers on bare `_` without `{`, preserving S1A_IW_SLC__1SDV_20251001 and snake_case)
    if (
      (text[i] === "^" || text[i] === "_") &&
      i + 1 < len &&
      text[i + 1] === "{" &&
      i > 0 &&
      /[A-Za-z0-9)\]}]/.test(text[i - 1])
    ) {
      const grp = readBracedGroup(text, i + 1);
      if (grp && grp.content.length > 0 && !grp.content.includes("\n")) {
        flushPlain();
        nodes.push({
          type: "math_inline",
          latex: text.slice(i, grp.nextIdx),
        });
        i = grp.nextIdx;
        continue;
      }
    }

    // Safe Markdown link: [label](https://...)
    if (text[i] === "[") {
      const closeBracket = text.indexOf("]", i + 1);
      if (closeBracket !== -1 && text[closeBracket + 1] === "(") {
        const closeParen = text.indexOf(")", closeBracket + 2);
        if (closeParen !== -1) {
          const labelRaw = text.slice(i + 1, closeBracket);
          const hrefRaw = text.slice(closeBracket + 2, closeParen).trim();
          if (/^https?:\/\//i.test(hrefRaw)) {
            flushPlain();
            nodes.push({
              type: "link",
              label: parseInlineNodes(labelRaw),
              href: hrefRaw,
            });
            i = closeParen + 1;
            continue;
          }
        }
      }
    }

    // Bold: **...**
    if (text[i] === "*" && text[i + 1] === "*") {
      const closeBold = text.indexOf("**", i + 2);
      if (closeBold !== -1 && closeBold > i + 2 && !text.slice(i + 2, closeBold).includes("\n")) {
        flushPlain();
        const inner = text.slice(i + 2, closeBold);
        nodes.push({ type: "bold", children: parseInlineNodes(inner) });
        i = closeBold + 2;
        continue;
      }
    }

    // Italic: *...* (never underscore `_`, preserving scientific scene IDs and snake_case tool names)
    if (text[i] === "*" && text[i + 1] !== "*" && text[i + 1] !== " " && text[i + 1] !== "\n") {
      const closeItalic = text.indexOf("*", i + 1);
      if (
        closeItalic !== -1 &&
        closeItalic > i + 1 &&
        text[closeItalic - 1] !== " " &&
        !text.slice(i + 1, closeItalic).includes("\n")
      ) {
        flushPlain();
        const inner = text.slice(i + 1, closeItalic);
        nodes.push({ type: "italic", children: parseInlineNodes(inner) });
        i = closeItalic + 1;
        continue;
      }
    }

    plainBuf += text[i];
    i++;
  }

  flushPlain();
  return nodes;
}

function renderInlineNodes(nodes: InlineNode[], keyPrefix: string): React.ReactNode {
  return nodes.map((node, idx) => {
    const key = `${keyPrefix}-${idx}`;
    switch (node.type) {
      case "text":
        return <React.Fragment key={key}>{node.value}</React.Fragment>;
      case "code":
        return (
          <code
            key={key}
            className="rounded bg-sp-surface-muted border border-sp-line px-1.5 py-0.5 font-mono text-xs text-sp-text"
          >
            {node.value}
            <span className="hidden select-none" aria-hidden="true">
              {`\`${node.value}\``}
            </span>
          </code>
        );
      case "math_inline":
        return renderLatexFormula(node.latex, false, key);
      case "math_block":
        return renderLatexFormula(node.latex, true, key);
      case "bold":
        return (
          <strong key={key} className="font-semibold text-sp-text">
            {renderInlineNodes(node.children, key)}
          </strong>
        );
      case "italic":
        return (
          <em key={key} className="italic text-sp-text">
            {renderInlineNodes(node.children, key)}
          </em>
        );
      case "link":
        return (
          <a
            key={key}
            href={node.href}
            target="_blank"
            rel="noopener noreferrer"
            className="text-sp-accent underline underline-offset-2 hover:text-sp-accent-strong"
          >
            {renderInlineNodes(node.label, key)}
          </a>
        );
    }
  });
}

async function writeClipboardSafe(text: string): Promise<boolean> {
  if (typeof navigator !== "undefined" && navigator.clipboard?.writeText) {
    try {
      await navigator.clipboard.writeText(text);
      return true;
    } catch {
      // Fallback below
    }
  }
  if (typeof document !== "undefined") {
    try {
      const ta = document.createElement("textarea");
      ta.value = text;
      ta.setAttribute("readonly", "true");
      ta.style.position = "fixed";
      ta.style.opacity = "0";
      document.body.appendChild(ta);
      ta.select();
      const ok = typeof document.execCommand === "function" ? document.execCommand("copy") : true;
      document.body.removeChild(ta);
      return ok;
    } catch {
      return false;
    }
  }
  return false;
}

function FencedCodeBlockView({ language, code }: { language: string; code: string }) {
  const { t } = useI18n();
  const [copied, setCopied] = useState(false);
  const [wrapLines, setWrapLines] = useState(false);
  const timerRef = useRef<ReturnType<typeof setTimeout> | null>(null);

  useEffect(() => {
    return () => {
      if (timerRef.current) clearTimeout(timerRef.current);
    };
  }, []);

  const handleCopy = useCallback(async () => {
    await writeClipboardSafe(code);
    setCopied(true);
    if (timerRef.current) clearTimeout(timerRef.current);
    timerRef.current = setTimeout(() => setCopied(false), 1800);
  }, [code]);

  return (
    <div
      data-testid="agent-code-block"
      className="rounded-lg border border-sp-line bg-sp-canvas overflow-hidden my-1.5"
    >
      <div className="flex items-center justify-between gap-2 border-b border-sp-line bg-sp-surface-muted px-3 py-1.5 select-none">
        <span className="font-mono text-xs text-sp-text-muted uppercase tracking-wide">
          {language || "text"}
        </span>
        <div className="flex items-center gap-1">
          <button
            type="button"
            data-testid="btn-toggle-code-wrap"
            aria-pressed={wrapLines}
            title={wrapLines ? "关闭自动换行" : "自动换行"}
            aria-label="自动换行"
            onClick={() => setWrapLines((prev) => !prev)}
            className={cn(
              "inline-flex items-center gap-1 rounded px-2 py-0.5 text-xs transition-colors duration-fast ease-sp cursor-pointer",
              wrapLines
                ? "bg-sp-active text-sp-text font-medium"
                : "text-sp-text-secondary hover:bg-sp-hover hover:text-sp-text"
            )}
          >
            <WrapText className="h-3.5 w-3.5" />
            <span>换行</span>
          </button>
          <button
            type="button"
            data-testid="btn-copy-code-block"
            onClick={handleCopy}
            aria-label={copied ? t("agent.actions.copied") : t("agent.actions.copyCode")}
            className="inline-flex items-center gap-1 rounded px-2 py-0.5 text-xs text-sp-text-secondary hover:bg-sp-hover hover:text-sp-text transition-colors duration-fast ease-sp cursor-pointer"
          >
            {copied ? (
              <>
                <Check className="h-3.5 w-3.5 text-sp-status-success" />
                <span className="text-sp-success-text">{t("agent.actions.copied")}</span>
              </>
            ) : (
              <>
                <Copy className="h-3.5 w-3.5 text-sp-text-muted" />
                <span>{t("agent.actions.copyCode")}</span>
              </>
            )}
          </button>
        </div>
      </div>
      <pre
        className={cn(
          "p-3 m-0 font-mono text-xs leading-[1.6] text-sp-text select-text",
          wrapLines ? "whitespace-pre-wrap break-words overflow-x-hidden" : "overflow-x-auto whitespace-pre"
        )}
      >
        <code>{code}</code>
      </pre>
    </div>
  );
}

export function serializeMarkdownTable(
  headers: string[],
  alignments: TableAlignment[],
  rows: string[][]
): string {
  const escapeCell = (raw: string) => raw.replace(/\|/g, "\\|").replace(/\r?\n/g, " ");
  const alignMarker = (a: TableAlignment | undefined) =>
    a === "center" ? ":---:" : a === "right" ? "---:" : ":---";
  const headerLine = `| ${headers.map(escapeCell).join(" | ")} |`;
  const sepLine = `| ${headers.map((_, idx) => alignMarker(alignments[idx])).join(" | ")} |`;
  const bodyLines = rows.map(
    (row) => `| ${headers.map((_, idx) => escapeCell(row[idx] ?? "")).join(" | ")} |`
  );
  return [headerLine, sepLine, ...bodyLines].join("\n");
}

export function serializeTableCsv(headers: string[], rows: string[][]): string {
  const escapeCsvField = (raw: string) => {
    const val = String(raw ?? "");
    if (/[",\r\n]/.test(val)) {
      return `"${val.replace(/"/g, '""')}"`;
    }
    return val;
  };
  const lines = [
    headers.map(escapeCsvField).join(","),
    ...rows.map((row) => headers.map((_, idx) => escapeCsvField(row[idx] ?? "")).join(",")),
  ];
  return `\uFEFF${lines.join("\r\n")}`;
}

function triggerCsvDownload(csv: string, filename = "eostage-table.csv"): boolean {
  try {
    if (typeof document === "undefined" || typeof URL === "undefined" || typeof URL.createObjectURL !== "function") {
      return false;
    }
    const blob = new Blob([csv], { type: "text/csv;charset=utf-8;" });
    const url = URL.createObjectURL(blob);
    const link = document.createElement("a");
    link.href = url;
    link.download = filename;
    link.style.display = "none";
    document.body.appendChild(link);
    link.click();
    document.body.removeChild(link);
    setTimeout(() => URL.revokeObjectURL(url), 1000);
    return true;
  } catch {
    return false;
  }
}

function MarkdownTableContent({
  headers,
  alignments,
  rows,
  keyPrefix,
  relaxed = false,
}: {
  headers: string[];
  alignments: TableAlignment[];
  rows: string[][];
  keyPrefix: string;
  relaxed?: boolean;
}) {
  return (
    <table className={cn("w-full border-collapse", relaxed ? "text-sm" : "text-xs")}>
      <thead className="bg-sp-surface-muted border-b border-sp-line sticky top-0 z-10">
        <tr>
          {headers.map((header, hIdx) => {
            const align = alignments[hIdx] ?? "left";
            return (
              <th
                key={`${keyPrefix}-th-${hIdx}`}
                className={cn(
                  relaxed ? "px-4 py-2.5 font-semibold text-sp-text whitespace-nowrap" : "px-3 py-2 font-semibold text-sp-text whitespace-nowrap",
                  align === "center" && "text-center",
                  align === "right" && "text-right",
                  align === "left" && "text-left"
                )}
              >
                {renderInlineNodes(parseInlineNodes(header), `${keyPrefix}-th-${hIdx}`)}
              </th>
            );
          })}
        </tr>
      </thead>
      <tbody className="divide-y divide-sp-line bg-sp-card">
        {rows.map((row, rIdx) => (
          <tr key={`${keyPrefix}-tr-${rIdx}`} className="hover:bg-sp-hover transition-colors duration-fast">
            {row.map((cell, cIdx) => {
              const align = alignments[cIdx] ?? "left";
              return (
                <td
                  key={`${keyPrefix}-td-${rIdx}-${cIdx}`}
                  className={cn(
                    relaxed
                      ? "px-4 py-2 text-sp-text align-top min-w-[84px] max-w-[440px] break-words"
                      : "px-3 py-1.5 text-sp-text align-top min-w-[84px] max-w-[420px] break-words",
                    align === "center" && "text-center",
                    align === "right" && "text-right tabular-nums",
                    align === "left" && "text-left"
                  )}
                >
                  {renderInlineNodes(parseInlineNodes(cell), `${keyPrefix}-td-${rIdx}-${cIdx}`)}
                </td>
              );
            })}
          </tr>
        ))}
      </tbody>
    </table>
  );
}

function MarkdownTableView({
  headers,
  alignments,
  rows,
  keyPrefix,
}: {
  headers: string[];
  alignments: TableAlignment[];
  rows: string[][];
  keyPrefix: string;
}) {
  const [copiedMd, setCopiedMd] = useState(false);
  const [downloadedCsv, setDownloadedCsv] = useState(false);
  const [expanded, setExpanded] = useState(false);
  const copyTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const csvTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);

  useEffect(() => {
    return () => {
      if (copyTimerRef.current) clearTimeout(copyTimerRef.current);
      if (csvTimerRef.current) clearTimeout(csvTimerRef.current);
    };
  }, []);

  useEffect(() => {
    if (!expanded) return;
    const onKeyDown = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        e.stopPropagation();
        setExpanded(false);
      }
    };
    window.addEventListener("keydown", onKeyDown, true);
    return () => window.removeEventListener("keydown", onKeyDown, true);
  }, [expanded]);

  const handleCopyMarkdown = useCallback(async () => {
    await writeClipboardSafe(serializeMarkdownTable(headers, alignments, rows));
    setCopiedMd(true);
    if (copyTimerRef.current) clearTimeout(copyTimerRef.current);
    copyTimerRef.current = setTimeout(() => setCopiedMd(false), 1800);
  }, [headers, alignments, rows]);

  const handleDownloadCsv = useCallback(() => {
    const csv = serializeTableCsv(headers, rows);
    triggerCsvDownload(csv, `eostage-table-${rows.length}rows.csv`);
    setDownloadedCsv(true);
    if (csvTimerRef.current) clearTimeout(csvTimerRef.current);
    csvTimerRef.current = setTimeout(() => setDownloadedCsv(false), 1800);
  }, [headers, rows]);

  const toolbarButtons = (inModal: boolean) => (
    <div className="flex items-center gap-1 select-none">
      <button
        type="button"
        data-testid={inModal ? "btn-modal-copy-table-markdown" : "btn-copy-table-markdown"}
        onClick={() => void handleCopyMarkdown()}
        title="复制 Markdown"
        aria-label={copiedMd ? "已复制 Markdown" : "复制 Markdown"}
        className="inline-flex h-6 items-center gap-1 rounded px-1.5 text-xs text-sp-text-muted hover:bg-sp-hover hover:text-sp-text transition-colors cursor-pointer"
      >
        {copiedMd ? (
          <>
            <Check className="h-3.5 w-3.5 text-sp-status-success" />
            <span className="text-sp-success-text">已复制</span>
          </>
        ) : (
          <Copy className="h-3.5 w-3.5" />
        )}
      </button>
      <button
        type="button"
        data-testid={inModal ? "btn-modal-download-table-csv" : "btn-download-table-csv"}
        onClick={handleDownloadCsv}
        title="下载 CSV"
        aria-label="下载 CSV"
        className="inline-flex h-6 items-center gap-1 rounded px-1.5 text-xs text-sp-text-muted hover:bg-sp-hover hover:text-sp-text transition-colors cursor-pointer"
      >
        {downloadedCsv ? (
          <Check className="h-3.5 w-3.5 text-sp-status-success" />
        ) : (
          <Download className="h-3.5 w-3.5" />
        )}
      </button>
      {!inModal ? (
        <button
          type="button"
          data-testid="btn-expand-table-preview"
          onClick={() => setExpanded(true)}
          title="放大预览表格"
          aria-label="放大预览表格"
          className="inline-flex h-6 items-center gap-1 rounded px-1.5 text-xs text-sp-text-muted hover:bg-sp-hover hover:text-sp-text transition-colors cursor-pointer"
        >
          <Maximize2 className="h-3.5 w-3.5" />
        </button>
      ) : (
        <button
          type="button"
          data-testid="btn-close-table-preview"
          onClick={() => setExpanded(false)}
          title="关闭预览"
          aria-label="关闭预览"
          className="inline-flex h-6 w-6 items-center justify-center rounded text-sp-text-secondary hover:bg-sp-hover hover:text-sp-text transition-colors cursor-pointer"
        >
          <X className="h-4 w-4" />
        </button>
      )}
    </div>
  );

  return (
    <div data-testid="agent-markdown-table" className="my-1.5">
      <div className="mb-1 flex items-center justify-end">
        {toolbarButtons(false)}
      </div>
      <div
        data-testid="md-table-scroll-box"
        className="relative overflow-x-auto max-w-full rounded-lg border border-sp-line"
      >
        <MarkdownTableContent
          headers={headers}
          alignments={alignments}
          rows={rows}
          keyPrefix={keyPrefix}
        />
      </div>
      {expanded && (
        <ModalOverlay
          closeOnBackdropClick={false}
          onClose={() => setExpanded(false)}
          role="dialog"
          aria-modal={true}
          aria-label="表格放大预览"
          data-testid="table-preview-modal"
          className="z-[980] p-4 sm:p-6"
        >
          <div className="flex max-h-[85vh] w-[min(92vw,72rem)] flex-col overflow-hidden rounded-sp-lg border border-sp-line bg-sp-surface shadow-modal">
            <div className="flex items-center justify-between border-b border-sp-line bg-sp-surface-muted px-4 py-2.5">
              <div className="flex items-center gap-2 text-xs font-semibold text-sp-text">
                <span>表格预览</span>
                <span className="text-sp-text-muted font-normal tabular-nums">
                  {rows.length} 行 × {headers.length} 列
                </span>
              </div>
              {toolbarButtons(true)}
            </div>
            <div className="min-h-0 flex-1 overflow-auto">
              <MarkdownTableContent
                headers={headers}
                alignments={alignments}
                rows={rows}
                keyPrefix={`${keyPrefix}-modal`}
                relaxed
              />
            </div>
          </div>
        </ModalOverlay>
      )}
    </div>
  );
}

export function AgentMarkdownRenderer({
  content,
  className,
  compact = false,
  streaming = false,
}: AgentMarkdownRendererProps) {
  const { answerMarkdown } = splitInlineThink(content);

  if (!answerMarkdown || answerMarkdown.trim().length === 0) {
    return null;
  }

  const blocks = parseMarkdownBlocks(answerMarkdown);
  const lastBlock = blocks[blocks.length - 1];
  const trailingCaretInParagraph = streaming && lastBlock?.type === "paragraph";

  return (
    <div
      data-testid="agent-markdown-renderer"
      className={cn(
        "select-text break-words text-sp-text",
        compact ? "text-xs leading-[1.6] space-y-1.5" : "text-sm leading-[1.65] space-y-2.5",
        className
      )}
    >
      {blocks.map((block, bIdx) => {
        const key = `md-b-${bIdx}`;
        switch (block.type) {
          case "heading": {
            const children = renderInlineNodes(parseInlineNodes(block.text), key);
            if (block.level === 1) {
              return (
                <h2 key={key} className="text-base font-semibold text-sp-text tracking-tight pt-1">
                  {children}
                </h2>
              );
            }
            if (block.level === 2) {
              return (
                <h3 key={key} className="text-sm font-semibold text-sp-text tracking-tight pt-1">
                  {children}
                </h3>
              );
            }
            return (
              <h4 key={key} className="text-xs font-semibold text-sp-text tracking-tight pt-0.5">
                {children}
              </h4>
            );
          }

          case "code_block":
            return <FencedCodeBlockView key={key} language={block.language} code={block.code} />;

          case "math_block":
            return renderLatexFormula(block.latex, true, key);

          case "blockquote":
            return (
              <blockquote
                key={key}
                className="border-l-2 border-sp-accent pl-3 py-0.5 text-sp-text-secondary space-y-1 m-0"
              >
                {block.lines.map((ln, lIdx) => (
                  <div key={`${key}-q-${lIdx}`}>
                    {renderInlineNodes(parseInlineNodes(ln), `${key}-q-${lIdx}`)}
                  </div>
                ))}
              </blockquote>
            );

          case "ul":
            return (
              <ul key={key} className="list-disc pl-5 space-y-1 m-0">
                {block.items.map((item, iIdx) => (
                  <li key={`${key}-li-${iIdx}`} className="text-sp-text">
                    {renderInlineNodes(parseInlineNodes(item), `${key}-li-${iIdx}`)}
                  </li>
                ))}
              </ul>
            );

          case "ol":
            return (
              <ol key={key} start={block.start} className="list-decimal pl-5 space-y-1 m-0">
                {block.items.map((item, iIdx) => (
                  <li key={`${key}-oli-${iIdx}`} className="text-sp-text">
                    {renderInlineNodes(parseInlineNodes(item), `${key}-oli-${iIdx}`)}
                  </li>
                ))}
              </ol>
            );

          case "table":
            return (
              <MarkdownTableView
                key={key}
                headers={block.headers}
                alignments={block.alignments}
                rows={block.rows}
                keyPrefix={key}
              />
            );

          case "hr":
            return <hr key={key} className="border-0 border-t border-sp-line my-2" />;

          case "paragraph":
            return (
              <p key={key} className="m-0 whitespace-pre-wrap">
                {block.lines.map((ln, lIdx) => (
                  <React.Fragment key={`${key}-p-${lIdx}`}>
                    {lIdx > 0 && "\n"}
                    {renderInlineNodes(parseInlineNodes(ln), `${key}-p-${lIdx}`)}
                  </React.Fragment>
                ))}
                {trailingCaretInParagraph && bIdx === blocks.length - 1 && (
                  <span aria-hidden="true" data-testid="agent-stream-caret" className="sp-stream-caret" />
                )}
              </p>
            );
        }
      })}
      {streaming && !trailingCaretInParagraph && (
        <span aria-hidden="true" data-testid="agent-stream-caret" className="sp-stream-caret" />
      )}
    </div>
  );
}
