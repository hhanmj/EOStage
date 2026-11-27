import type { SarProductRow, SceneRow } from "./bridge.ts";

export function compactSearchWorkbenchRows(
  scenes: SceneRow[],
): Array<SceneRow | SarProductRow> {
  // Online SAR rows already carry the canonical product. Persist that single
  // record instead of serialising the duplicate legacy Scene projection too.
  // Imported legacy rows remain readable without changing their contract.
  return scenes.map((scene) => scene.sar_product ?? scene);
}
