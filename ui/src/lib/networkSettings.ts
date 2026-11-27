export type NetworkProxyPreference =
  | { mode: "direct" }
  | { mode: "auto" }
  | { mode: "manual"; host: string; port: number };

const TRANSPORT_STAGES = new Set(["http_reached", "timeout", "transport_unavailable", "proxy_configuration_rejected"]);

export async function testNetworkConnection(
  invoke: (command: "application_test_network_connection", payload?: undefined) => Promise<unknown>,
): Promise<string> {
  const stage = await invoke("application_test_network_connection");
  if (typeof stage !== "string" || !TRANSPORT_STAGES.has(stage)) throw new Error("NETWORK_TEST_RESPONSE_INVALID");
  return stage;
}

export function networkProxyPreference(value: unknown): NetworkProxyPreference {
  if (!value || typeof value !== "object" || Array.isArray(value)) throw new Error("NETWORK_SETTINGS_INVALID");
  const setting = value as Record<string, unknown>;
  if (typeof setting.proxy_enabled !== "boolean" || typeof setting.proxy_url !== "string") {
    throw new Error("NETWORK_SETTINGS_INVALID");
  }
  if (!setting.proxy_enabled) return { mode: "direct" };
  if (!setting.proxy_url.trim()) return { mode: "auto" };
  let url: URL;
  try { url = new URL(setting.proxy_url); } catch { throw new Error("NETWORK_PROXY_INVALID"); }
  if (!["http:", "https:"].includes(url.protocol) || url.username || url.password
      || !url.hostname || url.pathname !== "/" || url.search || url.hash) {
    throw new Error("NETWORK_PROXY_INVALID");
  }
  const port = Number(url.port || (url.protocol === "https:" ? 443 : 80));
  if (!Number.isInteger(port) || port < 1 || port > 65535) throw new Error("NETWORK_PROXY_INVALID");
  return { mode: "manual", host: url.hostname, port };
}
