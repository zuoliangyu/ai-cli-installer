import { defineConfig } from "vite";
import { svelte } from "@sveltejs/vite-plugin-svelte";

const host = process.env.TAURI_DEV_HOST;

export default defineConfig(async () => {
  const pkg = await import("./package.json", { with: { type: "json" } });
  return {
    plugins: [svelte()],
    define: {
      // True when Vite is invoked from `tauri dev` / `tauri build` (those
      // commands set TAURI_ENV_PLATFORM). False for the Web-mode build that
      // gets embedded into `installer-web` via rust-embed.
      __IS_TAURI__: JSON.stringify(!!process.env.TAURI_ENV_PLATFORM),
      __APP_VERSION__: JSON.stringify(pkg.default.version),
    },
    clearScreen: false,
    server: {
      port: 1420,
      strictPort: true,
      host: host || false,
      hmr: host ? { protocol: "ws", host, port: 1421 } : undefined,
      watch: {
        ignored: ["**/src-tauri/**"],
      },
    },
  };
});
