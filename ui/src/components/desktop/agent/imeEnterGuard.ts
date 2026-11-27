/**
 * imeEnterGuard.ts
 * Pure state machine guarding Enter-to-send against active CJK IME composition
 * AND WebKit/WKWebView's synchronous post-compositionend Enter keydown quirk,
 * without blocking fast human Enter presses right after confirming Chinese IME text.
 *
 * Root cause of the fast-Enter newline bug:
 * Previously, `AgentComposer` ignored Enter for 500ms after `compositionend`
 * (`Date.now() - lastCompositionEndTimeRef.current < 500`). When typing Chinese
 * and pressing Space/digit to commit a candidate, then hitting Enter within
 * 80-450ms to send, the handler returned early without `preventDefault()`,
 * causing `<textarea>` to insert a newline instead of sending.
 *
 * Fix:
 * 1. Block while actively composing (`isComposing` state, `nativeEvent.isComposing`, or `keyCode === 229`).
 * 2. Track a narrow WebKit-only post-compositionend window (30ms, matching
 *    synchronous same-keystroke dispatch where WebKit fires `compositionend`
 *    immediately before `keydown(Enter)` on the *same* Enter keypress).
 * 3. Immediately clear the pending WebKit flag on any non-Enter `keyup` or
 *    non-composing non-Enter `keydown` (so Space/digit -> Enter sends even
 *    faster than 30ms!), and consume the flag on the first blocked Enter so
 *    a subsequent Enter always sends.
 */

export const WEBKIT_POST_COMPOSITION_ENTER_GUARD_MS = 30;

export interface ImeEnterGuardState {
  isComposing: boolean;
  lastCompositionEndMs: number;
  pendingPostCompositionEnter: boolean;
}

export function createImeEnterGuardState(): ImeEnterGuardState {
  return {
    isComposing: false,
    lastCompositionEndMs: 0,
    pendingPostCompositionEnter: false,
  };
}

export function onImeCompositionStart(state: ImeEnterGuardState): void {
  state.isComposing = true;
  state.pendingPostCompositionEnter = false;
}

export function onImeCompositionEnd(state: ImeEnterGuardState, nowMs: number = Date.now()): void {
  state.isComposing = false;
  state.lastCompositionEndMs = nowMs;
  state.pendingPostCompositionEnter = true;
}

export function onImeKeyUp(state: ImeEnterGuardState, key: string): void {
  if (key !== "Enter") {
    state.pendingPostCompositionEnter = false;
  }
}

export interface ShouldBlockEnterInput {
  key: string;
  nativeIsComposing: boolean;
  keyCode?: number;
  nowMs?: number;
}

/**
 * Returns true if the current keydown event is still part of an IME composition
 * (and therefore should NOT trigger Enter-to-send or composer shortcuts).
 */
export function shouldBlockKeyDownForIme(
  state: ImeEnterGuardState,
  input: ShouldBlockEnterInput
): boolean {
  if (state.isComposing || input.nativeIsComposing || input.keyCode === 229) {
    return true;
  }

  if (input.key !== "Enter") {
    state.pendingPostCompositionEnter = false;
    return false;
  }

  const nowMs = input.nowMs ?? Date.now();
  if (
    state.pendingPostCompositionEnter &&
    nowMs - state.lastCompositionEndMs < WEBKIT_POST_COMPOSITION_ENTER_GUARD_MS
  ) {
    // Consume the one-shot WebKit post-compositionend guard so a second Enter immediately sends.
    state.pendingPostCompositionEnter = false;
    return true;
  }

  state.pendingPostCompositionEnter = false;
  return false;
}
