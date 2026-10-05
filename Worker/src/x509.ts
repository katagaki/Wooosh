// Just enough DER and X.509 to verify the two chains admission relies on:
// App Attest (ECDSA P-256/P-384) and TPM endorsement keys (RSA or ECDSA).
// CA chains are evaluated only against roots this Worker bundles.

import { b64Decode, bytesEqual } from "./bytes";

export interface Der {
  /** Full identifier octet, e.g. 0x30 for SEQUENCE, 0xa3 for [3]. */
  tag: number;
  /** Whole TLV. */
  bytes: Uint8Array;
  value: Uint8Array;
}

export function readDer(buf: Uint8Array, off = 0): Der {
  if (off + 2 > buf.length) throw new Error("der: truncated");
  const tag = buf[off];
  if ((tag & 0x1f) === 0x1f) throw new Error("der: high tag numbers unsupported");
  let p = off + 1;
  let len = buf[p++];
  if (len & 0x80) {
    const n = len & 0x7f;
    if (n === 0 || n > 4) throw new Error("der: bad length");
    len = 0;
    for (let i = 0; i < n; i++) {
      if (p >= buf.length) throw new Error("der: truncated");
      len = len * 256 + buf[p++];
    }
  }
  const end = p + len;
  if (end > buf.length) throw new Error("der: truncated");
  return { tag, bytes: buf.subarray(off, end), value: buf.subarray(p, end) };
}

export function derChildren(d: Der): Der[] {
  const out: Der[] = [];
  let p = 0;
  while (p < d.value.length) {
    const c = readDer(d.value, p);
    out.push(c);
    p += c.bytes.length;
  }
  return out;
}

export function oidString(d: Der): string {
  if (d.tag !== 0x06) throw new Error("der: not an OID");
  const v = d.value;
  const parts: number[] = [Math.floor(v[0] / 40), v[0] % 40];
  let acc = 0;
  for (let i = 1; i < v.length; i++) {
    acc = acc * 128 + (v[i] & 0x7f);
    if (!(v[i] & 0x80)) {
      parts.push(acc);
      acc = 0;
    }
  }
  return parts.join(".");
}

function parseTime(d: Der): number {
  const s = new TextDecoder().decode(d.value);
  let y: number, rest: string;
  if (d.tag === 0x17) {
    const yy = Number(s.slice(0, 2));
    y = yy >= 50 ? 1900 + yy : 2000 + yy;
    rest = s.slice(2);
  } else if (d.tag === 0x18) {
    y = Number(s.slice(0, 4));
    rest = s.slice(4);
  } else throw new Error("x509: bad time");
  // TPM EK certs commonly use 99991231235959Z for "no expiry"; Date handles it.
  return Date.UTC(y, Number(rest.slice(0, 2)) - 1, Number(rest.slice(2, 4)), Number(rest.slice(4, 6)), Number(rest.slice(6, 8)), Number(rest.slice(8, 10)) || 0);
}

export interface Certificate {
  der: Uint8Array;
  tbs: Uint8Array;
  sigAlg: string;
  signature: Uint8Array;
  issuer: Uint8Array;
  subject: Uint8Array;
  notBefore: number;
  notAfter: number;
  spki: Uint8Array;
  spkiAlg: string;
  /** Named curve OID for EC keys. */
  spkiCurve?: string;
  /** BIT STRING contents without the unused-bits octet. */
  publicKey: Uint8Array;
  extensions: Map<string, Uint8Array>;
}

export function parseCertificate(der: Uint8Array): Certificate {
  const cert = readDer(der);
  if (cert.tag !== 0x30 || cert.bytes.length !== der.length) throw new Error("x509: not a certificate");
  const [tbsD, algD, sigD] = derChildren(cert);
  if (!tbsD || !algD || !sigD || sigD.tag !== 0x03) throw new Error("x509: malformed certificate");
  const tbs = derChildren(tbsD);
  let i = tbs[0]?.tag === 0xa0 ? 1 : 0;
  const issuer = tbs[i + 2];
  const validity = derChildren(tbs[i + 3]);
  const subject = tbs[i + 4];
  const spkiD = tbs[i + 5];
  if (!issuer || !subject || !spkiD || validity.length !== 2) throw new Error("x509: malformed tbs");
  const [spkiAlgD, spkiKeyD] = derChildren(spkiD);
  const spkiAlgParts = derChildren(spkiAlgD);
  const spkiAlg = oidString(spkiAlgParts[0]);
  const spkiCurve = spkiAlgParts[1]?.tag === 0x06 ? oidString(spkiAlgParts[1]) : undefined;
  const extensions = new Map<string, Uint8Array>();
  for (const t of tbs.slice(i + 6)) {
    if (t.tag !== 0xa3) continue;
    for (const ext of derChildren(derChildren(t)[0])) {
      const parts = derChildren(ext);
      const value = parts[parts.length - 1];
      if (value.tag !== 0x04) throw new Error("x509: bad extension");
      extensions.set(oidString(parts[0]), value.value);
    }
  }
  return {
    der,
    tbs: tbsD.bytes,
    sigAlg: oidString(derChildren(algD)[0]),
    signature: sigD.value.subarray(1),
    issuer: issuer.bytes,
    subject: subject.bytes,
    notBefore: parseTime(validity[0]),
    notAfter: parseTime(validity[1]),
    spki: spkiD.bytes,
    spkiAlg,
    spkiCurve,
    publicKey: spkiKeyD.value.subarray(1),
    extensions,
  };
}

export const OID = {
  ecPublicKey: "1.2.840.10045.2.1",
  rsaEncryption: "1.2.840.113549.1.1.1",
  p256: "1.2.840.10045.3.1.7",
  p384: "1.3.132.0.34",
} as const;

const SIG_ALGS: Record<string, { kind: "ecdsa" | "rsa"; hash: string }> = {
  "1.2.840.10045.4.3.2": { kind: "ecdsa", hash: "SHA-256" },
  "1.2.840.10045.4.3.3": { kind: "ecdsa", hash: "SHA-384" },
  "1.2.840.113549.1.1.11": { kind: "rsa", hash: "SHA-256" },
  "1.2.840.113549.1.1.12": { kind: "rsa", hash: "SHA-384" },
  "1.2.840.113549.1.1.13": { kind: "rsa", hash: "SHA-512" },
};

/** DER ECDSA-Sig-Value to the IEEE P1363 r||s form WebCrypto expects. */
export function ecdsaDerToRaw(sig: Uint8Array, size: number): Uint8Array {
  const [r, s] = derChildren(readDer(sig));
  if (!r || !s) throw new Error("ecdsa: bad signature");
  const out = new Uint8Array(size * 2);
  for (const [i, part] of [r.value, s.value].entries()) {
    let v = part;
    while (v.length > size && v[0] === 0) v = v.subarray(1);
    if (v.length > size) throw new Error("ecdsa: integer too large");
    out.set(v, i * size + (size - v.length));
  }
  return out;
}

export async function importEcKey(spki: Uint8Array, curve: string | undefined): Promise<{ key: CryptoKey; size: number }> {
  const named = curve === OID.p256 ? "P-256" : curve === OID.p384 ? "P-384" : null;
  if (!named) throw new Error(`x509: unsupported curve ${curve}`);
  const key = await crypto.subtle.importKey("spki", spki, { name: "ECDSA", namedCurve: named }, false, ["verify"]);
  return { key, size: named === "P-256" ? 32 : 48 };
}

/** True if `issuer`'s key produced `cert`'s signature. */
export async function verifySignedBy(cert: Certificate, issuer: Certificate): Promise<boolean> {
  const alg = SIG_ALGS[cert.sigAlg];
  if (!alg) throw new Error(`x509: unsupported signature algorithm ${cert.sigAlg}`);
  if (alg.kind === "ecdsa") {
    if (issuer.spkiAlg !== OID.ecPublicKey) return false;
    const { key, size } = await importEcKey(issuer.spki, issuer.spkiCurve);
    return crypto.subtle.verify({ name: "ECDSA", hash: alg.hash }, key, ecdsaDerToRaw(cert.signature, size), cert.tbs);
  }
  if (issuer.spkiAlg !== OID.rsaEncryption) return false;
  const key = await crypto.subtle.importKey("spki", issuer.spki, { name: "RSASSA-PKCS1-v1_5", hash: alg.hash }, false, ["verify"]);
  return crypto.subtle.verify("RSASSA-PKCS1-v1_5", key, cert.signature, cert.tbs);
}

/**
 * Verifies `chain` (leaf first, then intermediates in any order) up to one of
 * `roots`. Every certificate, root included, must be within its validity
 * window at `now`.
 */
export async function verifyChain(chain: Certificate[], roots: Certificate[], now = Date.now()): Promise<void> {
  if (chain.length === 0 || chain.length > 6) throw new Error("x509: bad chain length");
  const pool = chain.slice(1);
  let cur = chain[0];
  for (let depth = 0; depth < 8; depth++) {
    if (now < cur.notBefore || now > cur.notAfter) throw new Error("x509: certificate outside validity");
    for (const root of roots) {
      if (bytesEqual(root.subject, cur.issuer) && (await verifySignedBy(cur, root))) {
        if (now < root.notBefore || now > root.notAfter) throw new Error("x509: root outside validity");
        return;
      }
    }
    const next = pool.find((c) => bytesEqual(c.subject, cur.issuer));
    if (!next || !(await verifySignedBy(cur, next))) throw new Error("x509: chain does not reach a trusted root");
    cur = next;
  }
  throw new Error("x509: chain too deep");
}

const rootCache = new Map<string, Certificate[]>();

export function parseRoots(b64s: string[]): Certificate[] {
  const key = b64s.join("|");
  let roots = rootCache.get(key);
  if (!roots) {
    roots = [];
    for (const s of b64s) {
      try {
        roots.push(parseCertificate(b64Decode(s)));
      } catch {
        // A root this parser cannot read simply trusts nothing.
      }
    }
    rootCache.set(key, roots);
  }
  return roots;
}
