// Wooosh Rendezvous (DESIGN.md §9, PROTOCOL.md §9). Introduces two devices and
// stores nothing: no KV, D1, R2, Durable Object storage or logs.

import type { Env } from "./env";

export { Room } from "./room";
export { Quota } from "./quota";

/** 16 random bytes, base64url without padding. */
const ROOM_PATH = /^\/v1\/room\/([A-Za-z0-9_-]{22})$/;

export default {
  async fetch(req: Request, env: Env): Promise<Response> {
    const url = new URL(req.url);
    const m = ROOM_PATH.exec(url.pathname);
    if (m) {
      if (req.headers.get("Upgrade") !== "websocket") return new Response("expected websocket", { status: 426 });
      return env.ROOM.get(env.ROOM.idFromName(m[1])).fetch(req);
    }
    if (url.pathname === "/v1/health") return new Response("ok");
    return new Response("not found", { status: 404 });
  },
} satisfies ExportedHandler<Env>;
