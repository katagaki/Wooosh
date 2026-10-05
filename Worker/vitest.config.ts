import { cloudflareTest } from "@cloudflare/vitest-pool-workers";
import { defineConfig } from "vitest/config";
import cloudflareConfig from "./cloudflare.config";

// The pool cannot read cloudflare.config.ts, so the production Worker is
// translated into Miniflare options here rather than described twice.
const resolved = await (typeof cloudflareConfig === "function"
  ? cloudflareConfig({ mode: undefined, isPreview: false })
  : cloudflareConfig);
const worker = await (typeof resolved.worker === "function"
  ? resolved.worker({ mode: undefined, isPreview: false })
  : resolved.worker!);

const bindings: Record<string, string> = { SERVER_SECRET: "test-secret-not-for-production" };
const durableObjects: Record<string, { className: string; useSQLite: boolean }> = {};
for (const [name, b] of Object.entries(worker.env ?? {})) {
  if (b.type === "text") bindings[name] = b.value;
  if (b.type === "durable-object") durableObjects[name] = { className: String(b.exportName), useSQLite: true };
}

export default defineConfig({
  plugins: [
    cloudflareTest({
      main: String(worker.entrypoint),
      miniflare: {
        compatibilityDate: worker.compatibilityDate,
        compatibilityFlags: worker.compatibilityFlags,
        bindings,
        durableObjects,
      },
    }),
  ],
});
