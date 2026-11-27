import "../../test-utils/setup-dom";
import assert from "node:assert/strict";
import test from "node:test";
import React from "react";
import { render, cleanup } from "../../test-utils/render-harness";

Object.assign(globalThis, {
  Element: window.Element,
  requestAnimationFrame: (cb: FrameRequestCallback) =>
    setTimeout(() => cb(Date.now()), 0) as unknown as number,
  cancelAnimationFrame: (id: number) => clearTimeout(id),
});

test.afterEach(() => {
  cleanup();
});

test("W5-6 R4 MapContainer 动态 className 切换保留 leaflet-container", async (t) => {
  const { MapContainer, useMap } = await import("./leafletBindings.tsx");

  await t.test(
    "render MapContainer(className=\"base\") 含 leaflet-container；rerender className=\"base cursor-x\" 后 leaflet-container 仍在且外层 div 更新 className；unmount 正常",
    () => {
      let capturedMap: ReturnType<typeof useMap> | null = null;
      function MapProbe() {
        capturedMap = useMap();
        return null;
      }

      const view = render(
        <MapContainer center={[30, 110]} zoom={8} className="base">
          <MapProbe />
        </MapContainer>,
      );

      assert.ok(capturedMap, "应获取到 Leaflet map 实例");
      const leafletContainer = capturedMap!.getContainer();
      assert.equal(
        leafletContainer.classList.contains("leaflet-container"),
        true,
        "初始挂载时地图容器必须包含 leaflet-container 类",
      );

      view.rerender(
        <MapContainer center={[30, 110]} zoom={8} className="base cursor-x">
          <MapProbe />
        </MapContainer>,
      );

      assert.equal(
        leafletContainer.classList.contains("leaflet-container"),
        true,
        "rerender 更新 className 后地图容器必须仍保留 leaflet-container 类",
      );

      const outerWrapper = view.container.firstElementChild as HTMLElement | null;
      assert.ok(outerWrapper, "外层包装 div 应存在");
      assert.equal(
        outerWrapper!.classList.contains("base") &&
          outerWrapper!.classList.contains("cursor-x"),
        true,
        "React 侧动态 className 更新应落在外层 div 上",
      );
      assert.notEqual(
        outerWrapper,
        leafletContainer,
        "外层 React className 容器与内层 Leaflet 容器应分离",
      );

      view.unmount();
      assert.equal(
        (leafletContainer as unknown as { _leaflet_id?: number })._leaflet_id,
        undefined,
        "unmount 后应正常销毁 Leaflet map 实例",
      );
    },
  );
});
