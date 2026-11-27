export type BaseTileRuntime = {
  status: "loading" | "ready" | "failed";
  loadedCount: number;
  errorCount: number;
};

export function initialBaseTileRuntime(): BaseTileRuntime {
  return { status: "loading", loadedCount: 0, errorCount: 0 };
}

export function baseTileOnTileload(rt: BaseTileRuntime): BaseTileRuntime {
  return {
    status: "ready",
    loadedCount: rt.loadedCount + 1,
    errorCount: rt.errorCount,
  };
}

export function baseTileOnTileerror(rt: BaseTileRuntime): BaseTileRuntime {
  return {
    status: rt.loadedCount > 0 ? "ready" : "failed",
    loadedCount: rt.loadedCount,
    errorCount: rt.errorCount + 1,
  };
}

export function baseTileOnLoadEvent(rt: BaseTileRuntime): BaseTileRuntime {
  if (rt.loadedCount > 0) {
    return {
      ...rt,
      status: "ready",
    };
  }
  return rt;
}
