/**
 * 检索进度轮询器 (UnifiedPoller)
 * 依据 C1 契约与 W5-8 现状：
 * - 检索进度轮询：250 ms（事件驱动，由检索开始触发，终态自动停止）
 * - 任务总览轮询已收敛至 taskOverviewSource（有订阅者期间持续轮询，窗口可见 1 s / 隐藏 10 s，不空闲停）
 * - 可注入定时器依赖以便单元测试
 */

export interface PollerTimerDependencies {
  setInterval: (handler: () => void, timeout: number) => any;
  clearInterval: (handle: any) => void;
}

export const defaultTimerDependencies: PollerTimerDependencies = {
  setInterval: (handler, timeout) => setInterval(handler, timeout),
  clearInterval: (handle) => clearInterval(handle),
};

export interface UnifiedPollerOptions {
  onPollSearch: () => Promise<void> | void;
  timers?: PollerTimerDependencies;
}

export class UnifiedPoller {
  private timers: PollerTimerDependencies;
  private onPollSearch: () => Promise<void> | void;

  private searchTimerId: any = null;
  private activeSearchRequestId: string | null = null;

  constructor(options: UnifiedPollerOptions) {
    this.onPollSearch = options.onPollSearch;
    this.timers = options.timers || defaultTimerDependencies;
  }

  /**
   * 启动或更新检索轮询（250 ms）
   */
  public startSearch(requestId: string): void {
    if (!requestId) return;
    this.activeSearchRequestId = requestId;

    if (!this.searchTimerId) {
      this.searchTimerId = this.timers.setInterval(() => {
        void this.onPollSearch();
      }, 250);
    }
  }

  /**
   * 终止检索轮询
   */
  public stopSearch(): void {
    this.activeSearchRequestId = null;
    if (this.searchTimerId) {
      this.timers.clearInterval(this.searchTimerId);
      this.searchTimerId = null;
    }
  }

  public isSearching(): boolean {
    return this.searchTimerId !== null && this.activeSearchRequestId !== null;
  }

  public isIdle(): boolean {
    return this.searchTimerId === null;
  }

  public dispose(): void {
    this.stopSearch();
  }
}
