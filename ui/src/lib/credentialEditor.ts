import { t } from "../i18n/index.ts";
import type { DesktopCredentialBridge, DesktopCredentialField } from "./desktopShellBridge.ts";

export const CREDENTIAL_EDITOR_FIELDS = ["cdse_username", "cdse_password", "usgs_username", "usgs_application_token",
  "bhoonidhi_username", "bhoonidhi_password", "earthdata_username", "earthdata_password", "earthdata_token",
  "opentopography_key", "gacos_email"] as const satisfies readonly DesktopCredentialField[];
export type AccountProvider = "cdse" | "usgs" | "bhoonidhi" | "earthdata" | "opentopography" | "gacos";
export function isCredentialEditorField(value: unknown): value is DesktopCredentialField {
  return typeof value === "string" && (CREDENTIAL_EDITOR_FIELDS as readonly string[]).includes(value);
}
export function validCredentialEditorValue(value: unknown): value is string {
  return typeof value === "string" && value.length <= 1280 && !/[\u0000-\u001f\u007f-\u009f]/.test(value);
}
type EditResult = { ok: true; superseded?: true } | { ok: false; error: string };
type CheckState = "valid" | "invalid" | "unknown" | "unavailable";
type Pending = { value: string; order: number; resolve: (result: EditResult) => void };

/** A transport sequencer, not a credential cache. At most one latest queued
 * value per field and one running write; successful/failed values are discarded.
 * Kept per bridge so a remounted editor cannot read ahead of an outstanding save. */
export class CredentialEditorWriter {
  private pending = new Map<DesktopCredentialField, Pending>();
  private running: Promise<void> | null = null;
  private order = 0;
  private revisions = new Map<AccountProvider, number>();
  private checks = new Map<AccountProvider, { revision: number; state: CheckState; source?: string }>();
  // Retain only failure metadata across unmount, never a failed secret value.
  private outcomes = new Map<DesktopCredentialField, { order: number; failed: boolean }>();
  private port: Pick<DesktopCredentialBridge, "updateField" | "readForEdit">;
  constructor(port: Pick<DesktopCredentialBridge, "updateField" | "readForEdit">) { this.port = port; }
  revision(provider: AccountProvider): number { return this.revisions.get(provider) ?? 0; }
  checked(provider: AccountProvider, revision: number, result: boolean | CheckState, source?: string): void {
    if (revision === this.revision(provider)) this.checks.set(provider, {
      revision, state: typeof result === "boolean" ? result ? "valid" : "invalid" : result, source,
    });
  }
  /** Last explicit check is metadata, not a session or proof of present remote availability. */
  checkedState(provider: AccountProvider, source: string): CheckState | null {
    const check = this.checks.get(provider);
    return check?.revision === this.revision(provider) && check.source === source ? check.state : null;
  }
  needsCheck(provider: AccountProvider): boolean {
    const check = this.checks.get(provider);
    return check ? check.state !== "valid" || check.revision !== this.revision(provider) : this.revision(provider) > 0;
  }
  write(field: DesktopCredentialField, value: string): Promise<EditResult> {
    if (!isCredentialEditorField(field)) {
      return Promise.resolve({ ok: false, error: t("accounts.fields.invalidValue") });
    }
    const order = ++this.order;
    const valid = validCredentialEditorValue(value);
    this.outcomes.set(field, { order, failed: !valid });
    const provider = field.split("_")[0] as AccountProvider;
    this.revisions.set(provider, this.revision(provider) + 1);
    if (!valid) return Promise.resolve({ ok: false, error: t("accounts.fields.invalidValue") });
    const promise = new Promise<EditResult>(resolve => {
      this.pending.get(field)?.resolve({ ok: true, superseded: true });
      this.pending.set(field, { value, order, resolve });
    });
    this.start();
    return promise;
  }
  private start(): void {
    if (this.running || !this.pending.size) return;
    // A save acknowledgement may enqueue another edit before this chain's
    // finally runs. Recheck the bounded queue when releasing the running slot.
    this.running = Promise.resolve().then(() => this.drain()).finally(() => {
      this.running = null;
      this.start();
    });
  }
  private async drain(): Promise<void> {
    while (this.pending.size) {
      const [field, entry] = [...this.pending].sort((a,b) => a[1].order - b[1].order)[0]!;
      this.pending.delete(field);
      let outcome: EditResult;
      try {
        const result = await this.port.updateField(field, entry.value);
        outcome = result.ok ? { ok: true } : { ok: false, error: t("accounts.fields.autoSaveFailed") };
      } catch { outcome = { ok: false, error: t("accounts.fields.autoSaveFailed") }; }
      if (this.outcomes.get(field)?.order === entry.order) this.outcomes.set(field, { order: entry.order, failed: !outcome.ok });
      if (outcome.ok && entry.value && field.startsWith("earthdata_")) {
        const cleared: DesktopCredentialField[] = field === "earthdata_token"
          ? ["earthdata_username", "earthdata_password"] : ["earthdata_token"];
        for (const other of cleared) if ((this.outcomes.get(other)?.order ?? Infinity) < entry.order) this.outcomes.delete(other);
      }
      entry.value = "";
      entry.resolve(outcome);
    }
  }
  failedFields(fields: readonly DesktopCredentialField[]): DesktopCredentialField[] {
    return fields.filter(field => this.outcomes.get(field)?.failed);
  }
  async settled(): Promise<void> {
    while (this.running) await this.running;
  }
  async read(fields: readonly DesktopCredentialField[]): Promise<Partial<Record<DesktopCredentialField, string>>> {
    await this.settled();
    const result: Partial<Record<DesktopCredentialField, string>> = {};
    for (const field of fields) {
      const read = await this.port.readForEdit(field);
      if (!read.ok || (read.value !== null && !validCredentialEditorValue(read.value))) throw new Error(t("accounts.fields.readSavedFailed"));
      result[field] = read.value ?? "";
    }
    return result;
  }
}
const writers = new WeakMap<DesktopCredentialBridge, CredentialEditorWriter>();
export function credentialEditorWriter(port: DesktopCredentialBridge): CredentialEditorWriter {
  let writer = writers.get(port);
  if (!writer) { writer = new CredentialEditorWriter(port); writers.set(port, writer); }
  return writer;
}
