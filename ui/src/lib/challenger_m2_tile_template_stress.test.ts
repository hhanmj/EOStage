import "../test-utils/setup-dom.ts";
import assert from "node:assert/strict";
import test from "node:test";
import L from "leaflet";

function tileUrlWithUserToken(template: string, token: string): string {
  return template.replaceAll("{token}", encodeURIComponent(token.trim()));
}

test("Leaflet 1.9 tileLayer parses standard GEE XYZ tile templates", () => {
  const projectId = "ee-sceneport-test";
  const mapId = "0123456789abcdef";
  const template = `https://earthengine.googleapis.com/v1/projects/${projectId}/maps/${mapId}/tiles/{z}/{x}/{y}`;

  const container = document.createElement("div");
  const map = L.map(container).setView([31.86, 117.28], 10);

  const layer = L.tileLayer(template, {
    attribution: "Google Earth Engine",
    maxZoom: 22,
    minZoom: 0,
  });
  layer.addTo(map);

  assert.equal(layer.getAttribution(), "Google Earth Engine");

  // At zoom 10
  const urlZ10 = layer.getTileUrl(Object.assign(L.point(542, 388), { z: 10 }) as any);
  assert.equal(
    urlZ10,
    `https://earthengine.googleapis.com/v1/projects/${projectId}/maps/${mapId}/tiles/10/542/388`
  );

  // Set zoom to 18
  map.setZoom(18);
  const urlZ18 = layer.getTileUrl(Object.assign(L.point(105421, 89432), { z: 18 }) as any);
  assert.equal(
    urlZ18,
    `https://earthengine.googleapis.com/v1/projects/${projectId}/maps/${mapId}/tiles/18/105421/89432`
  );

  // Set zoom to 0
  map.setZoom(0);
  const urlZ0 = layer.getTileUrl(Object.assign(L.point(0, 0), { z: 0 }) as any);
  assert.equal(
    urlZ0,
    `https://earthengine.googleapis.com/v1/projects/${projectId}/maps/${mapId}/tiles/0/0/0`
  );

  map.remove();
});

test("Leaflet 1.9 tileLayer preserves query string tokens and parameters", () => {
  const projectId = "ee-test";
  const mapId = "map98765";
  const token = "ya29.a0AWY7CkkL983jd8_SECRET_TOKEN";
  const templateWithToken = `https://earthengine.googleapis.com/v1/projects/${projectId}/maps/${mapId}/tiles/{z}/{x}/{y}?token=${token}`;

  const container = document.createElement("div");
  const map = L.map(container).setView([0, 0], 8);

  const layer = L.tileLayer(templateWithToken);
  layer.addTo(map);

  const tileUrl = layer.getTileUrl(Object.assign(L.point(12, 34), { z: 8 }) as any);

  assert.equal(
    tileUrl,
    `https://earthengine.googleapis.com/v1/projects/${projectId}/maps/${mapId}/tiles/8/12/34?token=${token}`
  );
  assert.ok(!tileUrl.includes("{z}"), "No remaining {z}");
  assert.ok(!tileUrl.includes("{x}"), "No remaining {x}");
  assert.ok(!tileUrl.includes("{y}"), "No remaining {y}");

  map.remove();
});

test("Leaflet tileLayer dynamic URL update (setUrl) functions correctly", () => {
  const container = document.createElement("div");
  const map = L.map(container).setView([0, 0], 3);

  const layer = L.tileLayer("https://example.com/tiles/v1/{z}/{x}/{y}");
  layer.addTo(map);

  const initial = layer.getTileUrl(Object.assign(L.point(1, 2), { z: 3 }) as any);
  assert.equal(initial, "https://example.com/tiles/v1/3/1/2");

  layer.setUrl("https://earthengine.googleapis.com/v1/projects/new-proj/maps/new-map/tiles/{z}/{x}/{y}");
  const updated = layer.getTileUrl(Object.assign(L.point(1, 2), { z: 3 }) as any);
  assert.equal(
    updated,
    "https://earthengine.googleapis.com/v1/projects/new-proj/maps/new-map/tiles/3/1/2"
  );

  map.remove();
});

test("tileUrlWithUserToken handles special characters, spaces, and encoding", () => {
  const template = "https://t{s}.tianditu.gov.cn/DataServer?T=img_w&x={x}&y={y}&l={z}&tk={token}";

  // Standard token
  const t1 = tileUrlWithUserToken(template, "token123");
  assert.equal(
    t1,
    "https://t{s}.tianditu.gov.cn/DataServer?T=img_w&x={x}&y={y}&l={z}&tk=token123"
  );

  // Token with padding spaces
  const t2 = tileUrlWithUserToken(template, "   paddedToken   ");
  assert.equal(
    t2,
    "https://t{s}.tianditu.gov.cn/DataServer?T=img_w&x={x}&y={y}&l={z}&tk=paddedToken"
  );

  // Token with URL reserved characters
  const t3 = tileUrlWithUserToken(template, "abc+def/ghi=jkl?mno&pqr");
  assert.equal(
    t3,
    "https://t{s}.tianditu.gov.cn/DataServer?T=img_w&x={x}&y={y}&l={z}&tk=abc%2Bdef%2Fghi%3Djkl%3Fmno%26pqr"
  );

  // Multi-occurrence token template
  const multiTemplate = "https://server.org/{token}/tiles/{z}/{x}/{y}?auth={token}";
  const t4 = tileUrlWithUserToken(multiTemplate, "my-token");
  assert.equal(
    t4,
    "https://server.org/my-token/tiles/{z}/{x}/{y}?auth=my-token"
  );
});
