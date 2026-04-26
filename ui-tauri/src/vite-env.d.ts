/// <reference types="vite/client" />

interface ImportMetaEnv {
  readonly VITE_DAEMON?: "mock" | "bridge" | "tauri";
}

interface ImportMeta {
  readonly env: ImportMetaEnv;
}

declare const __APP_COMMIT__: string;
