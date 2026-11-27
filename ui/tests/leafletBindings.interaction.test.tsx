import assert from "node:assert/strict";
import test from "node:test";
import { JSDOM } from "jsdom";
import React, { createElement } from "react";

// 顶层优先初始化 jsdom 环境，确保后续动态加载 Leaflet 时 window 已就绪
const dom = new JSDOM("<!doctype html><html><body></body></html>", { url: "http://127.0.0.1/" });
Object.assign(globalThis, {
  window: dom.window,
  document: dom.window.document,
  HTMLElement: dom.window.HTMLElement,
  Element: dom.window.Element,
  Node: dom.window.Node,
  MutationObserver: dom.window.MutationObserver,
  ResizeObserver: class {
    observe() {}
    disconnect() {}
  },
  requestAnimationFrame: (cb: FrameRequestCallback) => setTimeout(() => cb(Date.now()), 0),
  cancelAnimationFrame: (id: number) => clearTimeout(id),
  IS_REACT_ACT_ENVIRONMENT: true,
});
Object.defineProperty(globalThis, "navigator", { configurable: true, value: dom.window.navigator });

// Leaflet 运行在 jsdom 所需的尺寸 stub
Object.defineProperty(dom.window.HTMLElement.prototype, "clientWidth", { configurable: true, value: 800 });
Object.defineProperty(dom.window.HTMLElement.prototype, "clientHeight", { configurable: true, value: 600 });
dom.window.HTMLElement.prototype.getBoundingClientRect = function () {
  return {
    top: 0,
    left: 0,
    right: 800,
    bottom: 600,
    width: 800,
    height: 600,
    x: 0,
    y: 0,
    toJSON() {},
  };
};

test("W5-3d: Leaflet Direct Bindings Suite", async (t) => {
  const { render } = await import("@testing-library/react");
  const L = (await import("leaflet")).default;
  const {
    MapContainer,
    TileLayer,
    ImageOverlay,
    Polygon,
    Polyline,
    Rectangle,
    CircleMarker,
    Pane,
    AttributionControl,
    ZoomControl,
    Tooltip,
    useMap,
    useMapEvents,
  } = await import("../src/components/map/leafletBindings.tsx");

  // 1. MapContainer 创建且卸载时销毁
  await t.test("1. MapContainer creates map on mount and calls remove on unmount", async () => {
    let capturedMap: any = null;
    function Consumer() {
      const map = useMap();
      capturedMap = map;
      return null;
    }

    const view = render(
      <MapContainer center={[30, 110]} zoom={8} style={{ width: 800, height: 600 }}>
        <Consumer />
      </MapContainer>
    );

    assert.ok(capturedMap, "子组件必须通过 useMap 获取到 map 实例");
    const container = capturedMap.getContainer();
    assert.equal(container.classList.contains("leaflet-container"), true, "容器必须挂载 leaflet class");

    // 卸载
    view.unmount();
    // 销毁后 container 不再拥有 _leaflet_id
    assert.equal(container._leaflet_id, undefined, "卸载后必须调用 map.remove()");
  });

  // 2. TileLayer 添加、卸载与 url 动态更新
  await t.test("2. TileLayer is added to map, updates url via setUrl, and removes on unmount", async () => {
    let capturedMap: any = null;
    function Consumer({ url }: { url: string }) {
      const map = useMap();
      capturedMap = map;
      return <TileLayer url={url} zIndex={10} />;
    }

    const view = render(
      <MapContainer center={[30, 110]} zoom={8} style={{ width: 800, height: 600 }}>
        <Consumer url="https://tile.example.com/{z}/{x}/{y}.png" />
      </MapContainer>
    );

    assert.ok(capturedMap);
    const map = capturedMap;

    let tileLayers: any[] = [];
    map.eachLayer((l: any) => {
      if (l instanceof L.TileLayer) tileLayers.push(l);
    });
    assert.equal(tileLayers.length, 1, "必须添加 1 个 TileLayer 到 map");
    const initialLayer = tileLayers[0];
    assert.equal(initialLayer._url, "https://tile.example.com/{z}/{x}/{y}.png");

    // 更新 url
    view.rerender(
      <MapContainer center={[30, 110]} zoom={8} style={{ width: 800, height: 600 }}>
        <Consumer url="https://tile2.example.com/{z}/{x}/{y}.png" />
      </MapContainer>
    );

    tileLayers = [];
    map.eachLayer((l: any) => {
      if (l instanceof L.TileLayer) tileLayers.push(l);
    });
    assert.equal(tileLayers.length, 1);
    assert.equal(tileLayers[0], initialLayer, "更新 url 不得重新创建图层实例");
    assert.equal(tileLayers[0]._url, "https://tile2.example.com/{z}/{x}/{y}.png");

    // 卸载
    view.unmount();
    assert.equal(map.hasLayer(initialLayer), false, "卸载后图层必须从地图移除");
  });

  // 3. Polygon positions 与 pathOptions 更新
  await t.test("3. Polygon updates positions via setLatLngs and pathOptions via setStyle", async () => {
    let capturedMap: any = null;
    function Consumer({
      positions,
      color,
    }: {
      positions: any;
      color: string;
    }) {
      const map = useMap();
      capturedMap = map;
      return <Polygon positions={positions} pathOptions={{ color }} />;
    }

    const pos1 = [
      [30, 110],
      [30, 111],
      [31, 111],
    ];
    const pos2 = [
      [32, 112],
      [32, 113],
      [33, 113],
    ];

    const view = render(
      <MapContainer center={[30, 110]} zoom={8} style={{ width: 800, height: 600 }}>
        <Consumer positions={pos1} color="#ff0000" />
      </MapContainer>
    );

    assert.ok(capturedMap);
    const map = capturedMap;

    let poly: any = null;
    map.eachLayer((l: any) => {
      if (l instanceof L.Polygon) poly = l;
    });
    assert.ok(poly, "必须创建并挂载 Polygon");
    assert.equal(poly.options.color, "#ff0000");

    // 更新 positions 与 color
    view.rerender(
      <MapContainer center={[30, 110]} zoom={8} style={{ width: 800, height: 600 }}>
        <Consumer positions={pos2} color="#00ff00" />
      </MapContainer>
    );

    assert.equal(poly.options.color, "#00ff00", "setStyle 必须更新样式");
    const latlngs = poly.getLatLngs()[0];
    assert.equal(Math.round(latlngs[0].lat), 32);

    view.unmount();
    assert.equal(map.hasLayer(poly), false, "卸载后 Polygon 必须移除");
  });

  // 4. 事件监听器替换（旧处理器注销，新处理器响应）
  await t.test("4. eventHandlers swap ensures latest handler is called and old is detached", async () => {
    let capturedMap: any = null;
    let oldCalled = 0;
    let newCalled = 0;

    function Consumer({ onPolygonClick }: { onPolygonClick: () => void }) {
      const map = useMap();
      capturedMap = map;
      return (
        <Polygon
          positions={[
            [30, 110],
            [30, 111],
            [31, 111],
          ]}
          eventHandlers={{ click: onPolygonClick }}
        />
      );
    }

    const view = render(
      <MapContainer center={[30, 110]} zoom={8} style={{ width: 800, height: 600 }}>
        <Consumer onPolygonClick={() => oldCalled++} />
      </MapContainer>
    );

    assert.ok(capturedMap);
    const map = capturedMap;
    let poly: any = null;
    map.eachLayer((l: any) => {
      if (l instanceof L.Polygon) poly = l;
    });
    assert.ok(poly);

    // 触发点击 1
    poly.fire("click");
    assert.equal(oldCalled, 1);
    assert.equal(newCalled, 0);

    // 切换为新回调
    view.rerender(
      <MapContainer center={[30, 110]} zoom={8} style={{ width: 800, height: 600 }}>
        <Consumer onPolygonClick={() => newCalled++} />
      </MapContainer>
    );

    // 再次触发点击
    poly.fire("click");
    assert.equal(oldCalled, 1, "旧回调不应再被调用");
    assert.equal(newCalled, 1, "新回调必须被调用");

    view.unmount();
  });

  // 5. Pane 创建与子图层 pane 选项继承
  await t.test("5. Pane creates pane with style and child layer inherits pane name", async () => {
    let capturedMap: any = null;

    function Consumer() {
      const map = useMap();
      capturedMap = map;
      return (
        <Pane name="custom-test-pane" style={{ zIndex: 450 }}>
          <Polygon
            positions={[
              [30, 110],
              [30, 111],
              [31, 111],
            ]}
          />
        </Pane>
      );
    }

    const view = render(
      <MapContainer center={[30, 110]} zoom={8} style={{ width: 800, height: 600 }}>
        <Consumer />
      </MapContainer>
    );

    assert.ok(capturedMap);
    const map = capturedMap;
    const paneEl = map.getPane("custom-test-pane");
    assert.ok(paneEl, "必须创建 custom-test-pane");
    assert.equal(paneEl.style.zIndex, "450");

    let poly: any = null;
    map.eachLayer((l: any) => {
      if (l instanceof L.Polygon) poly = l;
    });
    assert.ok(poly);
    assert.equal(poly.options.pane, "custom-test-pane", "子图层必须继承父 Pane 名称");

    view.unmount();
    // 卸载后 pane 元素被移除
    assert.equal(paneEl.parentNode, null, "卸载后 pane DOM 必须被移除");
  });

  // 6. useMapEvents 地图事件
  await t.test("6. useMapEvents subscribes to map events and fires callback", async () => {
    let clickCount = 0;
    let capturedMap: any = null;

    function EventConsumer() {
      const map = useMapEvents({
        click: () => {
          clickCount++;
        },
      });
      capturedMap = map;
      return null;
    }

    const view = render(
      <MapContainer center={[30, 110]} zoom={8} style={{ width: 800, height: 600 }}>
        <EventConsumer />
      </MapContainer>
    );

    assert.ok(capturedMap);
    const map = capturedMap;
    map.fire("click");
    assert.equal(clickCount, 1, "useMapEvents 绑定的 click 事件必须被触发");

    view.unmount();
    map.fire("click");
    assert.equal(clickCount, 1, "卸载后不应再触发事件");
  });

  // 7. AttributionControl 与 ZoomControl 添加和移除
  await t.test("7. Controls are mounted and removed cleanly", async () => {
    let capturedMap: any = null;

    function ControlsConsumer() {
      const map = useMap();
      capturedMap = map;
      return (
        <>
          <AttributionControl prefix="ScenePort" />
          <ZoomControl position="bottomright" />
        </>
      );
    }

    const view = render(
      <MapContainer center={[30, 110]} zoom={8} attributionControl={false} zoomControl={false} style={{ width: 800, height: 600 }}>
        <ControlsConsumer />
      </MapContainer>
    );

    assert.ok(capturedMap);
    const map = capturedMap;
    const container = map.getContainer();

    const zoomEl = container.querySelector(".leaflet-control-zoom");
    assert.ok(zoomEl, "ZoomControl 必须存在于地图容器内");

    const attribEl = container.querySelector(".leaflet-control-attribution");
    assert.ok(attribEl, "AttributionControl 必须存在于地图容器内");

    view.unmount();
  });

  // 8. Tooltip 绑定到矢量图层
  await t.test("8. Tooltip binds content to parent vector layer and unbinds on unmount", async () => {
    let capturedMap: any = null;

    function Consumer({ text }: { text: string }) {
      const map = useMap();
      capturedMap = map;
      return (
        <CircleMarker center={[30, 110]} radius={5}>
          <Tooltip>{text}</Tooltip>
        </CircleMarker>
      );
    }

    const view = render(
      <MapContainer center={[30, 110]} zoom={8} style={{ width: 800, height: 600 }}>
        <Consumer text="区域中心" />
      </MapContainer>
    );

    assert.ok(capturedMap);
    const map = capturedMap;
    let circle: any = null;
    map.eachLayer((l: any) => {
      if (l instanceof L.CircleMarker) circle = l;
    });
    assert.ok(circle);
    assert.ok(circle.getTooltip(), "CircleMarker 必须绑定 Tooltip");
    assert.equal(circle.getTooltip()?.getContent(), "区域中心");

    view.unmount();
    assert.equal(circle._tooltip, null, "卸载后 Tooltip 必须被 unbind");
  });

  // 9. Pane 卸载后从 map 注销，再次挂载时创建全新 attached 面板与子图层容器
  await t.test("9. Pane unmount unregisters from map so remount creates fresh attached pane", async () => {
    let capturedMap: any = null;

    function TestApp({ showPane }: { showPane: boolean }) {
      const map = useMap();
      capturedMap = map;
      return (
        <>
          {showPane && (
            <Pane name="p1" style={{ zIndex: 450 }}>
              <TileLayer url="https://tile.example.com/{z}/{x}/{y}.png" />
            </Pane>
          )}
        </>
      );
    }

    const { rerender } = render(
      <MapContainer center={[30, 110]} zoom={8} style={{ width: 800, height: 600 }}>
        <TestApp showPane={true} />
      </MapContainer>
    );

    assert.ok(capturedMap);
    const map = capturedMap;
    const mapContainer = map.getContainer();

    const firstPane = map.getPane("p1");
    assert.ok(firstPane);
    assert.equal(mapContainer.contains(firstPane), true, "第一次挂载时 p1 必须 attached 在地图容器内");

    // 卸载 Pane
    rerender(
      <MapContainer center={[30, 110]} zoom={8} style={{ width: 800, height: 600 }}>
        <TestApp showPane={false} />
      </MapContainer>
    );
    assert.equal(mapContainer.contains(firstPane), false, "卸载后旧 pane 必须脱离 DOM");

    // 再次挂载 Pane
    rerender(
      <MapContainer center={[30, 110]} zoom={8} style={{ width: 800, height: 600 }}>
        <TestApp showPane={true} />
      </MapContainer>
    );

    const secondPane = map.getPane("p1");
    assert.ok(secondPane, "重新挂载后必须能获取到 p1");
    assert.equal(mapContainer.contains(secondPane), true, "重新挂载后 p1 必须 attached 在地图容器内");

    let tileLayerContainer: HTMLElement | null = null;
    map.eachLayer((l: any) => {
      if (l instanceof L.TileLayer) {
        tileLayerContainer = l.getContainer();
      }
    });
    assert.ok(tileLayerContainer, "子图层 container 必须存在");
    assert.equal(secondPane.contains(tileLayerContainer), true, "子图层 container 必须在第二任 pane 内部");
  });

  // 10. ImageOverlay clipPath 在挂载即生效并在属性更新时同步
  await t.test("10. ImageOverlay applies clipPath on mount and syncs on prop update", async () => {
    let capturedMap: any = null;

    function TestOverlay({ clipPath }: { clipPath: string | null }) {
      const map = useMap();
      capturedMap = map;
      return (
        <ImageOverlay
          url="data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8BQDwAEhQGAhKmMIQAAAABJRU5ErkJggg=="
          bounds={[
            [30, 116],
            [32, 118],
          ]}
          opacity={1}
          clipPath={clipPath}
        />
      );
    }

    const firstClip = "polygon(10% 0%, 100% 10%, 90% 100%, 0% 90%)";
    const secondClip = "polygon(0% 0%, 100% 0%, 100% 100%)";

    const view = render(
      <MapContainer center={[31, 117]} zoom={8} style={{ width: 800, height: 600 }}>
        <TestOverlay clipPath={firstClip} />
      </MapContainer>
    );

    assert.ok(capturedMap);
    let overlay: any = null;
    capturedMap.eachLayer((l: any) => {
      if (l instanceof L.ImageOverlay) overlay = l;
    });
    assert.ok(overlay, "ImageOverlay 图层必须存在");
    const img = overlay.getElement() as HTMLImageElement | null;
    assert.ok(img, "ImageOverlay 图像 DOM 元素必须已挂载");
    assert.equal(img.style.clipPath, firstClip);

    view.rerender(
      <MapContainer center={[31, 117]} zoom={8} style={{ width: 800, height: 600 }}>
        <TestOverlay clipPath={secondClip} />
      </MapContainer>
    );
    assert.equal(img.style.clipPath, secondClip);

    view.rerender(
      <MapContainer center={[31, 117]} zoom={8} style={{ width: 800, height: 600 }}>
        <TestOverlay clipPath={null} />
      </MapContainer>
    );
    assert.equal(img.style.clipPath, "");

    view.unmount();
  });
});
