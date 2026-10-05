// Play Integrity, standard requests. Google decrypts the verdict server side;
// the Worker authenticates to Google as a service account.

import { b64Decode, b64urlEncode, hex, sha256, utf8, concat } from "../bytes";
import type { Env } from "../env";

let cachedAccess: { token: string; exp: number } | null = null;

function pemToDer(pem: string): Uint8Array {
  return b64Decode(pem.replace(/-----[^-]+-----/g, "").replace(/\s+/g, ""));
}

async function googleAccessToken(env: Env): Promise<string> {
  if (cachedAccess && Date.now() < cachedAccess.exp - 60_000) return cachedAccess.token;
  if (!env.PLAY_SA_EMAIL || !env.PLAY_SA_PRIVATE_KEY) throw new Error("play integrity: service account not configured");
  const now = Math.floor(Date.now() / 1000);
  const enc = (o: object) => b64urlEncode(utf8(JSON.stringify(o)));
  const unsigned = `${enc({ alg: "RS256", typ: "JWT" })}.${enc({
    iss: env.PLAY_SA_EMAIL,
    scope: "https://www.googleapis.com/auth/playintegrity",
    aud: "https://oauth2.googleapis.com/token",
    iat: now,
    exp: now + 3600,
  })}`;
  const key = await crypto.subtle.importKey(
    "pkcs8",
    pemToDer(env.PLAY_SA_PRIVATE_KEY.replace(/\\n/g, "\n")),
    { name: "RSASSA-PKCS1-v1_5", hash: "SHA-256" },
    false,
    ["sign"],
  );
  const sig = new Uint8Array(await crypto.subtle.sign("RSASSA-PKCS1-v1_5", key, utf8(unsigned)));
  const res = await fetch("https://oauth2.googleapis.com/token", {
    method: "POST",
    headers: { "content-type": "application/x-www-form-urlencoded" },
    body: `grant_type=urn%3Aietf%3Aparams%3Aoauth%3Agrant-type%3Ajwt-bearer&assertion=${unsigned}.${b64urlEncode(sig)}`,
  });
  if (!res.ok) throw new Error(`play integrity: oauth ${res.status}`);
  const body = (await res.json()) as { access_token: string; expires_in: number };
  cachedAccess = { token: body.access_token, exp: Date.now() + body.expires_in * 1000 };
  return body.access_token;
}

/**
 * The value the app passes as `requestHash`: binds the verdict to this
 * server nonce and to the install's own random ID, which becomes its quota
 * identity. Play Integrity deliberately exposes no device identifier.
 */
export async function expectedRequestHash(nonce: Uint8Array, installId: Uint8Array): Promise<string> {
  return hex(await sha256(concat(nonce, installId)));
}

interface Verdict {
  requestDetails?: { requestPackageName?: string; requestHash?: string; timestampMillis?: string };
  appIntegrity?: { appRecognitionVerdict?: string; packageName?: string; certificateSha256Digest?: string[] };
  deviceIntegrity?: { deviceRecognitionVerdict?: string[] };
}

const MAX_VERDICT_AGE_MS = 5 * 60 * 1000;

export async function verifyPlayIntegrity(
  env: Env,
  integrityToken: string,
  nonce: Uint8Array,
  installId: Uint8Array,
): Promise<void> {
  const access = await googleAccessToken(env);
  const res = await fetch(
    `https://playintegrity.googleapis.com/v1/${encodeURIComponent(env.PLAY_PACKAGE_NAME)}:decodeIntegrityToken`,
    {
      method: "POST",
      headers: { authorization: `Bearer ${access}`, "content-type": "application/json" },
      body: JSON.stringify({ integrity_token: integrityToken }),
    },
  );
  if (!res.ok) throw new Error(`play integrity: decode ${res.status}`);
  const v = ((await res.json()) as { tokenPayloadExternal?: Verdict }).tokenPayloadExternal ?? {};
  checkVerdict(env, v, await expectedRequestHash(nonce, installId));
}

export function checkVerdict(env: Env, v: Verdict, requestHash: string, now = Date.now()): void {
  const rd = v.requestDetails ?? {};
  if (rd.requestPackageName !== env.PLAY_PACKAGE_NAME) throw new Error("play integrity: package mismatch");
  if (rd.requestHash !== requestHash) throw new Error("play integrity: request hash mismatch");
  const ts = Number(rd.timestampMillis);
  if (!Number.isFinite(ts) || Math.abs(now - ts) > MAX_VERDICT_AGE_MS) throw new Error("play integrity: stale verdict");

  const app = v.appIntegrity ?? {};
  const devBuild = env.PLAY_ALLOW_UNRECOGNIZED === "true";
  if (app.appRecognitionVerdict !== "PLAY_RECOGNIZED" && !devBuild) throw new Error("play integrity: app not recognized");
  if (!devBuild) {
    if (app.packageName !== env.PLAY_PACKAGE_NAME) throw new Error("play integrity: app package mismatch");
    const allowed = env.PLAY_CERT_SHA256.split(",").map((s) => s.trim());
    const digests = (app.certificateSha256Digest ?? []).map((d) => b64urlEncode(b64Decode(d)));
    if (!digests.some((d) => allowed.includes(d))) throw new Error("play integrity: signing certificate mismatch");
  }
  if (!(v.deviceIntegrity?.deviceRecognitionVerdict ?? []).includes("MEETS_DEVICE_INTEGRITY")) {
    throw new Error("play integrity: device integrity not met");
  }
}
