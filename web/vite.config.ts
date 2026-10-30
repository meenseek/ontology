import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
import { graphStyles } from "./build/graph-styles.ts";
export default defineConfig({ plugins: [react(), graphStyles()], build: { outDir: "dist", rollupOptions: { input: { app: "index.html" } } } });
