import assert from "node:assert/strict";
import test from "node:test";
import type { SceneRow } from "../../lib/bridge.ts";
import { groupScenesByStack, sceneStackGroupKey } from "./resultGrouping.ts";
import { sortKeysFor, sortScenes } from "./resultListLogic.ts";

function makeScene(overrides: Partial<SceneRow>): SceneRow {
  return {
    scene_id: "s-1",
    file_name: "file.zip",
    acquisition_datetime: "2026-11-01T00:00:00Z",
    file_size_remote: 1000,
    ...overrides,
  };
}

test("groupScenesByStack groups SAR Path-Frame and optical MGRS scenes consistently", () => {
  const scenes: SceneRow[] = [
    makeScene({
      scene_id: "S1A_IW_SLC__1SDV_20261101",
      file_name: "S1A_IW_SLC__1SDV_20261101.zip",
      orbit_direction: "ASCENDING",
      relative_orbit: 84,
      frame: 70,
    }),
    makeScene({
      scene_id: "S2A_MSIL2A_20261102T030000_N0511_R032_T50SNA_20261102",
      file_name: "S2A_MSIL2A_20261102T030000_N0511_R032_T50SNA_20261102.SAFE",
      native_metadata: { properties: { cloud_cover: 8.2 } },
    }),
    makeScene({
      scene_id: "S1D_IW_SLC__1SDV_20261020",
      file_name: "S1D_IW_SLC__1SDV_20261020.zip",
      orbit_direction: "ASCENDING",
      relative_orbit: 84,
      frame: 70,
    }),
  ];

  const bySize = groupScenesByStack(scenes, { sortByStackSize: true });
  assert.equal(bySize.length, 2);
  assert.equal(bySize[0].items.length, 2);
  assert.equal(bySize[1].items.length, 1);
  assert.equal(bySize[0].key, sceneStackGroupKey(scenes[0]));
  assert.equal(bySize[0].key, sceneStackGroupKey(scenes[2]));

  const insertionOrder = groupScenesByStack(scenes, { sortByStackSize: false });
  assert.equal(insertionOrder.length, 2);
  assert.equal(insertionOrder[0].items.length, 2);
});

test("resultListLogic supports mixed multi-source sorting with missing values anchored last", () => {
  const scenes: SceneRow[] = [
    makeScene({ scene_id: "c", file_size_remote: undefined }),
    makeScene({
      scene_id: "a",
      file_size_remote: 500,
      platform: "Sentinel-2",
      native_metadata: { properties: { cloud_cover: 15 } },
    }),
    makeScene({
      scene_id: "b",
      file_size_remote: 200,
      platform: "Landsat",
      native_metadata: { properties: { cloud_cover: 3 } },
    }),
  ];

  const mixedKeys = sortKeysFor("mixed", true, {
    hasOpticalKeys: true,
    hasSarKeys: true,
    hasSourceKey: true,
  });
  assert.deepEqual(
    mixedKeys.map((k) => k.key),
    ["date", "cloud", "relativeOrbit", "frame", "source", "size"]
  );

  const ascSize = sortScenes(scenes, "size", "asc");
  assert.deepEqual(ascSize.map((s) => s.scene_id), ["b", "a", "c"]);

  const descSize = sortScenes(scenes, "size", "desc");
  assert.deepEqual(descSize.map((s) => s.scene_id), ["a", "b", "c"]);

  const ascCloud = sortScenes(scenes, "cloud", "asc");
  assert.deepEqual(ascCloud.map((s) => s.scene_id), ["b", "a", "c"]);

  const ascSource = sortScenes(scenes, "source", "asc");
  assert.deepEqual(ascSource.map((s) => s.scene_id), ["b", "a", "c"]);
});
