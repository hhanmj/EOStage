import { Fragment, type ReactNode } from "react";

import type { Json, SceneRow } from "@/lib/bridge";
import { sceneProductKey } from "@/lib/sarProduct";
import { sceneFootprintRenderModel, type SceneFootprintRenderModel } from "@/lib/sceneFootprint";
import { cn } from "@/lib/utils";

export type SceneFootprintPart = {
  key: string;
  positions: [number, number][][];
  className: string;
  dataSceneFootprint: "true";
  selected: boolean;
  onClick: () => void;
};

/**
 * The map's scene-only layer boundary. It admits only the validated frozen
 * geometry and delegates the map-engine primitive without creating bbox shape
 * fallbacks. Tests can execute the same React selection/marker behavior with a
 * small DOM renderer.
 */
export function SceneFootprintLayer({
  scene,
  footprint = sceneFootprintRenderModel(scene.footprint_geojson),
  renderGeometry,
  selected,
  drawActive,
  onSceneSelect,
  renderPart,
}: {
  scene: SceneRow;
  footprint?: SceneFootprintRenderModel | null;
  /** Optional render-only geometry; scene identity and authoritative GeoJSON stay unchanged. */
  renderGeometry?: Json | null;
  selected: boolean;
  drawActive: boolean;
  onSceneSelect?: (sceneId: string) => void;
  renderPart: (part: SceneFootprintPart) => ReactNode;
}) {
  const renderedFootprint = renderGeometry
    ? sceneFootprintRenderModel(scene.footprint_geojson, renderGeometry)
    : footprint;
  if (!renderedFootprint) return null;
  const identity = sceneProductKey(scene);
  const className = cn("scene-footprint", selected && "scene-footprint--selected");
  return (
    <>
      {renderedFootprint.polygons.map((positions, part) => (
        <Fragment key={`${identity}:poly:${part}`}>
          {renderPart({
            key: `${identity}:poly:${part}`,
            positions,
            className,
            dataSceneFootprint: "true",
            selected,
            onClick: () => {
              if (!drawActive) onSceneSelect?.(identity);
            },
          })}
        </Fragment>
      ))}
    </>
  );
}
