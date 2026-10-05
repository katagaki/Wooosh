// Wooosh Rendezvous, deployed with the `cf` CLI. PROTOCOL.md §9.1: no storage
// bindings, no logs.
//
//   cf dev --mode dev     local, development settings
//   cf deploy             production
//   cf deploy --mode dev  the development Worker
//
// Secrets, set with `cf workers secrets update` (locally: `.dev.vars`):
// SERVER_SECRET, PLAY_SA_EMAIL, PLAY_SA_PRIVATE_KEY.
import { bindings, defineConfig, exports } from "@cloudflare/config";

export default defineConfig(({ mode }) => {
  // Development accepts development App Attest keys and non-Play builds, and
  // relays unverified clients. Never point a release build at it.
  const dev = mode === "dev";
  const name = dev ? "wooosh-rendezvous-dev" : "wooosh-rendezvous";
  return {
    worker: {
      name,
      entrypoint: "./src/index.ts",
      compatibilityDate: "2026-08-15",
      workersDev: true,
      logpush: false,
      observability: { enabled: false },
      exports: {
        // The free plan only offers SQLite-backed Durable Objects. Neither
        // class writes to storage; the backend is required, not used.
        Room: exports.durableObject({ storage: "sqlite" }),
        Quota: exports.durableObject({ storage: "sqlite" }),
      },
      env: {
        ROOM: bindings.durableObject({ worker: name, exportName: "Room" }),
        QUOTA: bindings.durableObject({ worker: name, exportName: "Quota" }),
        SERVER_SECRET: bindings.secret(),
        PLAY_SA_EMAIL: bindings.secret(),
        PLAY_SA_PRIVATE_KEY: bindings.secret(),
        APPLE_APP_IDS: bindings.text("YYM4Z6MU8F.com.tsubuzaki.Wooosh"),
        PLAY_PACKAGE_NAME: bindings.text("com.tsubuzaki.WoooshGo"),
        // Release signing-certificate SHA-256, base64url. Fill in from Play
        // Console (App integrity → App signing key certificate) before deploying.
        PLAY_CERT_SHA256: bindings.text(""),
        ...(dev
          ? {
              APPLE_ALLOW_DEVELOPMENT: bindings.text("true"),
              PLAY_ALLOW_UNRECOGNIZED: bindings.text("true"),
              ANON_RELAY: bindings.text("true"),
            }
          : {}),
      },
    },
  };
});
