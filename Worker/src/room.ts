// One rendezvous room (PROTOCOL.md §9.4). Admission (§9.2) runs on the room's
// own WebSocket so attestation is verified with Durable Object CPU limits, not
// the free plan's 10 ms Worker limit. All state is in WebSocket attachments,
// which survive hibernation; nothing is written to storage.

import { DurableObject } from "cloudflare:workers";
import { verifyAssertion, verifyAttestation } from "./attest/apple";
import { verifyPlayIntegrity } from "./attest/android";
import { makeCredential, verifyEvidence } from "./attest/tpm";
import { b64Decode, b64urlEncode, bytesEqual, concat, randomBytes, sha256, utf8 } from "./bytes";
import { ADMISSION_TOKEN_TTL_MS, KEY_CREDENTIAL_TTL_MS, LONE_OCCUPANT_TTL_MS, QUOTA_ROOMS_PER_DAY, type Env, type Tier } from "./env";
import type { Quota } from "./quota";
import { openAdmission, openKeyCredential, quotaKey, sealAdmission, sealKeyCredential, utcDay } from "./token";

export type Role = "pub" | "redeem" | "peer" | "pair";

interface Attachment {
  role: Role;
  state: "challenged" | "tpm" | "admitted";
  nonce: string;
  joinedAt: number;
  /** Rate-limit key for unverified clients: the source /24 or /56. */
  net: string;
  tier?: Tier;
  /** SHA-256 of the expected activation secret, and the EK identity behind it. */
  tpmSecretHash?: string;
  tpmIdentity?: string;
  candSent?: boolean;
  relayRefused?: boolean;
}

/** Close codes 4000-4999 are application-defined; the reason carries the error code. */
const CLOSE_ERROR = 4000;
const CLOSE_PEER_LEFT = 4001;
const CLOSE_EXPIRED = 4002;

export const ROOM_ERRORS = [
  "bad_message",
  "bad_admit",
  "attestation_failed",
  "bad_token",
  "quota",
  "not_admitted",
  "relay_unavailable",
] as const;
export type RoomError = (typeof ROOM_ERRORS)[number];

function compatible(a: Role, b: Role): boolean {
  if (a === "pub") return b === "redeem";
  if (a === "redeem") return b === "pub";
  return a === b;
}

export function networkKey(ip: string): string {
  if (ip.includes(":")) {
    const [head, tail = ""] = ip.split("::");
    const h = head ? head.split(":") : [];
    const t = tail ? tail.split(":") : [];
    const groups = [...h, ...Array(Math.max(0, 8 - h.length - t.length)).fill("0"), ...t].map((g) => g.padStart(4, "0"));
    return `${groups.join("").slice(0, 14)}/56`;
  }
  return `${ip.split(".").slice(0, 3).join(".")}.0/24`;
}

function att(ws: WebSocket): Attachment {
  return ws.deserializeAttachment() as Attachment;
}

function send(ws: WebSocket, msg: object): void {
  try {
    ws.send(JSON.stringify(msg));
  } catch {
    // Already closing; the close handler tidies up.
  }
}

export class Room extends DurableObject<Env> {
  private open(): WebSocket[] {
    return this.ctx.getWebSockets().filter((s) => s.readyState === WebSocket.OPEN);
  }

  private other(ws: WebSocket): WebSocket | undefined {
    return this.open().find((s) => s !== ws);
  }

  private fail(ws: WebSocket, code: RoomError, detail?: string): void {
    send(ws, { t: "error", code, detail });
    ws.close(CLOSE_ERROR, code);
  }

  async fetch(req: Request): Promise<Response> {
    if (req.headers.get("Upgrade") !== "websocket") return new Response("expected websocket", { status: 426 });
    const role = new URL(req.url).searchParams.get("role") as Role | null;
    if (!role || !["pub", "redeem", "peer", "pair"].includes(role)) return new Response("bad role", { status: 400 });

    const now = Date.now();
    let present = this.open();
    for (const s of present) {
      const a = att(s);
      // A ticket or BLE room nobody joined in time is dead (§9.4); paired rooms wait indefinitely.
      if (present.length === 1 && a.role !== "pair" && now - a.joinedAt > LONE_OCCUPANT_TTL_MS) {
        s.close(CLOSE_EXPIRED, "expired");
        present = [];
      }
    }
    if (present.length >= 2) return new Response("room full", { status: 409 });
    // The publisher joins before its code is shown, so a redeemer finding no
    // publisher holds a withdrawn or expired code: say so now rather than let it wait.
    if (role === "redeem" && !present.some((s) => att(s).role === "pub")) return new Response("expired", { status: 410 });
    if (present.length === 1 && !compatible(att(present[0]).role, role)) return new Response("role conflict", { status: 409 });

    const [client, server] = Object.values(new WebSocketPair());
    this.ctx.acceptWebSocket(server);
    const nonce = b64urlEncode(randomBytes(32));
    const a: Attachment = {
      role,
      state: "challenged",
      nonce,
      joinedAt: now,
      net: networkKey(req.headers.get("CF-Connecting-IP") ?? "0.0.0.0"),
    };
    server.serializeAttachment(a);
    send(server, { t: "challenge", nonce });
    return new Response(null, { status: 101, webSocket: client });
  }

  async webSocketMessage(ws: WebSocket, msg: string | ArrayBuffer): Promise<void> {
    const a = att(ws);
    if (typeof msg !== "string") return this.relay(ws, a, msg);
    let m: any;
    try {
      m = JSON.parse(msg);
    } catch {
      return this.fail(ws, "bad_message");
    }
    if (a.state === "challenged" && m.t === "admit") return this.admit(ws, a, m);
    if (a.state === "tpm" && m.t === "tpm-activate") return this.tpmActivate(ws, a, m);
    if (a.state !== "admitted") return this.fail(ws, "not_admitted");
    switch (m.t) {
      case "cand": {
        a.candSent = true;
        ws.serializeAttachment(a);
        const o = this.other(ws);
        if (!o || att(o).state !== "admitted") return;
        // Forwarded untouched: the MAC is the receiver's to check (§9.5).
        send(o, { t: "cand", c: m.c, ufrag: m.ufrag, mac: m.mac });
        if (att(o).candSent) {
          send(ws, { t: "go" });
          send(o, { t: "go" });
        }
        return;
      }
      case "path":
        return;
      case "bye":
        for (const s of this.open()) s.close(1000, "bye");
        return;
      default:
        return this.fail(ws, "bad_message");
    }
  }

  private relay(ws: WebSocket, a: Attachment, frame: ArrayBuffer): void {
    if (a.state !== "admitted") return this.fail(ws, "not_admitted");
    const o = this.other(ws);
    if (!o || att(o).state !== "admitted") return;
    const anonRelay = this.env.ANON_RELAY === "true";
    const allowed = (t?: Tier) => t === "app" || t === "hardware" || (anonRelay && t === "unverified");
    if (!allowed(a.tier) || !allowed(att(o).tier)) {
      // Said once, not per frame; the frames themselves are dropped (§9.6).
      if (!a.relayRefused) {
        a.relayRefused = true;
        ws.serializeAttachment(a);
        send(ws, { t: "error", code: "relay_unavailable" });
      }
      return;
    }
    o.send(frame);
  }

  private async admit(ws: WebSocket, a: Attachment, m: any): Promise<void> {
    const nonce = b64Decode(a.nonce);
    const clientDataHash = await sha256(nonce);
    let tier: Tier;
    let qk: string | undefined;
    let identity: Uint8Array | undefined;
    let keyCredential: string | undefined;
    try {
      if (typeof m.token === "string") {
        const tok = await openAdmission(this.env, m.token);
        if (!tok) return this.fail(ws, "bad_token");
        tier = tok.tier;
        qk = tok.qk;
      } else if (m.apple?.attestation) {
        const key = await verifyAttestation(this.env, m.apple.key_id, m.apple.attestation, clientDataHash);
        tier = "app";
        identity = key.keyId;
        keyCredential = await sealKeyCredential(this.env, {
          kid: b64urlEncode(key.keyId),
          pub: b64urlEncode(key.publicKey),
          appId: key.appId,
          exp: Date.now() + KEY_CREDENTIAL_TTL_MS,
        });
      } else if (m.apple?.assertion) {
        const cred = await openKeyCredential(this.env, String(m.apple.key_credential ?? ""));
        if (!cred) return this.fail(ws, "bad_token", "key credential");
        await verifyAssertion(b64Decode(cred.pub), cred.appId, m.apple.assertion, clientDataHash);
        tier = "app";
        identity = b64Decode(cred.kid);
      } else if (m.android?.token) {
        const installId = b64Decode(String(m.android.install_id ?? ""));
        if (installId.length < 16 || installId.length > 64) return this.fail(ws, "bad_admit", "install id");
        await verifyPlayIntegrity(this.env, m.android.token, nonce, installId);
        tier = "app";
        identity = concat(utf8("android\0"), installId);
      } else if (m.tpm?.ek_cert) {
        const ev = await verifyEvidence(this.env, m.tpm.ek_cert, m.tpm.ek_chain ?? [], m.tpm.ak_public);
        const secret = randomBytes(32);
        const ch = await makeCredential(ev.ekCert, ev.name, secret);
        a.state = "tpm";
        a.tpmSecretHash = b64urlEncode(await sha256(secret));
        a.tpmIdentity = b64urlEncode(await sha256(ev.ekCert.spki));
        ws.serializeAttachment(a);
        send(ws, {
          t: "tpm-challenge",
          credential_blob: b64urlEncode(ch.credentialBlob),
          encrypted_secret: b64urlEncode(ch.encryptedSecret),
        });
        return;
      } else if (m.anon === true) {
        tier = "unverified";
      } else {
        return this.fail(ws, "bad_admit");
      }
    } catch (e) {
      return this.fail(ws, "attestation_failed", e instanceof Error ? e.message : String(e));
    }
    if (!qk) qk = tier === "unverified" ? `net:${a.net}` : await quotaKey(this.env, identity!);
    await this.finish(ws, a, tier, qk, typeof m.token !== "string" && tier !== "unverified", keyCredential);
  }

  private async tpmActivate(ws: WebSocket, a: Attachment, m: any): Promise<void> {
    let secret: Uint8Array;
    try {
      secret = b64Decode(String(m.secret ?? ""));
    } catch {
      return this.fail(ws, "bad_admit");
    }
    if (!a.tpmSecretHash || !bytesEqual(await sha256(secret), b64Decode(a.tpmSecretHash))) {
      return this.fail(ws, "attestation_failed", "credential activation");
    }
    const qk = await quotaKey(this.env, b64Decode(a.tpmIdentity!));
    a.tpmSecretHash = undefined;
    await this.finish(ws, a, "hardware", qk, true);
  }

  private async finish(ws: WebSocket, a: Attachment, tier: Tier, qk: string, issueToken: boolean, keyCredential?: string) {
    // Paired rooms are held open on every launch and cost nothing while hibernated, so they are not rationed.
    if (a.role !== "pair") {
      const quota = this.env.QUOTA.get(this.env.QUOTA.idFromName(qk)) as unknown as DurableObjectStub<Quota>;
      if (!(await quota.take(QUOTA_ROOMS_PER_DAY[tier]))) return this.fail(ws, "quota");
    }
    const token = issueToken
      ? await sealAdmission(this.env, { tier, qk, day: utcDay(), exp: Date.now() + ADMISSION_TOKEN_TTL_MS })
      : undefined;
    a.state = "admitted";
    a.tier = tier;
    ws.serializeAttachment(a);
    send(ws, { t: "admitted", tier, token, key_credential: keyCredential });
    const o = this.other(ws);
    if (o && att(o).state === "admitted") {
      send(ws, { t: "peer" });
      send(o, { t: "peer" });
    }
  }

  async webSocketClose(ws: WebSocket): Promise<void> {
    this.peerLeft(ws);
  }

  async webSocketError(ws: WebSocket): Promise<void> {
    this.peerLeft(ws);
  }

  private peerLeft(ws: WebSocket): void {
    const o = this.other(ws);
    if (o) o.close(CLOSE_PEER_LEFT, "peer_left");
  }
}
