/**
 * 同一类请求只采用最后发出的那一次的结果：较早发出、较晚返回的旧响应直接丢弃。
 * 用法同 permissionReqSeqRef：每次请求取序号，返回时序号仍是最新才写入。
 */
export interface LatestResultGate {
  run<T>(fetch: () => Promise<T>, apply: (value: T) => void): Promise<void>;
}

export function createLatestResultGate(): LatestResultGate {
  let latest = 0;
  return {
    async run<T>(fetch: () => Promise<T>, apply: (value: T) => void): Promise<void> {
      const mine = ++latest;
      const value = await fetch();
      if (mine === latest) apply(value);
    },
  };
}
