const listeners = new Set<() => void>();

export function subscribeChannelsChanged(listener: () => void): () => void {
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
  };
}

export function notifyChannelsChanged(): void {
  for (const listener of [...listeners]) {
    listener();
  }
}
