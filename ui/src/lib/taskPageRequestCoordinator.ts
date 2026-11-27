export class TaskPageRequestCoordinator {
  private generation = 0;
  private activeKey = "";
  private revisions = new Map<string, number>();
  private revisionPolicy: "exact" | "observed" = "exact";

  /** Publishes the authoritative runtime revision carried by live/history rows. */
  observeRevision(taskId: string, revision: number) {
    const previous = this.revisions.get(taskId);
    if (this.revisionPolicy === "exact" && previous !== undefined
      && revision > previous && this.activeKey.startsWith(`${taskId}:`)) {
      this.invalidate();
    }
    if (previous === undefined || revision > previous) this.revisions.set(taskId, revision);
  }

  request<T>(taskId: string, revision: number, pageIndex: number, read: () => Promise<T>,
    revisionPolicy: "exact" | "observed" = "exact"): Promise<T | null> {
    const observedRevision = this.revisions.get(taskId);
    if (revisionPolicy === "exact" && observedRevision !== undefined
      && revision < observedRevision) return Promise.resolve(null);
    if (observedRevision === undefined || revision > observedRevision) this.revisions.set(taskId, revision);
    // Read-only Store detail returns its own coherent observed revision. A live
    // progress tick must not starve that read. Navigation/close still invalidate
    // generations; mutation revision checks remain owned by Store commands.
    this.revisionPolicy = revisionPolicy;
    const generation = ++this.generation;
    const key = `${taskId}:${revision}:${pageIndex}`;
    this.activeKey = key;
    return Promise.resolve().then(read).then(
      (value) => generation === this.generation && this.activeKey === key ? value : null,
      (error) => {
        if (generation === this.generation && this.activeKey === key) throw error;
        return null;
      },
    );
  }

  invalidate() {
    this.generation += 1;
    this.activeKey = "";
  }
}
