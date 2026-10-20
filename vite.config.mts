import { defineConfig } from "vite";
import type { ProxyOptions } from "vite";

const backend: ProxyOptions = {
  target: "http://127.0.0.1:3210",
  changeOrigin: true,
  configure(proxy) {
    proxy.on("proxyReq", (request, incoming) => {
      // The loopback development server has a different port; preserve the
      // backend's production Origin/Host checks instead of weakening them there.
      if (incoming.headers.origin === "http://127.0.0.1:5173") {
        request.setHeader("origin", "http://127.0.0.1:3210");
      }
    });
  },
};

export default defineConfig({
  root: "frontend",
  build: {
    outDir: "../static/dist",
    emptyOutDir: true,
    target: ["es2022", "safari16"],
    assetsInlineLimit: 0,
    rollupOptions: {
      output: {
        entryFileNames: "app.js",
        assetFileNames: (asset) =>
          asset.names.some((n) => n.endsWith(".css"))
            ? "style.css"
            : asset.names.some((n) => n.includes("material-icons-outlined"))
              ? "material-icons[extname]"
              : "assets/[name]-[hash][extname]",
      },
    },
  },
  server: {
    host: "127.0.0.1",
    port: 5173,
    strictPort: true,
    proxy: { "/api": backend, "/auth": backend, "/favicon.svg": backend },
  },
});
