/// <reference types="vite/client" />

interface ImportMetaEnv {
  /** Closed-source cloud API base, e.g. `https://cameo.ink`. Unset → CLOUD_ENABLED=false. */
  readonly VITE_CAMEO_API_BASE?: string;
  /** Closed-source cloud API key, baked into official builds. Unset → CLOUD_ENABLED=false. */
  readonly VITE_CAMEO_API_KEY?: string;
}

interface ImportMeta {
  readonly env: ImportMetaEnv;
}

// Build-time package version inlined via vite.config.ts `define:`.
declare const __APP_VERSION__: string;
