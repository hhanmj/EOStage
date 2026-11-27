/// <reference types="vite/client" />

interface ImportMetaEnv {
  readonly VITE_EO_ATLAS_CDN_BASE_URL?: string;
  readonly VITE_EO_ATLAS_LOCAL_PACK_AVAILABLE?: string;
}

interface ImportMeta {
  readonly env: ImportMetaEnv;
}

declare module "*.css";
declare module "jsdom";
