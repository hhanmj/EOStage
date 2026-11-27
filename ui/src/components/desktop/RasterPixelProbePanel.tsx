import React, { useEffect, useState } from "react";
import { Crosshair } from "lucide-react";
import { probeGisRasterPixel, type RasterPixelProbeResult } from "../../lib/bridge.ts";

export interface RasterPixelProbePanelProps {
  sourcePath: string;
  defaultLon?: number | null;
  defaultLat?: number | null;
}

function summarizeCrs(crs: string | null): string {
  if (!crs || !crs.trim()) return "未知坐标系";
  const match = crs.match(/^(?:PROJCS|PROJCRS|GEOGCS|GEOGCRS)\["([^"]+)"/i);
  if (match && match[1]) {
    return match[1];
  }
  return crs.length > 48 ? `${crs.slice(0, 48)}…` : crs;
}

export function RasterPixelProbePanel({
  sourcePath,
  defaultLon,
  defaultLat,
}: RasterPixelProbePanelProps) {
  const formatCoord = (val: number | null | undefined, fallback: string) =>
    typeof val === "number" && Number.isFinite(val) ? val.toFixed(5) : fallback;

  const [lonInput, setLonInput] = useState<string>(() => formatCoord(defaultLon, "117.22720"));
  const [latInput, setLatInput] = useState<string>(() => formatCoord(defaultLat, "31.82060"));
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [result, setResult] = useState<RasterPixelProbeResult | null>(null);

  useEffect(() => {
    setLonInput(formatCoord(defaultLon, "117.22720"));
    setLatInput(formatCoord(defaultLat, "31.82060"));
  }, [sourcePath, defaultLon, defaultLat]);

  const handleProbe = async () => {
    const lon = Number.parseFloat(lonInput.trim());
    const lat = Number.parseFloat(latInput.trim());
    if (!Number.isFinite(lon) || !Number.isFinite(lat) || Math.abs(lon) > 180 || Math.abs(lat) > 90) {
      setError("请输入有效的 WGS84 经纬度坐标（经度 -180~180，纬度 -90~90）");
      setResult(null);
      return;
    }

    setLoading(true);
    setError(null);
    try {
      const probe = await probeGisRasterPixel(sourcePath, lon, lat);
      if (!probe) {
        setError("当前运行环境暂不支持本地像元探针查询");
        setResult(null);
      } else {
        setResult(probe);
      }
    } catch (err: unknown) {
      const msg =
        err instanceof Error
          ? err.message
          : typeof err === "string"
            ? err
            : "像元探针查询失败";
      setError(msg);
      setResult(null);
    } finally {
      setLoading(false);
    }
  };

  return (
    <div
      data-testid="raster-pixel-probe-panel"
      className="mb-1 rounded-sp-sm border border-sp-line bg-sp-surface px-2.5 py-2 text-xs space-y-2"
    >
      <div className="flex flex-wrap items-center justify-between gap-2">
        <div className="flex items-center gap-1.5 font-semibold text-sp-text">
          <Crosshair className="h-3.5 w-3.5 text-sp-accent shrink-0" />
          <span>物理像元探针 (Pixel Identify)</span>
        </div>
        <div className="flex flex-wrap items-center gap-1.5">
          <label className="inline-flex items-center gap-1 text-sp-text-secondary">
            <span>经度</span>
            <input
              type="text"
              inputMode="decimal"
              aria-label="像元探针经度"
              data-testid="pixel-probe-lon-input"
              value={lonInput}
              onChange={(e) => setLonInput(e.target.value)}
              className="w-24 rounded-sp-sm border border-sp-line bg-sp-surface-muted px-1.5 py-0.5 font-mono text-xs text-sp-text focus:outline-none focus:border-sp-accent"
            />
          </label>
          <label className="inline-flex items-center gap-1 text-sp-text-secondary">
            <span>纬度</span>
            <input
              type="text"
              inputMode="decimal"
              aria-label="像元探针纬度"
              data-testid="pixel-probe-lat-input"
              value={latInput}
              onChange={(e) => setLatInput(e.target.value)}
              className="w-24 rounded-sp-sm border border-sp-line bg-sp-surface-muted px-1.5 py-0.5 font-mono text-xs text-sp-text focus:outline-none focus:border-sp-accent"
            />
          </label>
          <button
            type="button"
            data-testid="pixel-probe-run-btn"
            disabled={loading}
            onClick={() => void handleProbe()}
            className="inline-flex items-center gap-1 rounded-sp-sm border border-sp-line bg-sp-surface-muted px-2 py-0.5 text-xs font-semibold text-sp-accent hover:bg-sp-hover disabled:opacity-50 cursor-pointer"
          >
            <span>{loading ? "查询中..." : "像元探针查询"}</span>
          </button>
        </div>
      </div>

      {error && (
        <div role="alert" className="text-sp-danger-text">
          {error}
        </div>
      )}

      {result && (
        <div
          data-testid="pixel-probe-results"
          className="rounded-sp-sm border border-sp-line bg-sp-surface-muted/70 px-2 py-1.5 space-y-1"
        >
          <div className="flex flex-wrap items-center justify-between gap-2 text-xs text-sp-text-secondary font-mono">
            <span>
              像元坐标: ({result.col}, {result.row}) · 尺寸 {result.width} × {result.height}
            </span>
            <span>坐标系: {summarizeCrs(result.crs)}</span>
          </div>

          {!result.in_bounds ? (
            <div className="text-sp-warning-text">
              查询坐标 ({result.lon.toFixed(5)}, {result.lat.toFixed(5)}) 超出当前栅格覆盖范围
            </div>
          ) : (
            <div className="space-y-0.5">
              {result.bands.map((band) => {
                const isScaled = band.scale !== 1 || band.offset !== 0;
                return (
                  <div
                    key={band.band}
                    className="flex flex-wrap items-center justify-between gap-2 text-xs font-mono text-sp-text"
                  >
                    <span className="font-semibold">
                      B{band.band} ({band.name || band.dtype})
                    </span>
                    <div className="flex flex-wrap items-center gap-2">
                      {band.is_nodata ? (
                        <span className="rounded bg-sp-warning-bg px-1.5 py-0.5 text-sp-warning-text">
                          NoData
                        </span>
                      ) : (
                        <span>
                          {band.physical_value !== null ? band.physical_value.toFixed(4) : "—"}
                          {isScaled && band.raw_value !== null ? ` (raw ${band.raw_value})` : ""}
                        </span>
                      )}
                      {band.qa_semantics && (
                        <span className="rounded bg-sp-accent/10 px-1.5 py-0.5 text-sp-accent-strong">
                          {band.qa_semantics}
                        </span>
                      )}
                    </div>
                  </div>
                );
              })}
            </div>
          )}
        </div>
      )}
    </div>
  );
}
