// Self-verifying blobs, so the Worker remembers nothing between sessions
// (PROTOCOL.md §9.2). Only this Worker ever verifies them, so HMAC suffices.

import { b64Decode, b64urlEncode, bytesEqual, hmacSha256, utf8 } from "./bytes";
import type { Env, Tier } from "./env";

async function seal(env: Env, purpose: string, payload: object): Promise<string> {
  const body = utf8(JSON.stringify(payload));
  const mac = await hmacSha256(utf8(env.SERVER_SECRET), new Uint8Array([...utf8(purpose + "\0"), ...body]));
  return `${b64urlEncode(body)}.${b64urlEncode(mac)}`;
}

async function open<T>(env: Env, purpose: string, token: string): Promise<T | null> {
  const [bodyS, macS] = token.split(".");
  if (!bodyS || !macS) return null;
  let body: Uint8Array, mac: Uint8Array;
  try {
    body = b64Decode(bodyS);
    mac = b64Decode(macS);
  } catch {
    return null;
  }
  const expect = await hmacSha256(utf8(env.SERVER_SECRET), new Uint8Array([...utf8(purpose + "\0"), ...body]));
  if (!bytesEqual(mac, expect)) return null;
  const payload = JSON.parse(new TextDecoder().decode(body)) as T & { exp: number };
  if (typeof payload.exp !== "number" || Date.now() > payload.exp) return null;
  return payload;
}

export interface AdmissionToken {
  tier: Tier;
  /** Quota key, already rotated for `day`. */
  qk: string;
  day: string;
  exp: number;
}

export const sealAdmission = (env: Env, t: AdmissionToken) => seal(env, "admission", t);
export const openAdmission = (env: Env, s: string) => open<AdmissionToken>(env, "admission", s);

/** App Attest public key, held by the client so the Worker needs no table of keys. */
export interface KeyCredential {
  /** base64url key ID (SHA-256 of the public key). */
  kid: string;
  /** base64url uncompressed P-256 point. */
  pub: string;
  appId: string;
  exp: number;
}

export const sealKeyCredential = (env: Env, c: KeyCredential) => seal(env, "appattest-key", c);
export const openKeyCredential = (env: Env, s: string) => open<KeyCredential>(env, "appattest-key", s);

export function utcDay(now = Date.now()): string {
  return new Date(now).toISOString().slice(0, 10);
}

/**
 * HMAC(daily secret, hardware identity). The daily secret is derived, never
 * stored, so a quota key cannot be linked across days and the identity itself
 * is never kept.
 */
export async function quotaKey(env: Env, identity: Uint8Array, day = utcDay()): Promise<string> {
  const daily = await hmacSha256(utf8(env.SERVER_SECRET), utf8(`quota-day\0${day}`));
  return b64urlEncode((await hmacSha256(daily, identity)).subarray(0, 16));
}
