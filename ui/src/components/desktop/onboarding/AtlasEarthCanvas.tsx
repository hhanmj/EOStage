import React, { useEffect, useRef } from "react";
import type { Locale } from "@/i18n";
import {
  classifyOceanName,
  formatLocalizedCountryName,
  GLOBE_ASSET_URLS,
  type CountrySummaryRecord,
} from "./onboardingAtlasData";

interface CompactCountryPolygon extends CountrySummaryRecord {
  polygons: number[][][][];
}

export interface HoverTargetInfo {
  kind: "country" | "ocean";
  id: number;
  iso3?: string;
  name: string;
  lat: number;
  lon: number;
  screenX: number;
  screenY: number;
}

export interface GlobeClickPick {
  kind: "country" | "ocean";
  iso3?: string;
  name: string;
  lat: number;
  lon: number;
}

export interface GlobeLayoutMetrics {
  globeCx: number;
  globeCy: number;
  globeR: number;
  cssW: number;
  cssH: number;
}

export interface AtlasEarthCanvasProps {
  locale: Locale;
  panelOpen: boolean;
  selectedCountryIso3: string | null;
  flyToRequest: { lat: number; lon: number; seq: number } | null;
  onReady: (countries: readonly CountrySummaryRecord[]) => void;
  onHoverChange: (info: HoverTargetInfo | null) => void;
  onSelectLocation: (pick: GlobeClickPick) => void;
  onLayoutMetrics?: (metrics: GlobeLayoutMetrics) => void;
}

const AXIAL_TILT = -0.409; // 23.4 deg Earth axial tilt
const AUTO_SPIN_RAD_PER_SEC = 0.085; // graceful continuous 3D Earth rotation (~4.9 deg/s)
const HOVER_COUNTRY_SPIN_RAD_PER_SEC = 0.020; // gently slows on country hover for effortless selection
const DRAG_SENS = 0.005;
const MOMENTUM_DECAY = 0.94;
const PITCH_CLAMP = 1.52; // ~87.1 deg vertical tilt freedom near poles without gimbal flip
const INITIAL_SPIN_Y = 4.0; // Old World: Africa · Europe · Middle East · Asia
const ID_MAP_W = 4096;
const ID_MAP_H = 2048;

const clamp = (v: number, lo: number, hi: number) => Math.max(lo, Math.min(hi, v));

function latLonToLocalUnit(latDeg: number, lonDeg: number): [number, number, number] {
  const lat = (latDeg * Math.PI) / 180;
  const lon = (lonDeg * Math.PI) / 180;
  const cl = Math.cos(lat);
  return [cl * Math.cos(lon), Math.sin(lat), -cl * Math.sin(lon)];
}

function buildGlobeModelMatrix(
  tx: number,
  ty: number,
  rx: number,
  rz: number,
  ry: number,
  scale: number
): Float32Array {
  const cy = Math.cos(ry);
  const sy = Math.sin(ry);
  const cz = Math.cos(rz);
  const sz = Math.sin(rz);
  const cx = Math.cos(rx);
  const sx = Math.sin(rx);

  const r00 = cz * cy;
  const r01 = -sz;
  const r02 = cz * sy;
  const r10 = sz * cy;
  const r11 = cz;
  const r12 = sz * sy;
  const r20 = -sy;
  const r21 = 0;
  const r22 = cy;

  const m00 = r00 * scale;
  const m01 = r01 * scale;
  const m02 = r02 * scale;
  const m10 = (cx * r10 - sx * r20) * scale;
  const m11 = (cx * r11 - sx * r21) * scale;
  const m12 = (cx * r12 - sx * r22) * scale;
  const m20 = (sx * r10 + cx * r20) * scale;
  const m21 = (sx * r11 + cx * r21) * scale;
  const m22 = (sx * r12 + cx * r22) * scale;

  return new Float32Array([
    m00, m10, m20, 0,
    m01, m11, m21, 0,
    m02, m12, m22, 0,
    tx, ty, 0, 1,
  ]);
}

function buildPerspectiveMatrix(fovDeg: number, aspect: number, near: number, far: number): Float32Array {
  const f = 1 / Math.tan((fovDeg * Math.PI) / 360);
  const nf = 1 / (near - far);
  return new Float32Array([
    f / aspect, 0, 0, 0,
    0, f, 0, 0,
    0, 0, (far + near) * nf, -1,
    0, 0, 2 * far * near * nf, 0,
  ]);
}

function buildViewMatrix(camZ: number): Float32Array {
  return new Float32Array([
    1, 0, 0, 0,
    0, 1, 0, 0,
    0, 0, 1, 0,
    0, 0, -camZ, 1,
  ]);
}

function transformPointByColMajor4(m: Float32Array, x: number, y: number, z: number): [number, number, number] {
  return [
    m[0] * x + m[4] * y + m[8] * z + m[12],
    m[1] * x + m[5] * y + m[9] * z + m[13],
    m[2] * x + m[6] * y + m[10] * z + m[14],
  ];
}

function rotateWorldDirToLocal(
  rx: number,
  rz: number,
  ry: number,
  wx: number,
  wy: number,
  wz: number
): [number, number, number] {
  const m = buildGlobeModelMatrix(0, 0, rx, rz, ry, 1.0);
  return [
    m[0] * wx + m[1] * wy + m[2] * wz,
    m[4] * wx + m[5] * wy + m[6] * wz,
    m[8] * wx + m[9] * wy + m[10] * wz,
  ];
}

function solveFlyToAngles(latDeg: number, lonDeg: number): { spinY: number; tiltX: number } {
  const lat = (latDeg * Math.PI) / 180;
  const lon = (lonDeg * Math.PI) / 180;
  const tanProduct = clamp(Math.tan(AXIAL_TILT) * Math.tan(lat), -0.995, 0.995);
  const u = -Math.acos(tanProduct);
  const spinY = u - lon;

  const cl = Math.cos(lat);
  const sl = Math.sin(lat);
  const x2 = cl * Math.cos(u);
  const z2 = -cl * Math.sin(u);
  const v1y = x2 * Math.sin(AXIAL_TILT) + sl * Math.cos(AXIAL_TILT);
  const v1z = z2;
  const tiltX = clamp(Math.atan2(v1y, v1z), -PITCH_CLAMP, PITCH_CLAMP);
  return { spinY, tiltX };
}

function shortestAngleTarget(current: number, target: number): number {
  const twoPi = Math.PI * 2;
  const delta = ((((target - current) % twoPi) + Math.PI * 3) % twoPi) - Math.PI;
  return current + delta;
}

function createUvSphere(latSegs: number, lonSegs: number, radius: number) {
  const positions: number[] = [];
  const uvs: number[] = [];
  const indices: number[] = [];
  for (let iy = 0; iy <= latSegs; iy++) {
    const v = iy / latSegs;
    const lat = (v - 0.5) * Math.PI;
    const cl = Math.cos(lat);
    const sl = Math.sin(lat);
    for (let ix = 0; ix <= lonSegs; ix++) {
      const u = ix / lonSegs;
      const lon = (u - 0.5) * Math.PI * 2;
      const x = radius * cl * Math.cos(lon);
      const y = radius * sl;
      const z = -radius * cl * Math.sin(lon);
      positions.push(x, y, z);
      uvs.push(u, v);
    }
  }
  const stride = lonSegs + 1;
  for (let iy = 0; iy < latSegs; iy++) {
    for (let ix = 0; ix < lonSegs; ix++) {
      const a = iy * stride + ix;
      const b = a + 1;
      const c = a + stride;
      const d = c + 1;
      indices.push(a, b, d, a, d, c);
    }
  }
  return {
    positions: new Float32Array(positions),
    uvs: new Float32Array(uvs),
    indices: new Uint16Array(indices),
  };
}

function createProgram(gl: WebGLRenderingContext, vsSrc: string, fsSrc: string): WebGLProgram | null {
  const compile = (type: number, src: string) => {
    const shader = gl.createShader(type);
    if (!shader) return null;
    gl.shaderSource(shader, src);
    gl.compileShader(shader);
    if (!gl.getShaderParameter(shader, gl.COMPILE_STATUS)) {
      console.error("WebGL shader compile error:", gl.getShaderInfoLog(shader));
      gl.deleteShader(shader);
      return null;
    }
    return shader;
  };
  const vs = compile(gl.VERTEX_SHADER, vsSrc);
  const fs = compile(gl.FRAGMENT_SHADER, fsSrc);
  if (!vs || !fs) return null;
  const prog = gl.createProgram();
  if (!prog) return null;
  gl.attachShader(prog, vs);
  gl.attachShader(prog, fs);
  gl.linkProgram(prog);
  gl.deleteShader(vs);
  gl.deleteShader(fs);
  if (!gl.getProgramParameter(prog, gl.LINK_STATUS)) {
    console.error("WebGL program link error:", gl.getProgramInfoLog(prog));
    gl.deleteProgram(prog);
    return null;
  }
  return prog;
}

const EARTH_VS = `
attribute vec3 aPos;
attribute vec2 aUv;
uniform mat4 uModel;
uniform mat4 uView;
uniform mat4 uProj;
uniform vec3 uCamPos;
varying vec2 vUv;
varying vec3 vNormalW;
varying vec3 vNormalL;
varying vec3 vTangentW;
varying vec3 vBitangentW;
varying vec3 vViewDir;
void main() {
  vUv = aUv;
  vNormalL = normalize(aPos);
  vec3 rawT = vec3(vNormalL.z, 0.0, -vNormalL.x);
  float tLen = length(rawT);
  vec3 tL = tLen > 0.0001 ? rawT / tLen : vec3(0.0, 0.0, -1.0);
  vec3 bL = normalize(cross(vNormalL, tL));
  vec4 wp = uModel * vec4(aPos, 1.0);
  vNormalW = normalize((uModel * vec4(vNormalL, 0.0)).xyz);
  vTangentW = normalize((uModel * vec4(tL, 0.0)).xyz);
  vBitangentW = normalize((uModel * vec4(bL, 0.0)).xyz);
  vViewDir = normalize(uCamPos - wp.xyz);
  gl_Position = uProj * uView * wp;
}
`;

// Every smoothstep(a, b, x) maintains a < b. NASA Bathy + Topographic 3D Bump Relief + Filmic Lighting + Rayleigh Rim.
const EARTH_FS = `
precision highp float;
uniform sampler2D dayTex, nightTex, specTex, uIdMap, bumpTex;
uniform vec3 sunDir;
uniform float uHoverId, uHoverPrevId, uHoverMix, uSelectId;
uniform vec3 uOceanDir;
uniform float uOceanActive, uOceanTime;
varying vec2 vUv;
varying vec3 vNormalW;
varying vec3 vNormalL;
varying vec3 vTangentW;
varying vec3 vBitangentW;
varying vec3 vViewDir;

vec3 srgbToLinear(vec3 c) {
  vec3 lo = c / 12.92;
  vec3 hi = pow((c + 0.055) / 1.055, vec3(2.4));
  return mix(lo, hi, step(vec3(0.04045), c));
}

void main() {
  vec3 nGeom = normalize(vNormalW);
  float hL = texture2D(bumpTex, vUv - vec2(1.0 / 2048.0, 0.0)).r;
  float hR = texture2D(bumpTex, vUv + vec2(1.0 / 2048.0, 0.0)).r;
  float hD = texture2D(bumpTex, vUv - vec2(0.0, 1.0 / 1024.0)).r;
  float hU = texture2D(bumpTex, vUv + vec2(0.0, 1.0 / 1024.0)).r;
  float bumpStrength = 2.6;
  vec3 nLit = normalize(
    nGeom - normalize(vTangentW) * ((hR - hL) * bumpStrength) - normalize(vBitangentW) * ((hU - hD) * bumpStrength)
  );

  vec3 s = normalize(sunDir);
  vec3 v = normalize(vViewDir);
  float day = smoothstep(-0.28, 0.22, dot(nGeom, s));
  float ndl = max(dot(nLit, s), 0.0);

  vec3 dayCol   = srgbToLinear(texture2D(dayTex, vUv).rgb);
  vec3 nightCol = srgbToLinear(texture2D(nightTex, vUv).rgb);
  float ocean   = texture2D(specTex, vUv).r;

  vec3 ambientFill = mix(vec3(0.16, 0.22, 0.32), vec3(0.28, 0.30, 0.32), day);
  vec3 sunLight = vec3(1.04, 0.99, 0.93) * pow(ndl, 0.86) * (0.25 + 0.85 * day);
  vec3 col = dayCol * (ambientFill + sunLight);

  // Warm city lights on shadowed hemisphere
  col += nightCol * (1.0 - day) * 0.62 * vec3(1.0, 0.88, 0.66);

  // Restrained natural ocean satin sheen (preserves bathymetry ridges, zero milky hotspot)
  vec3 hlf = normalize(s + v);
  float spec = pow(max(dot(nGeom, hlf), 0.0), 90.0) * ocean * day;
  col += spec * vec3(0.68, 0.84, 1.0) * 0.09;

  // Subtle true-ocean cobalt clarity (zero purple cast)
  col = mix(col, col * vec3(0.88, 0.98, 1.08), ocean * 0.22);

  // Gentle limb shading
  float facing = max(dot(nGeom, v), 0.0);
  col *= mix(0.84, 1.0, smoothstep(0.0, 0.48, facing));

  // Pixel-accurate country hover & selected highlights
  float sid = texture2D(uIdMap, vUv).r * 255.0;
  float hovCur  = step(0.5, uHoverId)     * (1.0 - step(0.5, abs(sid - uHoverId)));
  float hovPrev = step(0.5, uHoverPrevId) * (1.0 - step(0.5, abs(sid - uHoverPrevId)));
  float hov = max(hovCur * uHoverMix, hovPrev * (1.0 - uHoverMix));
  float sel = step(0.5, uSelectId) * (1.0 - step(0.5, abs(sid - uSelectId)));
  col = mix(col, vec3(0.86, 0.09, 0.10), max(hov * 0.50, sel * 0.62));
  col += sel * pow(1.0 - facing, 3.0) * vec3(0.9, 0.2, 0.2) * 0.4;

  // Crisp hairline sky-blue Fresnel rim (hugs outermost grazing edge only; zero misty haze)
  float fresnel = pow(1.0 - facing, 6.5);
  col += vec3(0.32, 0.64, 0.98) * fresnel * (0.03 + 0.09 * day) * (1.0 - sel * 0.7);

  // Concentric geodesic ocean ripples on open-water hover
  if (uOceanActive > 0.001) {
    float water = 1.0 - step(0.5, sid);
    float d = acos(clamp(dot(normalize(vNormalL), uOceanDir), -1.0, 1.0));
    float env   = 1.0 - smoothstep(0.0, 0.16, d);
    float wave  = sin(d * 78.0 - uOceanTime * 3.0);
    float crest = pow(max(wave, 0.0), 3.2) * smoothstep(0.014, 0.05, d);
    float touch = 1.0 - smoothstep(0.0, 0.02, d);
    col += (crest * env * 0.72 + touch * 0.32) * water * uOceanActive * vec3(0.52, 0.80, 1.0);
  }

  // Gentle filmic highlight rolloff + linear->sRGB encode
  col = col / (1.0 + max(col, 0.0) * 0.16);
  col = pow(max(col, 0.0), vec3(0.4545));
  gl_FragColor = vec4(col, 1.0);
}
`;

const CLOUDS_VS = `
attribute vec3 aPos;
attribute vec2 aUv;
uniform mat4 uModel;
uniform mat4 uView;
uniform mat4 uProj;
varying vec2 vUv;
varying vec3 vNormalW;
void main() {
  vUv = aUv;
  vec3 nl = normalize(aPos);
  vec4 wp = uModel * vec4(aPos, 1.0);
  vNormalW = normalize((uModel * vec4(nl, 0.0)).xyz);
  gl_Position = uProj * uView * wp;
}
`;

const CLOUDS_FS = `
precision mediump float;
uniform sampler2D cloudsTex;
uniform vec3 sunDir;
varying vec2 vUv;
varying vec3 vNormalW;
void main() {
  vec4 cs = texture2D(cloudsTex, vUv);
  float c = smoothstep(0.66, 0.99, cs.r * cs.a);
  float day = smoothstep(-0.05, 0.45, dot(normalize(vNormalW), normalize(sunDir)));
  gl_FragColor = vec4(vec3(0.85) * (0.5 + day * 0.5), c * day * 0.045);
}
`;

const ATMO_VS = `
attribute vec3 aPos;
uniform mat4 uModel;
uniform mat4 uView;
uniform mat4 uProj;
uniform vec3 uCamPos;
varying vec3 vNormalW;
varying vec3 vViewDir;
void main() {
  vec4 wp = uModel * vec4(aPos, 1.0);
  vNormalW = normalize((uModel * vec4(normalize(aPos), 0.0)).xyz);
  vViewDir = normalize(uCamPos - wp.xyz);
  gl_Position = uProj * uView * wp;
}
`;

const ATMO_FS = `
precision mediump float;
uniform vec3 sunDir;
varying vec3 vNormalW;
varying vec3 vViewDir;
void main() {
  vec3 n = normalize(vNormalW);
  vec3 v = normalize(vViewDir);
  float edge = smoothstep(0.0, 0.20, clamp(dot(-n, v), 0.0, 1.0));
  float halo = pow(edge, 3.2);
  float sunLit = smoothstep(-0.38, 0.35, dot(n, normalize(sunDir)));
  vec3 glowCol = mix(vec3(0.18, 0.42, 0.78), vec3(0.42, 0.74, 1.0), sunLit);
  gl_FragColor = vec4(glowCol, halo * (0.04 + 0.10 * sunLit));
}
`;

const STARS_VS = `
attribute vec3 aPos;
attribute float aSize;
attribute float aPhase;
uniform mat4 uViewProj;
uniform float uTime;
uniform float uDpr;
varying float vTw;
varying float vTint;
void main() {
  gl_Position = uViewProj * vec4(aPos, 1.0);
  vTw = 0.72 + 0.28 * sin(uTime * 1.5 + aPhase);
  vTint = fract(aPhase * 0.6180339);
  gl_PointSize = aSize * uDpr * (0.92 + 0.25 * vTw);
}
`;

const STARS_FS = `
precision mediump float;
varying float vTw;
varying float vTint;
void main() {
  float d = length(gl_PointCoord - 0.5);
  float a = pow(1.0 - smoothstep(0.0, 0.48, d), 2.8);
  vec3 warmCol = vec3(1.0, 0.94, 0.86);
  vec3 coolCol = vec3(0.84, 0.92, 1.0);
  vec3 starCol = mix(coolCol, warmCol, smoothstep(0.65, 0.95, vTint));
  gl_FragColor = vec4(starCol, min(1.0, a * vTw * 1.25));
}
`;

function loadImg(url: string): Promise<HTMLImageElement> {
  return new Promise((resolve, reject) => {
    const img = new Image();
    img.crossOrigin = "anonymous";
    img.onload = () => resolve(img);
    img.onerror = (err) => reject(err);
    img.src = url;
  });
}

function rasterizeCountryIdCanvas(countries: readonly CompactCountryPolygon[]): {
  canvas: HTMLCanvasElement;
  pixels: Uint8ClampedArray;
} {
  const canvas = document.createElement("canvas");
  canvas.width = ID_MAP_W;
  canvas.height = ID_MAP_H;
  const ctx = canvas.getContext("2d");
  if (!ctx) {
    return { canvas, pixels: new Uint8ClampedArray(ID_MAP_W * ID_MAP_H * 4) };
  }
  ctx.imageSmoothingEnabled = false;
  ctx.fillStyle = "#000000";
  ctx.fillRect(0, 0, ID_MAP_W, ID_MAP_H);

  const toX = (lon: number) => ((lon + 180) / 360) * ID_MAP_W;
  const toY = (lat: number) => ((90 - lat) / 180) * ID_MAP_H;

  countries.forEach((country, idx) => {
    const id = idx + 1;
    if (id > 250) return;
    ctx.fillStyle = `rgb(${id}, 0, 0)`;
    ctx.beginPath();
    for (const poly of country.polygons) {
      for (const ring of poly) {
        if (!ring || ring.length < 3) continue;
        ctx.moveTo(toX(ring[0][0]), toY(ring[0][1]));
        for (let i = 1; i < ring.length; i++) {
          const prevLon = ring[i - 1][0];
          const curLon = ring[i][0];
          if (Math.abs(curLon - prevLon) > 180) {
            ctx.moveTo(toX(curLon), toY(ring[i][1]));
          } else {
            ctx.lineTo(toX(curLon), toY(ring[i][1]));
          }
        }
        ctx.closePath();
      }
    }
    ctx.fill("evenodd");
  });

  const pixels = ctx.getImageData(0, 0, ID_MAP_W, ID_MAP_H).data;
  return { canvas, pixels };
}

export function AtlasEarthCanvas({
  locale,
  panelOpen,
  selectedCountryIso3,
  flyToRequest,
  onReady,
  onHoverChange,
  onSelectLocation,
  onLayoutMetrics,
}: AtlasEarthCanvasProps) {
  const canvasRef = useRef<HTMLCanvasElement | null>(null);
  const localeRef = useRef(locale);
  const panelOpenRef = useRef(panelOpen);
  const selectedIso3Ref = useRef(selectedCountryIso3);
  const onReadyRef = useRef(onReady);
  const onHoverRef = useRef(onHoverChange);
  const onSelectRef = useRef(onSelectLocation);
  const onLayoutMetricsRef = useRef(onLayoutMetrics);

  localeRef.current = locale;
  panelOpenRef.current = panelOpen;
  selectedIso3Ref.current = selectedCountryIso3;
  onReadyRef.current = onReady;
  onHoverRef.current = onHoverChange;
  onSelectRef.current = onSelectLocation;
  onLayoutMetricsRef.current = onLayoutMetrics;

  const simRef = useRef<{
    spinY: number;
    tiltX: number;
    worldX: number;
    worldY: number;
    globeScale: number;
    camZ: number;
    cssW: number;
    cssH: number;
    cloudOffset: number;
    velY: number;
    velX: number;
    entranceScale: number;
    flyAnim: {
      active: boolean;
      t0: number;
      dur: number;
      fromSpin: number;
      toSpin: number;
      fromTilt: number;
      toTilt: number;
    } | null;
    countries: CompactCountryPolygon[];
    idPixels: Uint8ClampedArray | null;
    selectedCountryId: number;
    lastFlySeq: number;
  }>({
    spinY: INITIAL_SPIN_Y,
    tiltX: 0.06,
    worldX: -0.45,
    worldY: 0,
    globeScale: 1.0,
    camZ: 6.0,
    cssW: 1280,
    cssH: 800,
    cloudOffset: 0,
    velY: 0,
    velX: 0,
    entranceScale: 0.001,
    flyAnim: null,
    countries: [],
    idPixels: null,
    selectedCountryId: 0,
    lastFlySeq: -1,
  });

  // Sync selectedCountryId whenever ISO3 or country polygons change
  useEffect(() => {
    const sim = simRef.current;
    if (!selectedCountryIso3 || sim.countries.length === 0) {
      sim.selectedCountryId = 0;
      return;
    }
    const idx = sim.countries.findIndex((c) => c.iso3 === selectedCountryIso3);
    sim.selectedCountryId = idx >= 0 ? idx + 1 : 0;
  }, [selectedCountryIso3]);

  // Reset tilt gently when closing country drawer
  useEffect(() => {
    if (!panelOpen) {
      simRef.current.flyAnim = {
        active: true,
        t0: performance.now(),
        dur: 920,
        fromSpin: simRef.current.spinY,
        toSpin: simRef.current.spinY,
        fromTilt: simRef.current.tiltX,
        toTilt: 0.04,
      };
    }
  }, [panelOpen]);

  // Shortest-path fly-to trigger when selecting/searching a country or landmark
  useEffect(() => {
    if (!flyToRequest || flyToRequest.seq === simRef.current.lastFlySeq) return;
    simRef.current.lastFlySeq = flyToRequest.seq;
    const sim = simRef.current;
    const { spinY, tiltX } = solveFlyToAngles(flyToRequest.lat, flyToRequest.lon);
    sim.flyAnim = {
      active: true,
      t0: performance.now(),
      dur: 1280,
      fromSpin: sim.spinY,
      toSpin: shortestAngleTarget(sim.spinY, spinY),
      fromTilt: sim.tiltX,
      toTilt: tiltX,
    };
    sim.velY = 0;
    sim.velX = 0;
  }, [flyToRequest]);

  useEffect(() => {
    const canvas = canvasRef.current;
    if (!canvas) return;
    let gl: WebGLRenderingContext | null = null;
    try {
      gl = canvas.getContext("webgl", {
        alpha: true,
        antialias: true,
        powerPreference: "high-performance",
      });
    } catch {
      gl = null;
    }

    // Headless / jsdom fallback
    if (!gl) {
      const w = canvas.clientWidth || (typeof window !== "undefined" ? window.innerWidth : 1280) || 1280;
      const h = canvas.clientHeight || (typeof window !== "undefined" ? window.innerHeight : 800) || 800;
      onLayoutMetricsRef.current?.({
        globeCx: w < 720 ? w * 0.5 : w * 0.5 - 120,
        globeCy: h * 0.5,
        globeR: Math.min(h * 0.34, 280),
        cssW: w,
        cssH: h,
      });
      onReadyRef.current([]);
      return;
    }

    const webgl = gl;
    let disposed = false;
    const sim = simRef.current;

    const earthProg = createProgram(webgl, EARTH_VS, EARTH_FS);
    const cloudsProg = createProgram(webgl, CLOUDS_VS, CLOUDS_FS);
    const atmoProg = createProgram(webgl, ATMO_VS, ATMO_FS);
    const starsProg = createProgram(webgl, STARS_VS, STARS_FS);
    if (!earthProg || !cloudsProg || !atmoProg || !starsProg) {
      onReadyRef.current([]);
      return;
    }

    const earthGeo = createUvSphere(96, 96, 1.0);
    const cloudGeo = createUvSphere(64, 64, 1.006);
    const atmoGeo = createUvSphere(48, 64, 1.018);

    const makeMeshBuffers = (geo: { positions: Float32Array; uvs: Float32Array; indices: Uint16Array }) => {
      const posBuf = webgl.createBuffer();
      webgl.bindBuffer(webgl.ARRAY_BUFFER, posBuf);
      webgl.bufferData(webgl.ARRAY_BUFFER, geo.positions, webgl.STATIC_DRAW);

      const uvBuf = webgl.createBuffer();
      webgl.bindBuffer(webgl.ARRAY_BUFFER, uvBuf);
      webgl.bufferData(webgl.ARRAY_BUFFER, geo.uvs, webgl.STATIC_DRAW);

      const idxBuf = webgl.createBuffer();
      webgl.bindBuffer(webgl.ELEMENT_ARRAY_BUFFER, idxBuf);
      webgl.bufferData(webgl.ELEMENT_ARRAY_BUFFER, geo.indices, webgl.STATIC_DRAW);

      return { posBuf, uvBuf, idxBuf, count: geo.indices.length };
    };

    const earthMesh = makeMeshBuffers(earthGeo);
    const cloudMesh = makeMeshBuffers(cloudGeo);
    const atmoMesh = makeMeshBuffers(atmoGeo);

    // 2600 crisp pinpoint deep-space stars on r = 45..70
    const STAR_COUNT = 2600;
    const starPos = new Float32Array(STAR_COUNT * 3);
    const starSize = new Float32Array(STAR_COUNT);
    const starPhase = new Float32Array(STAR_COUNT);
    for (let i = 0; i < STAR_COUNT; i++) {
      const u = Math.random();
      const v = Math.random();
      const theta = u * Math.PI * 2;
      const phi = Math.acos(2 * v - 1);
      const r = 45 + Math.random() * 25;
      starPos[i * 3] = r * Math.sin(phi) * Math.cos(theta);
      starPos[i * 3 + 1] = r * Math.cos(phi);
      starPos[i * 3 + 2] = r * Math.sin(phi) * Math.sin(theta);
      starSize[i] = 1.1 + Math.pow(Math.random(), 3.0) * 3.1;
      starPhase[i] = Math.random() * Math.PI * 2;
    }
    const starPosBuf = webgl.createBuffer();
    webgl.bindBuffer(webgl.ARRAY_BUFFER, starPosBuf);
    webgl.bufferData(webgl.ARRAY_BUFFER, starPos, webgl.STATIC_DRAW);
    const starSizeBuf = webgl.createBuffer();
    webgl.bindBuffer(webgl.ARRAY_BUFFER, starSizeBuf);
    webgl.bufferData(webgl.ARRAY_BUFFER, starSize, webgl.STATIC_DRAW);
    const starPhaseBuf = webgl.createBuffer();
    webgl.bindBuffer(webgl.ARRAY_BUFFER, starPhaseBuf);
    webgl.bufferData(webgl.ARRAY_BUFFER, starPhase, webgl.STATIC_DRAW);

    const createSolidTexture = (r: number, g: number, b: number, a = 255) => {
      const tex = webgl.createTexture();
      webgl.bindTexture(webgl.TEXTURE_2D, tex);
      webgl.texImage2D(webgl.TEXTURE_2D, 0, webgl.RGBA, 1, 1, 0, webgl.RGBA, webgl.UNSIGNED_BYTE, new Uint8Array([r, g, b, a]));
      webgl.texParameteri(webgl.TEXTURE_2D, webgl.TEXTURE_MIN_FILTER, webgl.LINEAR);
      webgl.texParameteri(webgl.TEXTURE_2D, webgl.TEXTURE_MAG_FILTER, webgl.LINEAR);
      webgl.texParameteri(webgl.TEXTURE_2D, webgl.TEXTURE_WRAP_S, webgl.CLAMP_TO_EDGE);
      webgl.texParameteri(webgl.TEXTURE_2D, webgl.TEXTURE_WRAP_T, webgl.CLAMP_TO_EDGE);
      return tex;
    };

    const dayTex = createSolidTexture(12, 22, 62);
    const nightTex = createSolidTexture(0, 0, 0);
    const specTex = createSolidTexture(180, 180, 180);
    const cloudTex = createSolidTexture(0, 0, 0);
    const bumpTex = createSolidTexture(0, 0, 0);
    const idTex = webgl.createTexture();
    webgl.bindTexture(webgl.TEXTURE_2D, idTex);
    webgl.texImage2D(webgl.TEXTURE_2D, 0, webgl.RGBA, 1, 1, 0, webgl.RGBA, webgl.UNSIGNED_BYTE, new Uint8Array([0, 0, 0, 255]));
    webgl.texParameteri(webgl.TEXTURE_2D, webgl.TEXTURE_MIN_FILTER, webgl.NEAREST);
    webgl.texParameteri(webgl.TEXTURE_2D, webgl.TEXTURE_MAG_FILTER, webgl.NEAREST);
    webgl.texParameteri(webgl.TEXTURE_2D, webgl.TEXTURE_WRAP_S, webgl.CLAMP_TO_EDGE);
    webgl.texParameteri(webgl.TEXTURE_2D, webgl.TEXTURE_WRAP_T, webgl.CLAMP_TO_EDGE);

    const anisoExt =
      webgl.getExtension("EXT_texture_filter_anisotropic") ||
      webgl.getExtension("MOZ_EXT_texture_filter_anisotropic") ||
      webgl.getExtension("WEBKIT_EXT_texture_filter_anisotropic");

    const uploadImageTexture = (tex: WebGLTexture | null, img: HTMLImageElement) => {
      if (!tex || disposed) return;
      webgl.bindTexture(webgl.TEXTURE_2D, tex);
      webgl.pixelStorei(webgl.UNPACK_FLIP_Y_WEBGL, 1);
      webgl.texImage2D(webgl.TEXTURE_2D, 0, webgl.RGBA, webgl.RGBA, webgl.UNSIGNED_BYTE, img);
      webgl.texParameteri(webgl.TEXTURE_2D, webgl.TEXTURE_MIN_FILTER, webgl.LINEAR_MIPMAP_LINEAR);
      webgl.texParameteri(webgl.TEXTURE_2D, webgl.TEXTURE_MAG_FILTER, webgl.LINEAR);
      webgl.texParameteri(webgl.TEXTURE_2D, webgl.TEXTURE_WRAP_S, webgl.REPEAT);
      webgl.texParameteri(webgl.TEXTURE_2D, webgl.TEXTURE_WRAP_T, webgl.CLAMP_TO_EDGE);
      webgl.generateMipmap(webgl.TEXTURE_2D);
      if (anisoExt) {
        const maxA = webgl.getParameter(anisoExt.MAX_TEXTURE_MAX_ANISOTROPY_EXT) || 4;
        webgl.texParameterf(webgl.TEXTURE_2D, anisoExt.TEXTURE_MAX_ANISOTROPY_EXT, Math.min(8, maxA));
      }
    };

    let loadedCountries: CompactCountryPolygon[] = [];
    let texturesReady = false;

    Promise.all([
      loadImg(GLOBE_ASSET_URLS.day)
        .then((im) => uploadImageTexture(dayTex, im))
        .catch(() => {}),
      loadImg(GLOBE_ASSET_URLS.night)
        .then((im) => uploadImageTexture(nightTex, im))
        .catch(() => {}),
      loadImg(GLOBE_ASSET_URLS.specular)
        .then((im) => uploadImageTexture(specTex, im))
        .catch(() => {}),
      loadImg(GLOBE_ASSET_URLS.clouds)
        .then((im) => uploadImageTexture(cloudTex, im))
        .catch(() => {}),
      loadImg(GLOBE_ASSET_URLS.topology)
        .then((im) => uploadImageTexture(bumpTex, im))
        .catch(() => {}),
      fetch(GLOBE_ASSET_URLS.countries)
        .then((r) => (r.ok ? (r.json() as Promise<CompactCountryPolygon[]>) : []))
        .then((list) => {
          if (disposed || !Array.isArray(list)) return;
          loadedCountries = list;
          sim.countries = list;
          const { canvas: idCanvas, pixels } = rasterizeCountryIdCanvas(list);
          sim.idPixels = pixels;
          if (selectedIso3Ref.current) {
            const idx = list.findIndex((c) => c.iso3 === selectedIso3Ref.current);
            sim.selectedCountryId = idx >= 0 ? idx + 1 : 0;
          }
          webgl.bindTexture(webgl.TEXTURE_2D, idTex);
          webgl.pixelStorei(webgl.UNPACK_FLIP_Y_WEBGL, 1);
          webgl.texImage2D(webgl.TEXTURE_2D, 0, webgl.RGBA, webgl.RGBA, webgl.UNSIGNED_BYTE, idCanvas);
          webgl.texParameteri(webgl.TEXTURE_2D, webgl.TEXTURE_MIN_FILTER, webgl.NEAREST);
          webgl.texParameteri(webgl.TEXTURE_2D, webgl.TEXTURE_MAG_FILTER, webgl.NEAREST);
          webgl.texParameteri(webgl.TEXTURE_2D, webgl.TEXTURE_WRAP_S, webgl.CLAMP_TO_EDGE);
          webgl.texParameteri(webgl.TEXTURE_2D, webgl.TEXTURE_WRAP_T, webgl.CLAMP_TO_EDGE);
        })
        .catch(() => {}),
    ]).finally(() => {
      if (disposed) return;
      texturesReady = true;
      onReadyRef.current(loadedCountries);
    });

    const earthLocs = {
      aPos: webgl.getAttribLocation(earthProg, "aPos"),
      aUv: webgl.getAttribLocation(earthProg, "aUv"),
      uModel: webgl.getUniformLocation(earthProg, "uModel"),
      uView: webgl.getUniformLocation(earthProg, "uView"),
      uProj: webgl.getUniformLocation(earthProg, "uProj"),
      uCamPos: webgl.getUniformLocation(earthProg, "uCamPos"),
      dayTex: webgl.getUniformLocation(earthProg, "dayTex"),
      nightTex: webgl.getUniformLocation(earthProg, "nightTex"),
      specTex: webgl.getUniformLocation(earthProg, "specTex"),
      uIdMap: webgl.getUniformLocation(earthProg, "uIdMap"),
      bumpTex: webgl.getUniformLocation(earthProg, "bumpTex"),
      sunDir: webgl.getUniformLocation(earthProg, "sunDir"),
      uHoverId: webgl.getUniformLocation(earthProg, "uHoverId"),
      uHoverPrevId: webgl.getUniformLocation(earthProg, "uHoverPrevId"),
      uHoverMix: webgl.getUniformLocation(earthProg, "uHoverMix"),
      uSelectId: webgl.getUniformLocation(earthProg, "uSelectId"),
      uOceanDir: webgl.getUniformLocation(earthProg, "uOceanDir"),
      uOceanActive: webgl.getUniformLocation(earthProg, "uOceanActive"),
      uOceanTime: webgl.getUniformLocation(earthProg, "uOceanTime"),
    };
    const cloudLocs = {
      aPos: webgl.getAttribLocation(cloudsProg, "aPos"),
      aUv: webgl.getAttribLocation(cloudsProg, "aUv"),
      uModel: webgl.getUniformLocation(cloudsProg, "uModel"),
      uView: webgl.getUniformLocation(cloudsProg, "uView"),
      uProj: webgl.getUniformLocation(cloudsProg, "uProj"),
      cloudsTex: webgl.getUniformLocation(cloudsProg, "cloudsTex"),
      sunDir: webgl.getUniformLocation(cloudsProg, "sunDir"),
    };
    const atmoLocs = {
      aPos: webgl.getAttribLocation(atmoProg, "aPos"),
      uModel: webgl.getUniformLocation(atmoProg, "uModel"),
      uView: webgl.getUniformLocation(atmoProg, "uView"),
      uProj: webgl.getUniformLocation(atmoProg, "uProj"),
      uCamPos: webgl.getUniformLocation(atmoProg, "uCamPos"),
      sunDir: webgl.getUniformLocation(atmoProg, "sunDir"),
    };
    const starLocs = {
      aPos: webgl.getAttribLocation(starsProg, "aPos"),
      aSize: webgl.getAttribLocation(starsProg, "aSize"),
      aPhase: webgl.getAttribLocation(starsProg, "aPhase"),
      uViewProj: webgl.getUniformLocation(starsProg, "uViewProj"),
      uTime: webgl.getUniformLocation(starsProg, "uTime"),
      uDpr: webgl.getUniformLocation(starsProg, "uDpr"),
    };

    const sLen = Math.hypot(0.45, 0.28, 0.85);
    const sunDir = [0.45 / sLen, 0.28 / sLen, 0.85 / sLen] as const;

    let dpr = Math.min(window.devicePixelRatio || 1, 1.75);

    const updateViewportSize = () => {
      sim.cssW = Math.max(320, canvas.clientWidth || window.innerWidth || 1280);
      sim.cssH = Math.max(320, canvas.clientHeight || window.innerHeight || 800);
      dpr = Math.min(window.devicePixelRatio || 1, 1.75);
      canvas.width = Math.round(sim.cssW * dpr);
      canvas.height = Math.round(sim.cssH * dpr);
      webgl.viewport(0, 0, canvas.width, canvas.height);
      const aspect = sim.cssW / sim.cssH;
      const tanHalfFov = Math.tan((15 * Math.PI) / 180);
      const f = 1 / tanHalfFov;
      const minCamZForWordmark = sim.cssW < 720 ? 6.0 : (f * 4.85) / (2 * Math.max(0.65, aspect));
      sim.camZ = Math.max(6.0, 1 / (0.82 * tanHalfFov * Math.min(aspect, 1.4)), minCamZForWordmark);
      const idleWorldX = sim.cssW < 720 ? 0 : -0.45;
      const globeCx = (0.5 + (0.5 * idleWorldX * (f / aspect)) / sim.camZ) * sim.cssW;
      const globeCy = sim.cssH * 0.5;
      const globeR = (f / Math.sqrt(Math.max(1, sim.camZ * sim.camZ - 1))) * (sim.cssH * 0.5);
      onLayoutMetricsRef.current?.({
        globeCx,
        globeCy,
        globeR,
        cssW: sim.cssW,
        cssH: sim.cssH,
      });
    };
    updateViewportSize();

    const pickAtClientXY = (
      clientX: number,
      clientY: number
    ): {
      kind: "country" | "ocean";
      id: number;
      iso3?: string;
      name: string;
      lat: number;
      lon: number;
      localDir: [number, number, number];
    } | null => {
      const rect = canvas.getBoundingClientRect();
      if (rect.width <= 0 || rect.height <= 0) return null;
      const ndcX = ((clientX - rect.left) / rect.width) * 2 - 1;
      const ndcY = -(((clientY - rect.top) / rect.height) * 2 - 1);
      const aspect = rect.width / rect.height;
      const tanHalf = Math.tan((15 * Math.PI) / 180);
      const vx = ndcX * aspect * tanHalf;
      const vy = ndcY * tanHalf;
      const vz = -1.0;
      const dLen = Math.hypot(vx, vy, vz);
      const dx = vx / dLen;
      const dy = vy / dLen;
      const dz = vz / dLen;

      const r = Math.max(0.05, sim.entranceScale * sim.globeScale);
      const ocX = -sim.worldX;
      const ocY = -sim.worldY;
      const ocZ = sim.camZ;
      const b = ocX * dx + ocY * dy + ocZ * dz;
      const c = ocX * ocX + ocY * ocY + ocZ * ocZ - r * r;
      const disc = b * b - c;
      if (disc < 0) return null;
      const t = -b - Math.sqrt(disc);
      if (t <= 0) return null;

      const hitX = dx * t;
      const hitY = dy * t;
      const hitZ = sim.camZ + dz * t;
      const nwX = (hitX - sim.worldX) / r;
      const nwY = (hitY - sim.worldY) / r;
      const nwZ = hitZ / r;

      const [lx, ly, lz] = rotateWorldDirToLocal(sim.tiltX, AXIAL_TILT, sim.spinY, nwX, nwY, nwZ);
      const u = (((Math.atan2(lz, -lx) / (Math.PI * 2)) % 1) + 1) % 1;
      const v = 1 - Math.acos(clamp(ly, -1, 1)) / Math.PI;
      const lat = (v - 0.5) * 180;
      const lon = (u - 0.5) * 360;

      const idPixels = sim.idPixels;
      let votedId = 0;
      if (idPixels) {
        const px = clamp(Math.floor(u * ID_MAP_W), 0, ID_MAP_W - 1);
        const py = clamp(Math.floor((1 - v) * ID_MAP_H), 0, ID_MAP_H - 1);
        const readId = (x: number, y: number) =>
          idPixels[(clamp(y, 0, ID_MAP_H - 1) * ID_MAP_W + ((x + ID_MAP_W) % ID_MAP_W)) * 4];
        const center = readId(px, py);
        const counts = new Map<number, number>();
        for (let oy = -1; oy <= 1; oy++) {
          for (let ox = -1; ox <= 1; ox++) {
            const val = readId(px + ox, py + oy);
            counts.set(val, (counts.get(val) ?? 0) + 1);
          }
        }
        const centerSupport = counts.get(center) ?? 0;
        if (center > 0 && centerSupport >= 2) {
          votedId = center;
        } else {
          let bestLand = 0;
          let bestVotes = 0;
          for (const [k, cnt] of counts.entries()) {
            if (k > 0 && cnt > bestVotes) {
              bestLand = k;
              bestVotes = cnt;
            }
          }
          votedId = bestVotes >= 4 ? bestLand : 0;
        }
      }

      if (votedId > 0 && sim.countries[votedId - 1]) {
        const country = sim.countries[votedId - 1];
        return {
          kind: "country",
          id: votedId,
          iso3: country.iso3,
          name: formatLocalizedCountryName(country.iso3, country.nameEn, localeRef.current),
          lat,
          lon,
          localDir: [lx, ly, lz],
        };
      }
      return {
        kind: "ocean",
        id: 0,
        name: classifyOceanName(lat, lon, localeRef.current),
        lat,
        lon,
        localDir: [lx, ly, lz],
      };
    };

    let pointerDown = false;
    let lastX = 0;
    let lastY = 0;
    let movedDist = 0;

    let hoverId = 0;
    let hoverPrevId = 0;
    let hoverMix = 1;
    let oceanHover = false;
    let oceanActive = 0;
    let oceanTime = 0;
    let oceanDir: [number, number, number] = [0, 0, 1];
    let starTime = 0;
    let currentSpinSpeed = AUTO_SPIN_RAD_PER_SEC;
    let entranceElapsed = 0;

    const handlePointerDown = (e: PointerEvent) => {
      if (e.button !== 0 && e.pointerType === "mouse") return;
      pointerDown = true;
      lastX = e.clientX;
      lastY = e.clientY;
      movedDist = 0;
      sim.flyAnim = null;
      canvas.setPointerCapture?.(e.pointerId);
    };

    const handlePointerMove = (e: PointerEvent) => {
      if (pointerDown) {
        const dx = e.clientX - lastX;
        const dy = e.clientY - lastY;
        lastX = e.clientX;
        lastY = e.clientY;
        movedDist += Math.hypot(dx, dy);
        sim.velY = dx * DRAG_SENS;
        sim.velX = dy * DRAG_SENS;
        sim.spinY += sim.velY;
        sim.tiltX = clamp(sim.tiltX + sim.velX, -PITCH_CLAMP, PITCH_CLAMP);
        canvas.style.cursor = "grabbing";
        onHoverRef.current(null);
        return;
      }

      const pick = pickAtClientXY(e.clientX, e.clientY);

      if (!pick) {
        if (hoverId !== 0) {
          hoverPrevId = hoverId;
          hoverId = 0;
          hoverMix = 0;
        }
        oceanHover = false;
        canvas.style.cursor = "default";
        onHoverRef.current(null);
        return;
      }

      if (pick.kind === "country") {
        oceanHover = false;
        if (hoverId !== pick.id) {
          hoverPrevId = hoverId;
          hoverId = pick.id;
          hoverMix = 0;
        }
        canvas.style.cursor = "pointer";
        onHoverRef.current({
          kind: "country",
          id: pick.id,
          iso3: pick.iso3,
          name: pick.name,
          lat: pick.lat,
          lon: pick.lon,
          screenX: e.clientX,
          screenY: e.clientY,
        });
      } else {
        if (hoverId !== 0) {
          hoverPrevId = hoverId;
          hoverId = 0;
          hoverMix = 0;
        }
        oceanHover = true;
        oceanDir = pick.localDir;
        canvas.style.cursor = "grab";
        onHoverRef.current({
          kind: "ocean",
          id: 0,
          name: pick.name,
          lat: pick.lat,
          lon: pick.lon,
          screenX: e.clientX,
          screenY: e.clientY,
        });
      }
    };

    const handlePointerUp = (e: PointerEvent) => {
      if (!pointerDown) return;
      pointerDown = false;
      canvas.releasePointerCapture?.(e.pointerId);
      if (movedDist < 6) {
        const pick = pickAtClientXY(e.clientX, e.clientY);
        if (pick && pick.kind === "country") {
          sim.selectedCountryId = pick.id;
          onSelectRef.current({
            kind: pick.kind,
            iso3: pick.iso3,
            name: pick.name,
            lat: pick.lat,
            lon: pick.lon,
          });
        }
      }
    };

    const handlePointerCancel = () => {
      pointerDown = false;
      canvas.style.cursor = "default";
    };

    const handlePointerLeave = () => {
      if (!pointerDown) {
        oceanHover = false;
        if (hoverId !== 0) {
          hoverPrevId = hoverId;
          hoverId = 0;
          hoverMix = 0;
        }
        onHoverRef.current(null);
      }
    };

    const handleWheel = (e: WheelEvent) => {
      e.preventDefault();
      sim.flyAnim = null;
      sim.velY = clamp(sim.velY + e.deltaY * 0.00034, -0.16, 0.16);
    };

    canvas.addEventListener("pointerdown", handlePointerDown);
    canvas.addEventListener("pointermove", handlePointerMove);
    canvas.addEventListener("pointerup", handlePointerUp);
    canvas.addEventListener("pointercancel", handlePointerCancel);
    canvas.addEventListener("pointerleave", handlePointerLeave);
    canvas.addEventListener("wheel", handleWheel, { passive: false });
    window.addEventListener("resize", updateViewportSize);

    let rafId = 0;
    let lastNow = performance.now();
    let hiddenPaused = document.hidden;

    const handleVisibility = () => {
      hiddenPaused = document.hidden;
      if (!hiddenPaused) {
        lastNow = performance.now();
      }
    };
    document.addEventListener("visibilitychange", handleVisibility);

    const renderFrame = (now: number) => {
      rafId = window.requestAnimationFrame(renderFrame);
      if (disposed || hiddenPaused) return;

      const dt = Math.min(0.05, Math.max(0.001, (now - lastNow) / 1000));
      lastNow = now;

      // 1.8s power3.out entrance scale from 0.001 -> 1.0 once day map & country geometry are ready
      if (texturesReady && sim.entranceScale < 1) {
        entranceElapsed = Math.min(1.8, entranceElapsed + dt);
        const tNorm = entranceElapsed / 1.8;
        sim.entranceScale = 0.001 + 0.999 * (1 - Math.pow(1 - tNorm, 3));
      }

      const mobile = sim.cssW < 720;
      const targetWorldX = mobile ? 0 : panelOpenRef.current ? -1.42 : -0.45;
      const morphEase = Math.min(1, dt * 5.2);
      sim.worldX += (targetWorldX - sim.worldX) * morphEase;
      sim.worldY += (0 - sim.worldY) * morphEase;
      sim.globeScale += (1.0 - sim.globeScale) * morphEase;

      if (sim.flyAnim?.active) {
        const raw = clamp((now - sim.flyAnim.t0) / sim.flyAnim.dur, 0, 1);
        const eased = raw < 0.5 ? 4 * raw * raw * raw : 1 - Math.pow(-2 * raw + 2, 3) / 2;
        sim.spinY = sim.flyAnim.fromSpin + (sim.flyAnim.toSpin - sim.flyAnim.fromSpin) * eased;
        sim.tiltX = sim.flyAnim.fromTilt + (sim.flyAnim.toTilt - sim.flyAnim.fromTilt) * eased;
        if (raw >= 1) sim.flyAnim.active = false;
      } else if (!pointerDown) {
        if (!panelOpenRef.current) {
          const targetSpin = hoverId > 0 ? HOVER_COUNTRY_SPIN_RAD_PER_SEC : AUTO_SPIN_RAD_PER_SEC;
          currentSpinSpeed += (targetSpin - currentSpinSpeed) * Math.min(1, dt * 6.0);
          sim.spinY += currentSpinSpeed * dt;
        }
        sim.spinY += sim.velY;
        sim.velY *= MOMENTUM_DECAY;
        sim.tiltX = clamp(sim.tiltX + sim.velX, -PITCH_CLAMP, PITCH_CLAMP);
        sim.velX *= MOMENTUM_DECAY;
      }

      sim.cloudOffset += 0.006 * dt;
      starTime += dt;
      if (hoverMix < 1) hoverMix = Math.min(1, hoverMix + dt * 5.0);
      oceanActive += ((oceanHover ? 1 : 0) - oceanActive) * Math.min(1, dt * 9.0);
      if (oceanActive > 0.002) oceanTime += dt;

      const aspect = sim.cssW / sim.cssH;
      const proj = buildPerspectiveMatrix(30, aspect, 0.1, 100);
      const view = buildViewMatrix(sim.camZ);
      const effScale = Math.max(0.001, sim.entranceScale * sim.globeScale);
      const model = buildGlobeModelMatrix(
        sim.worldX,
        sim.worldY,
        sim.tiltX,
        AXIAL_TILT,
        sim.spinY,
        effScale
      );

      const viewProj = new Float32Array(16);
      for (let col = 0; col < 4; col++) {
        for (let row = 0; row < 4; row++) {
          let sum = 0;
          for (let k = 0; k < 4; k++) {
            sum += proj[k * 4 + row] * view[col * 4 + k];
          }
          viewProj[col * 4 + row] = sum;
        }
      }

      webgl.clearColor(0, 0, 0, 0);
      webgl.clear(webgl.COLOR_BUFFER_BIT | webgl.DEPTH_BUFFER_BIT);

      // 1. Twinkling fixed starfield
      webgl.disable(webgl.DEPTH_TEST);
      webgl.enable(webgl.BLEND);
      webgl.blendFunc(webgl.SRC_ALPHA, webgl.ONE_MINUS_SRC_ALPHA);
      webgl.useProgram(starsProg);
      webgl.bindBuffer(webgl.ARRAY_BUFFER, starPosBuf);
      webgl.enableVertexAttribArray(starLocs.aPos);
      webgl.vertexAttribPointer(starLocs.aPos, 3, webgl.FLOAT, false, 0, 0);
      webgl.bindBuffer(webgl.ARRAY_BUFFER, starSizeBuf);
      webgl.enableVertexAttribArray(starLocs.aSize);
      webgl.vertexAttribPointer(starLocs.aSize, 1, webgl.FLOAT, false, 0, 0);
      webgl.bindBuffer(webgl.ARRAY_BUFFER, starPhaseBuf);
      webgl.enableVertexAttribArray(starLocs.aPhase);
      webgl.vertexAttribPointer(starLocs.aPhase, 1, webgl.FLOAT, false, 0, 0);
      webgl.uniformMatrix4fv(starLocs.uViewProj, false, viewProj);
      webgl.uniform1f(starLocs.uTime, starTime);
      webgl.uniform1f(starLocs.uDpr, dpr);
      webgl.drawArrays(webgl.POINTS, 0, STAR_COUNT);
      webgl.disableVertexAttribArray(starLocs.aPos);
      webgl.disableVertexAttribArray(starLocs.aSize);
      webgl.disableVertexAttribArray(starLocs.aPhase);

      // 2. Lit 3D Earth sphere
      webgl.enable(webgl.DEPTH_TEST);
      webgl.enable(webgl.CULL_FACE);
      webgl.cullFace(webgl.BACK);
      webgl.disable(webgl.BLEND);
      webgl.useProgram(earthProg);

      webgl.bindBuffer(webgl.ARRAY_BUFFER, earthMesh.posBuf);
      webgl.enableVertexAttribArray(earthLocs.aPos);
      webgl.vertexAttribPointer(earthLocs.aPos, 3, webgl.FLOAT, false, 0, 0);
      webgl.bindBuffer(webgl.ARRAY_BUFFER, earthMesh.uvBuf);
      webgl.enableVertexAttribArray(earthLocs.aUv);
      webgl.vertexAttribPointer(earthLocs.aUv, 2, webgl.FLOAT, false, 0, 0);
      webgl.bindBuffer(webgl.ELEMENT_ARRAY_BUFFER, earthMesh.idxBuf);

      webgl.uniformMatrix4fv(earthLocs.uModel, false, model);
      webgl.uniformMatrix4fv(earthLocs.uView, false, view);
      webgl.uniformMatrix4fv(earthLocs.uProj, false, proj);
      webgl.uniform3f(earthLocs.uCamPos, 0, 0, sim.camZ);
      webgl.uniform3f(earthLocs.sunDir, sunDir[0], sunDir[1], sunDir[2]);
      webgl.uniform1f(earthLocs.uHoverId, hoverId);
      webgl.uniform1f(earthLocs.uHoverPrevId, hoverPrevId);
      webgl.uniform1f(earthLocs.uHoverMix, hoverMix);
      webgl.uniform1f(earthLocs.uSelectId, panelOpenRef.current ? sim.selectedCountryId : 0);
      webgl.uniform3f(earthLocs.uOceanDir, oceanDir[0], oceanDir[1], oceanDir[2]);
      webgl.uniform1f(earthLocs.uOceanActive, oceanActive);
      webgl.uniform1f(earthLocs.uOceanTime, oceanTime);

      webgl.activeTexture(webgl.TEXTURE0);
      webgl.bindTexture(webgl.TEXTURE_2D, dayTex);
      webgl.uniform1i(earthLocs.dayTex, 0);
      webgl.activeTexture(webgl.TEXTURE1);
      webgl.bindTexture(webgl.TEXTURE_2D, nightTex);
      webgl.uniform1i(earthLocs.nightTex, 1);
      webgl.activeTexture(webgl.TEXTURE2);
      webgl.bindTexture(webgl.TEXTURE_2D, specTex);
      webgl.uniform1i(earthLocs.specTex, 2);
      webgl.activeTexture(webgl.TEXTURE3);
      webgl.bindTexture(webgl.TEXTURE_2D, idTex);
      webgl.uniform1i(earthLocs.uIdMap, 3);
      webgl.activeTexture(webgl.TEXTURE4);
      webgl.bindTexture(webgl.TEXTURE_2D, bumpTex);
      webgl.uniform1i(earthLocs.bumpTex, 4);

      webgl.drawElements(webgl.TRIANGLES, earthMesh.count, webgl.UNSIGNED_SHORT, 0);
      webgl.disableVertexAttribArray(earthLocs.aPos);
      webgl.disableVertexAttribArray(earthLocs.aUv);

      // 3. Whisper cloud shell
      webgl.enable(webgl.BLEND);
      webgl.blendFunc(webgl.SRC_ALPHA, webgl.ONE_MINUS_SRC_ALPHA);
      webgl.depthMask(false);
      webgl.useProgram(cloudsProg);

      const cloudModel = buildGlobeModelMatrix(
        sim.worldX,
        sim.worldY,
        sim.tiltX,
        AXIAL_TILT,
        sim.spinY + sim.cloudOffset,
        effScale
      );

      webgl.bindBuffer(webgl.ARRAY_BUFFER, cloudMesh.posBuf);
      webgl.enableVertexAttribArray(cloudLocs.aPos);
      webgl.vertexAttribPointer(cloudLocs.aPos, 3, webgl.FLOAT, false, 0, 0);
      webgl.bindBuffer(webgl.ARRAY_BUFFER, cloudMesh.uvBuf);
      webgl.enableVertexAttribArray(cloudLocs.aUv);
      webgl.vertexAttribPointer(cloudLocs.aUv, 2, webgl.FLOAT, false, 0, 0);
      webgl.bindBuffer(webgl.ELEMENT_ARRAY_BUFFER, cloudMesh.idxBuf);

      webgl.uniformMatrix4fv(cloudLocs.uModel, false, cloudModel);
      webgl.uniformMatrix4fv(cloudLocs.uView, false, view);
      webgl.uniformMatrix4fv(cloudLocs.uProj, false, proj);
      webgl.uniform3f(cloudLocs.sunDir, sunDir[0], sunDir[1], sunDir[2]);
      webgl.activeTexture(webgl.TEXTURE0);
      webgl.bindTexture(webgl.TEXTURE_2D, cloudTex);
      webgl.uniform1i(cloudLocs.cloudsTex, 0);

      webgl.drawElements(webgl.TRIANGLES, cloudMesh.count, webgl.UNSIGNED_SHORT, 0);
      webgl.disableVertexAttribArray(cloudLocs.aPos);
      webgl.disableVertexAttribArray(cloudLocs.aUv);

      // 4. Thin Rayleigh sky-blue atmospheric rim halo (BackSide shell outside Earth limb)
      webgl.enable(webgl.DEPTH_TEST);
      webgl.enable(webgl.CULL_FACE);
      webgl.cullFace(webgl.FRONT);
      webgl.enable(webgl.BLEND);
      webgl.blendFunc(webgl.SRC_ALPHA, webgl.ONE);
      webgl.depthMask(false);
      webgl.useProgram(atmoProg);

      webgl.bindBuffer(webgl.ARRAY_BUFFER, atmoMesh.posBuf);
      webgl.enableVertexAttribArray(atmoLocs.aPos);
      webgl.vertexAttribPointer(atmoLocs.aPos, 3, webgl.FLOAT, false, 0, 0);
      webgl.bindBuffer(webgl.ELEMENT_ARRAY_BUFFER, atmoMesh.idxBuf);

      webgl.uniformMatrix4fv(atmoLocs.uModel, false, model);
      webgl.uniformMatrix4fv(atmoLocs.uView, false, view);
      webgl.uniformMatrix4fv(atmoLocs.uProj, false, proj);
      webgl.uniform3f(atmoLocs.uCamPos, 0, 0, sim.camZ);
      webgl.uniform3f(atmoLocs.sunDir, sunDir[0], sunDir[1], sunDir[2]);

      webgl.drawElements(webgl.TRIANGLES, atmoMesh.count, webgl.UNSIGNED_SHORT, 0);
      webgl.disableVertexAttribArray(atmoLocs.aPos);
      webgl.cullFace(webgl.BACK);
      webgl.depthMask(true);
    };

    rafId = window.requestAnimationFrame(renderFrame);

    return () => {
      disposed = true;
      window.cancelAnimationFrame(rafId);
      canvas.removeEventListener("pointerdown", handlePointerDown);
      canvas.removeEventListener("pointermove", handlePointerMove);
      canvas.removeEventListener("pointerup", handlePointerUp);
      canvas.removeEventListener("pointercancel", handlePointerCancel);
      canvas.removeEventListener("pointerleave", handlePointerLeave);
      canvas.removeEventListener("wheel", handleWheel);
      window.removeEventListener("resize", updateViewportSize);
      document.removeEventListener("visibilitychange", handleVisibility);

      [dayTex, nightTex, specTex, cloudTex, bumpTex, idTex].forEach((t) => t && webgl.deleteTexture(t));
      [
        earthMesh.posBuf,
        earthMesh.uvBuf,
        earthMesh.idxBuf,
        cloudMesh.posBuf,
        cloudMesh.uvBuf,
        cloudMesh.idxBuf,
        atmoMesh.posBuf,
        atmoMesh.uvBuf,
        atmoMesh.idxBuf,
        starPosBuf,
        starSizeBuf,
        starPhaseBuf,
      ].forEach((b) => b && webgl.deleteBuffer(b));
      [earthProg, cloudsProg, atmoProg, starsProg].forEach((p) => webgl.deleteProgram(p));
    };
  }, []);

  return (
    <canvas
      ref={canvasRef}
      data-testid="welcome-atlas-webgl-canvas"
      data-eo-earth-shader="atlas-v4-crisp-space"
      className="pointer-events-auto absolute inset-0 z-10 h-full w-full touch-none select-none"
    />
  );
}
