import { env, exports as workerExports } from "cloudflare:workers";
import { describe, expect, it } from "vitest";
import { b64urlEncode } from "../src/bytes";
import type { Env } from "../src/env";
import { networkKey } from "../src/room";
import { sealAdmission, utcDay } from "../src/token";

const testEnv = env as unknown as Env;
const worker = (workerExports as unknown as { default: Fetcher }).default;

class Client {
  readonly inbox: any[] = [];
  readonly binary: Uint8Array[] = [];
  private waiters: Array<() => void> = [];
  closed?: { code: number; reason: string };

  constructor(readonly ws: WebSocket) {
    ws.accept();
    ws.addEventListener("message", (e) => {
      if (typeof e.data === "string") this.inbox.push(JSON.parse(e.data));
      // The test-side socket hands binary frames over as Blobs.
      else void (e.data as Blob).arrayBuffer().then((b) => {
        this.binary.push(new Uint8Array(b));
        this.wake();
      });
      this.wake();
    });
    ws.addEventListener("close", (e) => {
      this.closed = { code: e.code, reason: e.reason };
      this.wake();
    });
  }

  private wake() {
    const w = this.waiters;
    this.waiters = [];
    w.forEach((f) => f());
  }

  async next(t?: string): Promise<any> {
    for (let i = 0; i < 200; i++) {
      const idx = this.inbox.findIndex((m) => !t || m.t === t);
      if (idx >= 0) return this.inbox.splice(idx, 1)[0];
      if (this.closed) throw new Error(`closed ${this.closed.code} ${this.closed.reason} waiting for ${t}`);
      await new Promise<void>((r) => {
        this.waiters.push(r);
        setTimeout(r, 25);
      });
    }
    throw new Error(`timed out waiting for ${t}`);
  }

  async nextBinary(): Promise<Uint8Array> {
    for (let i = 0; i < 200 && this.binary.length === 0; i++) await new Promise((r) => setTimeout(r, 10));
    const b = this.binary.shift();
    if (!b) throw new Error("no binary frame");
    return b;
  }

  send(m: object) {
    this.ws.send(JSON.stringify(m));
  }
}

function roomId(): string {
  return b64urlEncode(crypto.getRandomValues(new Uint8Array(16)));
}

async function connect(room: string, role: string, ip = "203.0.113.7"): Promise<Client | Response> {
  const res = await worker.fetch(`https://rendezvous.test/v1/room/${room}?role=${role}`, {
    headers: { Upgrade: "websocket", "CF-Connecting-IP": ip },
  });
  if (!res.webSocket) return res;
  return new Client(res.webSocket);
}

async function join(room: string, role: string, admit: object, ip?: string): Promise<Client> {
  const c = await connect(room, role, ip);
  if (!(c instanceof Client)) throw new Error(`join failed: ${c.status}`);
  expect((await c.next("challenge")).nonce).toMatch(/^[A-Za-z0-9_-]{43}$/);
  c.send({ t: "admit", ...admit });
  return c;
}

const appToken = () =>
  sealAdmission(testEnv, { tier: "app", qk: b64urlEncode(crypto.getRandomValues(new Uint8Array(16))), day: utcDay(), exp: Date.now() + 60_000 });

describe("rendezvous room", () => {
  it("rejects malformed paths and non-WebSocket requests", async () => {
    expect((await worker.fetch("https://rendezvous.test/v1/room/short")).status).toBe(404);
    expect((await worker.fetch(`https://rendezvous.test/v1/room/${roomId()}`)).status).toBe(426);
  });

  it("introduces publisher and redeemer, forwards candidates, then says go", async () => {
    const room = roomId();
    const pub = await join(room, "pub", { token: await appToken() });
    expect((await pub.next("admitted")).tier).toBe("app");
    const red = await join(room, "redeem", { token: await appToken() });
    expect((await red.next("admitted")).tier).toBe("app");
    await pub.next("peer");
    await red.next("peer");

    pub.send({ t: "cand", c: [{ kind: "host", a: "192.0.2.1:5000" }], ufrag: "AAAAAAAAAAA", mac: "mac-a" });
    red.send({ t: "cand", c: [{ kind: "srflx", a: "198.51.100.2:6000" }], ufrag: "BBBBBBBBBBB", mac: "mac-b" });
    expect(await red.next("cand")).toMatchObject({ c: [{ a: "192.0.2.1:5000" }], mac: "mac-a" });
    expect(await pub.next("cand")).toMatchObject({ c: [{ a: "198.51.100.2:6000" }], mac: "mac-b" });
    await pub.next("go");
    await red.next("go");

    // Relay frames pass through untouched between admitted, relay-eligible clients.
    const frame = new Uint8Array([5, 0, 3, 1, 2, 3]);
    pub.ws.send(frame);
    expect(await red.nextBinary()).toEqual(frame);
  });

  it("issues a reusable admission token to anonymous clients only as unverified, and never relays them in production", async () => {
    const room = roomId();
    const a = await join(room, "peer", { anon: true }, "198.51.100.10");
    const admitted = await a.next("admitted");
    expect(admitted.tier).toBe("unverified");
    expect(admitted.token).toBeUndefined();
    const b = await join(room, "peer", { token: await appToken() });
    await b.next("admitted");
    await a.next("peer");
    a.ws.send(new Uint8Array([5, 0, 1, 9]));
    expect((await a.next("error")).code).toBe("relay_unavailable");
    expect(b.binary.length).toBe(0);
  });

  it("turns a redeemer away at once when no publisher is waiting", async () => {
    const r = await connect(roomId(), "redeem");
    expect((r as Response).status).toBe(410);
  });

  it("holds at most two clients, of compatible roles", async () => {
    const room = roomId();
    await join(room, "pub", { anon: true }, "198.51.100.20");
    const second = await connect(room, "pub");
    expect(second).toBeInstanceOf(Response);
    expect((second as Response).status).toBe(409);
    await join(room, "redeem", { anon: true }, "198.51.100.21");
    const third = await connect(room, "redeem");
    expect((third as Response).status).toBe(409);
  });

  it("refuses forged tokens and unknown admission evidence", async () => {
    const c = await join(roomId(), "pub", { token: "e30.AAAA" });
    expect((await c.next("error")).code).toBe("bad_token");
    const d = await join(roomId(), "pub", { magic: true });
    expect((await d.next("error")).code).toBe("bad_admit");
  });

  it("refuses room messages before admission", async () => {
    const c = await connect(roomId(), "pub");
    if (!(c instanceof Client)) throw new Error("expected socket");
    await c.next("challenge");
    c.send({ t: "cand", c: [] });
    expect((await c.next("error")).code).toBe("not_admitted");
  });

  it("rations unverified clients per network, not per connection", async () => {
    const ip = "192.0.2.99";
    for (let i = 0; i < 5; i++) {
      const c = await join(roomId(), "pub", { anon: true }, ip);
      await c.next("admitted");
    }
    const over = await join(roomId(), "pub", { anon: true }, "192.0.2.42");
    expect((await over.next("error")).code).toBe("quota");
  });

  it("tells the remaining client when its peer leaves", async () => {
    const room = roomId();
    const a = await join(room, "pair", { token: await appToken() });
    const b = await join(room, "pair", { token: await appToken() });
    await a.next("peer");
    b.ws.close(1000, "done");
    for (let i = 0; i < 100 && !a.closed; i++) await new Promise((r) => setTimeout(r, 10));
    expect(a.closed?.reason).toBe("peer_left");
  });
});

describe("network keys", () => {
  it("groups IPv4 by /24 and IPv6 by /56", () => {
    expect(networkKey("203.0.113.7")).toBe("203.0.113.0/24");
    expect(networkKey("2001:db8:aa:bb00::1")).toBe(networkKey("2001:db8:aa:bbff:1::9"));
    expect(networkKey("2001:db8:aa:bb00::1")).not.toBe(networkKey("2001:db8:aa:cc00::1"));
  });
});
