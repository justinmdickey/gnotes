import { defineConfig } from "vite";
import { svelte } from "@sveltejs/vite-plugin-svelte";
import { VitePWA } from "vite-plugin-pwa";

export default defineConfig({
  plugins: [
    svelte(),
    VitePWA({
      registerType: "autoUpdate",
      manifest: {
        name: "Gnotes",
        short_name: "Gnotes",
        description: "Self-hosted Markdown notes",
        theme_color: "#3584e4",
        background_color: "#fafafa",
        display: "standalone",
        icons: [
          { src: "icon.svg", sizes: "any", type: "image/svg+xml", purpose: "any" },
          { src: "icon-192.png", sizes: "192x192", type: "image/png", purpose: "any" },
          { src: "icon-512.png", sizes: "512x512", type: "image/png", purpose: "any" },
          { src: "icon-maskable-512.png", sizes: "512x512", type: "image/png", purpose: "maskable" },
        ],
      },
      workbox: {
        globPatterns: ["**/*.{js,css,html,svg,png,ico,wasm}"],
        maximumFileSizeToCacheInBytes: 8 * 1024 * 1024,
        navigateFallbackDenylist: [/^\/api\//],
      },
    }),
  ],
  resolve: {
    // loro-crdt's dev entry needs native .wasm imports. Its web entry works in both dev and build, and
    // fetches the .wasm in main.ts, where the service worker can serve it offline.
    alias: [{ find: /^loro-crdt$/, replacement: "loro-crdt/web" }],
  },
  server: {
    proxy: { "/api": { target: "http://localhost:8080", ws: true } },
  },
});
