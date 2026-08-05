import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

// 开发时把 /api 代理到 Rust 后端；构建产物输出到 dist（由后端静态托管）。
export default defineConfig({
  // 相对资源路径让同一份构建产物可部署在 / 或任意可配置子路径下。
  base: "./",
  plugins: [react()],
  server: {
    port: 5173,
    proxy: {
      "/api": {
        target: "http://localhost:8080",
        changeOrigin: true,
      },
    },
  },
  build: {
    outDir: "dist",
    emptyOutDir: true,
  },
});
