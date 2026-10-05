// Test-only DER/X.509 and CBOR builders, for minting App Attest-shaped chains
// under a throwaway root.

import { concat, utf8 } from "../src/bytes";

function len(n: number): Uint8Array {
  if (n < 0x80) return new Uint8Array([n]);
  if (n < 0x100) return new Uint8Array([0x81, n]);
  return new Uint8Array([0x82, n >> 8, n & 0xff]);
}

export const tlv = (tag: number, body: Uint8Array) => concat(new Uint8Array([tag]), len(body.length), body);
export const seq = (...xs: Uint8Array[]) => tlv(0x30, concat(...xs));
export const set = (...xs: Uint8Array[]) => tlv(0x31, concat(...xs));
export const octet = (b: Uint8Array) => tlv(0x04, b);
export const ctx = (n: number, body: Uint8Array) => tlv(0xa0 + n, body);

export function int(b: Uint8Array): Uint8Array {
  let v = b;
  while (v.length > 1 && v[0] === 0 && !(v[1] & 0x80)) v = v.subarray(1);
  if (v[0] & 0x80) v = concat(new Uint8Array([0]), v);
  return tlv(0x02, v);
}

export function oid(s: string): Uint8Array {
  const p = s.split(".").map(Number);
  const out = [40 * p[0] + p[1]];
  for (const n of p.slice(2)) {
    const bytes = [n & 0x7f];
    for (let v = n >> 7; v > 0; v >>= 7) bytes.unshift((v & 0x7f) | 0x80);
    out.push(...bytes);
  }
  return tlv(0x06, new Uint8Array(out));
}

const name = (cn: string) => seq(set(seq(oid("2.5.4.3"), tlv(0x0c, utf8(cn)))));
const utcTime = (d: Date) => tlv(0x17, utf8(d.toISOString().replace(/[-:T]/g, "").slice(2, 14) + "Z"));

export async function spkiOf(key: CryptoKey): Promise<Uint8Array> {
  return new Uint8Array((await crypto.subtle.exportKey("spki", key)) as ArrayBuffer);
}

export interface Issued {
  der: Uint8Array;
  keys: CryptoKeyPair;
}

/** ECDSA certificate; `issuer` absent means self-signed. */
export async function issue(opts: {
  cn: string;
  curve: "P-256" | "P-384";
  issuer?: { cn: string; keys: CryptoKeyPair; curve: "P-256" | "P-384" };
  extensions?: Array<[string, Uint8Array]>;
  notAfter?: Date;
  /** Certify this key pair instead of generating one. */
  keys?: CryptoKeyPair;
}): Promise<Issued> {
  const keys =
    opts.keys ?? ((await crypto.subtle.generateKey({ name: "ECDSA", namedCurve: opts.curve }, true, ["sign", "verify"])) as CryptoKeyPair);
  const signer = opts.issuer ?? { cn: opts.cn, keys, curve: opts.curve };
  const hash = signer.curve === "P-384" ? "SHA-384" : "SHA-256";
  const sigAlg = seq(oid(hash === "SHA-384" ? "1.2.840.10045.4.3.3" : "1.2.840.10045.4.3.2"));
  const exts = opts.extensions?.length
    ? [ctx(3, seq(...opts.extensions.map(([o, v]) => seq(oid(o), octet(v)))))]
    : [];
  const tbs = seq(
    ctx(0, int(new Uint8Array([2]))),
    int(crypto.getRandomValues(new Uint8Array(8))),
    sigAlg,
    name(signer.cn),
    seq(utcTime(new Date(Date.now() - 3600_000)), utcTime(opts.notAfter ?? new Date(Date.now() + 86400_000))),
    name(opts.cn),
    await spkiOf(keys.publicKey),
    ...exts,
  );
  const raw = new Uint8Array(await crypto.subtle.sign({ name: "ECDSA", hash }, signer.keys.privateKey, tbs));
  const half = raw.length / 2;
  const sig = seq(int(raw.subarray(0, half)), int(raw.subarray(half)));
  return { der: seq(tbs, sigAlg, tlv(0x03, concat(new Uint8Array([0]), sig))), keys };
}

export function toB64(b: Uint8Array): string {
  let s = "";
  for (const x of b) s += String.fromCharCode(x);
  return btoa(s);
}

export function fromHex(h: string): Uint8Array {
  return new Uint8Array(h.match(/../g)!.map((x) => parseInt(x, 16)));
}

/** Minimal CBOR encoder: maps with text keys, arrays, byte and text strings, small uints. */
export function cbor(v: unknown): Uint8Array {
  const head = (major: number, n: number) =>
    n < 24
      ? new Uint8Array([(major << 5) | n])
      : n < 0x100
        ? new Uint8Array([(major << 5) | 24, n])
        : new Uint8Array([(major << 5) | 25, n >> 8, n & 0xff]);
  if (v instanceof Uint8Array) return concat(head(2, v.length), v);
  if (typeof v === "string") {
    const b = utf8(v);
    return concat(head(3, b.length), b);
  }
  if (typeof v === "number") return head(0, v);
  if (Array.isArray(v)) return concat(head(4, v.length), ...v.map(cbor));
  const entries = Object.entries(v as Record<string, unknown>);
  return concat(head(5, entries.length), ...entries.flatMap(([k, x]) => [cbor(k), cbor(x)]));
}

/** Raw ECDSA signature to DER, the form App Attest assertions carry. */
export function rawSigToDer(raw: Uint8Array): Uint8Array {
  const half = raw.length / 2;
  return seq(int(raw.subarray(0, half)), int(raw.subarray(half)));
}
