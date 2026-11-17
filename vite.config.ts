import { defineConfig } from "vite";
// @ts-expect-error type error without @types/node package
import process from "node:process";
const host = process.env.TAURI_DEV_HOST;

// https://vite.dev/config/
export default defineConfig(() => ({

  // Vite options tailored for Tauri development and only applied in `tauri dev` or `tauri build`
  //
  // 1. prevent Vite from obscuring rust errors
  clearScreen: false,
  // 只从 index.html 扫描依赖，避免进入 target/ 等大目录
  optimizeDeps: { entries: ["index.html"] },
  // 2. tauri expects a fixed port, fail if that port is not available
  server: {
    port: 27461,
    strictPort: true,
    host: host || false,
    hmr: host
      ? {
          protocol: "ws",
          host,
          port: 27462,
        }
      : undefined,
    watch: {
      // 3. tell Vite to ignore watching `src-tauri`
      // workspace 编译产物（target/）体积很大，不要让 Vite 监视
      ignored: ["**/src-tauri/**", "**/target/**", "**/.superpowers/**", "**/docs/**"],
    },
  },
}));
