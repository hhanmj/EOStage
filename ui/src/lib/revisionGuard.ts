/**
 * Small, presentation-only guard for Store-backed snapshots and transient
 * events. It never stores products or tasks; it only prevents an old response
 * or duplicate event from overwriting the latest projection after reconnect.
 */
export class RevisionGuard {
  private readonly latest = new Map<string, number>();
  private readonly seenEvents = new Set<string>();
  private readonly eventOrder: string[] = [];
  private readonly maxEventIds: number;

  constructor(maxEventIds = 256) {
    if (!Number.isInteger(maxEventIds) || maxEventIds < 1) {
      throw new RangeError("maxEventIds must be a positive integer");
    }
    this.maxEventIds = maxEventIds;
  }

  acceptSnapshot(stream: string, revision: number): boolean {
    if (!this.validRevision(revision)) return false;
    const previous = this.latest.get(stream);
    if (previous !== undefined && revision <= previous) return false;
    this.latest.set(stream, revision);
    return true;
  }

  /**
   * Accept an atomic restore containing multiple independent revision streams.
   * Equal revisions are harmless within a mixed snapshot, while any stream
   * older than the last accepted value invalidates the whole response.  This
   * keeps a late partial restore from overwriting a newer projection.
   */
  acceptSnapshotBatch(
    entries: readonly { stream: string; revision: number }[],
  ): boolean {
    // An empty Store is a valid liveness snapshot but has no revision ledger;
    // each such read may be published without changing guarded state.
    if (entries.length === 0) return true;
    const streams = new Set<string>();
    let advanced = false;
    for (const entry of entries) {
      if (
        !entry.stream ||
        streams.has(entry.stream) ||
        !this.validRevision(entry.revision)
      ) {
        return false;
      }
      streams.add(entry.stream);
      const previous = this.latest.get(entry.stream);
      if (previous !== undefined && entry.revision < previous) return false;
      if (previous === undefined || entry.revision > previous) advanced = true;
    }
    if (!advanced) return false;
    for (const entry of entries) this.latest.set(entry.stream, entry.revision);
    return true;
  }

  acceptEvent(stream: string, eventId: string, revision: number): boolean {
    if (!this.validRevision(revision) || !eventId) return false;
    const scopedEventId = `${stream}\u0000${eventId}`;
    if (this.seenEvents.has(scopedEventId)) return false;
    const previous = this.latest.get(stream);
    if (previous !== undefined && revision < previous) return false;
    this.latest.set(stream, Math.max(previous ?? revision, revision));
    this.seenEvents.add(scopedEventId);
    this.eventOrder.push(scopedEventId);
    while (this.eventOrder.length > this.maxEventIds) {
      const oldest = this.eventOrder.shift();
      if (oldest) this.seenEvents.delete(oldest);
    }
    return true;
  }

  /** A reconnect starts a fresh read; the next snapshot may repeat its revision. */
  reconnect(stream: string): void {
    this.latest.delete(stream);
  }

  reset(): void {
    this.latest.clear();
    this.seenEvents.clear();
    this.eventOrder.length = 0;
  }

  private validRevision(revision: number): boolean {
    return Number.isSafeInteger(revision) && revision >= 0;
  }
}
