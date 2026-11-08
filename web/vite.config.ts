import vike from "vike/plugin";
import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
import tailwindcss from "@tailwindcss/vite";
import relay from "vite-plugin-relay-lite";

// Dev server: the backend runs in the compose stack (bin/dev) on :8080.
const backend = "http://localhost:8080";

export default defineConfig({
  plugins: [vike(), react(), tailwindcss(), relay()],
  server: {
    proxy: {
      "/graphql": { target: backend, ws: true },
      "/health": backend,
    },
  },
});
