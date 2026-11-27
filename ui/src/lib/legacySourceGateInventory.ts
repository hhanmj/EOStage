/**
 * Legacy Source Gate Inventory Helper
 *
 * Dedicated read-only inventory helper designed for future migration of existing
 * localStorage gate records in EOStage.
 *
 * Discovered Storage Keys:
 * - Fixed: "eostage.sourceMutationGate.v1" (from ui/src/lib/taskSourceMutationGate.ts)
 * - Fixed: "eostage.reacquireRequestGate.v1" (from ui/src/lib/reacquireRequestGate.ts)
 * - Prefix: "insar:nisar-recovery:pending:v1:" (from ui/src/components/NisarArchivedRecoveryConfirmation.tsx)
 *
 * CRITICAL SAFETY AND CONCURRENCY INVARIANTS:
 * 1. Observation-Time Scope:
 *    A successful inventory result contains observations from this storage instance only.
 *    Concurrent writes can produce a torn or ABA snapshot even when both passes match;
 *    the result is not proof that all entries existed together at any instant.
 * 2. No Global Writer Stoppage Guarantee:
 *    A successful snapshot does NOT establish that all profiles, windows, WebViews, workers,
 *    or concurrent processes have stopped writing. LocalStorage provides no cross-process
 *    mutual exclusion, CAS (Compare-And-Swap), or transactional isolation.
 * 3. Migration Barrier Invariant:
 *    Success must NEVER itself open a migration barrier or release any safety gate.
 *    This helper is strictly non-destructive, read-only inspection.
 * 4. Observable Change Detection (Not Claiming Atomicity):
 *    Repeated snapshots detect some changes between passes, not every concurrent change.
 *    They provide no transactional or cross-process atomicity.
 * 5. Data Preservation Without Mutation:
 *    Captures exact raw strings as stored. Never parses, truncates, deduplicates, mutates,
 *    logs, or exposes data in errors. Malformed JSON, duplicates within arrays, empty strings,
 *    and oversized records are captured exactly as raw strings.
 * 6. Fail-Closed:
 *    Access failures and observed inconsistencies return a generic failure with no partial result.
 *    Changes between reads can still go undetected.
 */

/**
 * Minimal Storage-like reader interface satisfied by standard Web Storage
 * (e.g., window.localStorage) or mock/isolated storage implementations.
 */
export interface LegacyStorageLikeReader {
  readonly length: number;
  key(index: number): string | null;
  getItem(key: string): string | null;
}

/**
 * Fixed storage key for task source mutation gate.
 * Discovered from ui/src/lib/taskSourceMutationGate.ts
 */
export const LEGACY_SOURCE_MUTATION_GATE_KEY = "eostage.sourceMutationGate.v1" as const;

/**
 * Fixed storage key for reacquire request gate.
 * Discovered from ui/src/lib/reacquireRequestGate.ts
 */
export const LEGACY_REACQUIRE_REQUEST_GATE_KEY = "eostage.reacquireRequestGate.v1" as const;

/**
 * Key prefix for NISAR archived recovery pending confirmation gates.
 * Discovered from ui/src/components/NisarArchivedRecoveryConfirmation.tsx
 */
export const LEGACY_NISAR_RECOVERY_KEY_PREFIX = "insar:nisar-recovery:pending:v1:" as const;

/**
 * All legacy fixed gate keys known to EOStage.
 */
export const LEGACY_FIXED_GATE_KEYS: readonly string[] = Object.freeze([
  LEGACY_SOURCE_MUTATION_GATE_KEY,
  LEGACY_REACQUIRE_REQUEST_GATE_KEY,
]);

/**
 * Constant generic failure code.
 * Intentionally generic to avoid exposing keys, values, or internal diagnostic paths.
 */
export const LEGACY_GATE_INVENTORY_GENERIC_ERROR = "STORAGE_INVENTORY_SCAN_FAILED" as const;

/**
 * Raw entry capturing a discovered gate key and its unparsed, unmodified raw string value.
 */
export interface LegacyGateRawEntry {
  readonly key: string;
  readonly rawValue: string;
}

/**
 * Observations of legacy source gates from two matching passes over one storage instance.
 */
export interface LegacyGateInventory {
  /**
   * Fixed gate record for "eostage.sourceMutationGate.v1".
   * Exact raw string value if key was present in storage; null if key was absent.
   */
  readonly sourceMutationGate: string | null;

  /**
   * Fixed gate record for "eostage.reacquireRequestGate.v1".
   * Exact raw string value if key was present in storage; null if key was absent.
   */
  readonly reacquireRequestGate: string | null;

  /**
   * All discovered NISAR pending recovery gate records whose keys begin with
   * "insar:nisar-recovery:pending:v1:".
   */
  readonly nisarRecoveryGates: readonly LegacyGateRawEntry[];

  /**
   * All discovered legacy gate entries (both fixed keys that were present and NISAR prefix keys),
   * sorted deterministically by key.
   */
  readonly allEntries: readonly LegacyGateRawEntry[];
}

export type LegacyGateInventoryResult =
  | {
      readonly ok: true;
      readonly inventory: LegacyGateInventory;
    }
  | {
      readonly ok: false;
      readonly error: typeof LEGACY_GATE_INVENTORY_GENERIC_ERROR;
    };

// A storage with more keys fails closed; no entries are returned or truncated.
const MAX_STORAGE_KEYS = 100_000;

interface InternalSnapshot {
  readonly storageLength: number;
  readonly fixedEntries: Map<string, string | null>;
  readonly nisarEntries: Map<string, string>;
}

/**
 * Executes a single bounded pass across the reader to enumerate keys and capture raw values.
 * Returns null on access errors or inconsistencies that this pass can observe.
 */
function captureSinglePass(reader: LegacyStorageLikeReader): InternalSnapshot | null {
  try {
    if (!reader || typeof reader !== "object") {
      return null;
    }

    const initialLength = reader.length;
    if (typeof initialLength !== "number" || !Number.isSafeInteger(initialLength) || initialLength < 0 || initialLength > MAX_STORAGE_KEYS) {
      return null;
    }

    const discoveredNisarKeys = new Set<string>();
    const enumeratedKeys = new Set<string>();

    for (let index = 0; index < initialLength; index += 1) {
      const candidateKey = reader.key(index);
      if (typeof candidateKey !== "string") {
        // Returned null or non-string while index < initialLength indicates
        // concurrent removal or invalid reader implementation.
        return null;
      }
      // Web Storage keys are unique. A duplicate indicates an inconsistent scan.
      if (enumeratedKeys.has(candidateKey)) return null;
      enumeratedKeys.add(candidateKey);
      if (candidateKey.startsWith(LEGACY_NISAR_RECOVERY_KEY_PREFIX)) {
        discoveredNisarKeys.add(candidateKey);
      }
    }

    // Verify length remained constant throughout key enumeration
    if (reader.length !== initialLength) {
      return null;
    }

    // Capture fixed keys
    const fixedEntries = new Map<string, string | null>();
    for (let index = 0; index < LEGACY_FIXED_GATE_KEYS.length; index += 1) {
      const fixedKey = LEGACY_FIXED_GATE_KEYS[index];
      const rawVal = reader.getItem(fixedKey);
      if ((rawVal !== null && typeof rawVal !== "string") ||
          enumeratedKeys.has(fixedKey) !== (rawVal !== null)) {
        return null;
      }
      fixedEntries.set(fixedKey, rawVal);
    }

    // Capture discovered NISAR prefix keys in deterministic order
    const nisarEntries = new Map<string, string>();
    const sortedNisarKeys = Array.from(discoveredNisarKeys).sort();
    for (let index = 0; index < sortedNisarKeys.length; index += 1) {
      const nisarKey = sortedNisarKeys[index];
      const rawVal = reader.getItem(nisarKey);
      if (typeof rawVal !== "string") {
        // Discovered key no longer returns a string; key was deleted mid-scan.
        return null;
      }
      nisarEntries.set(nisarKey, rawVal);
    }

    // Verify length remained constant after all value reads
    if (reader.length !== initialLength) {
      return null;
    }

    return {
      storageLength: initialLength,
      fixedEntries,
      nisarEntries,
    };
  } catch {
    return null;
  }
}

/**
 * Compares two snapshots for exact observable identity across length, keys, and raw values.
 */
function areSnapshotsIdentical(a: InternalSnapshot, b: InternalSnapshot): boolean {
  if (a.storageLength !== b.storageLength) {
    return false;
  }

  if (a.fixedEntries.size !== b.fixedEntries.size) {
    return false;
  }
  for (const [key, valA] of a.fixedEntries) {
    if (!b.fixedEntries.has(key) || b.fixedEntries.get(key) !== valA) {
      return false;
    }
  }

  if (a.nisarEntries.size !== b.nisarEntries.size) {
    return false;
  }
  for (const [key, valA] of a.nisarEntries) {
    if (!b.nisarEntries.has(key) || b.nisarEntries.get(key) !== valA) {
      return false;
    }
  }

  return true;
}

/**
 * Performs a read-only inventory of legacy source gates from an injected storage reader.
 *
 * Uses two complete passes to detect some changes. Reads can still be torn or exhibit ABA.
 * On detected inconsistency or any access failure, returns no partial result.
 *
 * SAFETY WARNINGS:
 * - Success contains observations from this storage instance; it is not an atomic snapshot.
 * - Does NOT establish that all profiles, windows, or concurrent writers have stopped.
 * - Must NEVER itself open a migration barrier or release a gate.
 * - Raw string data is never parsed, truncated, deduplicated, mutated, or logged.
 */
export function inventoryLegacySourceGates(
  reader: LegacyStorageLikeReader,
): LegacyGateInventoryResult {
  try {
    if (!reader || typeof reader !== "object") {
      return { ok: false, error: LEGACY_GATE_INVENTORY_GENERIC_ERROR };
    }

    // Pass 1: Initial snapshot capture
    const firstPass = captureSinglePass(reader);
    if (!firstPass) {
      return { ok: false, error: LEGACY_GATE_INVENTORY_GENERIC_ERROR };
    }

    // Pass 2: Repeated snapshot capture for change detection
    const secondPass = captureSinglePass(reader);
    if (!secondPass) {
      return { ok: false, error: LEGACY_GATE_INVENTORY_GENERIC_ERROR };
    }

    // Compare snapshots to detect observable changes mid-scan
    if (!areSnapshotsIdentical(firstPass, secondPass)) {
      return { ok: false, error: LEGACY_GATE_INVENTORY_GENERIC_ERROR };
    }

    // Return observed values; equal passes do not establish an atomic snapshot.
    const sourceMutationGate = firstPass.fixedEntries.get(LEGACY_SOURCE_MUTATION_GATE_KEY) ?? null;
    const reacquireRequestGate = firstPass.fixedEntries.get(LEGACY_REACQUIRE_REQUEST_GATE_KEY) ?? null;

    const nisarRecoveryGates: LegacyGateRawEntry[] = [];
    const sortedNisarKeys = Array.from(firstPass.nisarEntries.keys()).sort();
    for (let index = 0; index < sortedNisarKeys.length; index += 1) {
      const key = sortedNisarKeys[index];
      const rawValue = firstPass.nisarEntries.get(key);
      if (typeof rawValue === "string") {
        nisarRecoveryGates.push(Object.freeze({ key, rawValue }));
      }
    }

    const allEntries: LegacyGateRawEntry[] = [];
    if (sourceMutationGate !== null) {
      allEntries.push(Object.freeze({ key: LEGACY_SOURCE_MUTATION_GATE_KEY, rawValue: sourceMutationGate }));
    }
    if (reacquireRequestGate !== null) {
      allEntries.push(Object.freeze({ key: LEGACY_REACQUIRE_REQUEST_GATE_KEY, rawValue: reacquireRequestGate }));
    }
    for (let index = 0; index < nisarRecoveryGates.length; index += 1) {
      allEntries.push(nisarRecoveryGates[index]);
    }
    allEntries.sort((a, b) => (a.key < b.key ? -1 : a.key > b.key ? 1 : 0));

    return {
      ok: true,
      inventory: {
        sourceMutationGate,
        reacquireRequestGate,
        nisarRecoveryGates: Object.freeze(nisarRecoveryGates),
        allEntries: Object.freeze(allEntries),
      },
    };
  } catch {
    return { ok: false, error: LEGACY_GATE_INVENTORY_GENERIC_ERROR };
  }
}
