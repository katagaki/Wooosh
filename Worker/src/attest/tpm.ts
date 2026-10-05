// TPM 2.0 credential activation (TCG TPM 2.0 Part 1 §24, Part 3 TPM2_MakeCredential),
// computed in software. Proves an attestation key (AK) is resident in the same
// genuine TPM as the endorsement key (EK) the manufacturer certified.
// RSA EKs only: they are what Windows provisions by default.

import { b64Decode, concat, hmacSha256, randomBytes, sha256, u16be, u32be, utf8 } from "../bytes";
import type { Env } from "../env";
import { TPM_ROOTS } from "../roots/tpm";
import { OID, parseCertificate, parseRoots, verifyChain, type Certificate } from "../x509";

const TPM_ALG_SHA256 = 0x000b;

const ATTR = {
  fixedTPM: 1 << 1,
  fixedParent: 1 << 4,
  sensitiveDataOrigin: 1 << 5,
  restricted: 1 << 16,
  decrypt: 1 << 17,
  sign: 1 << 18,
} as const;

/** Accepts TPMT_PUBLIC or TPM2B_PUBLIC (size-prefixed). */
export function unwrapPublic(blob: Uint8Array): Uint8Array {
  if (blob.length > 2 && ((blob[0] << 8) | blob[1]) === blob.length - 2) return blob.subarray(2);
  return blob;
}

/** Name = nameAlg || H(TPMT_PUBLIC), after checking it is a restricted signing key bound to its TPM. */
export async function akName(akPublic: Uint8Array): Promise<Uint8Array> {
  if (akPublic.length < 10) throw new Error("tpm: AK public too short");
  const dv = new DataView(akPublic.buffer, akPublic.byteOffset, akPublic.byteLength);
  const nameAlg = dv.getUint16(2);
  const attrs = dv.getUint32(4);
  if (nameAlg !== TPM_ALG_SHA256) throw new Error("tpm: AK name algorithm is not SHA-256");
  const required = ATTR.fixedTPM | ATTR.fixedParent | ATTR.sensitiveDataOrigin | ATTR.restricted | ATTR.sign;
  if ((attrs & required) !== required || attrs & ATTR.decrypt) {
    throw new Error("tpm: AK is not a restricted, TPM-bound signing key");
  }
  return concat(u16be(nameAlg), await sha256(akPublic));
}

/** KDFa (Part 1 §11.4.10.2), SHA-256, label NUL-terminated. */
export async function kdfa(key: Uint8Array, label: string, contextU: Uint8Array, contextV: Uint8Array, bits: number): Promise<Uint8Array> {
  const out: Uint8Array[] = [];
  let have = 0;
  for (let i = 1; have * 8 < bits; i++) {
    const block = await hmacSha256(key, concat(u32be(i), utf8(label), new Uint8Array([0]), contextU, contextV, u32be(bits)));
    out.push(block);
    have += block.length;
  }
  return concat(...out).subarray(0, bits / 8);
}

async function aesBlock(key: CryptoKey, block: Uint8Array): Promise<Uint8Array> {
  // One AES-CBC block with a zero IV is exactly the raw block cipher.
  const ct = new Uint8Array(await crypto.subtle.encrypt({ name: "AES-CBC", iv: new Uint8Array(16) }, key, block));
  return ct.subarray(0, 16);
}

/** AES-CFB-128 with a zero IV, the mode TPMs use for credential protection. */
export async function aesCfbEncrypt(keyBytes: Uint8Array, plain: Uint8Array): Promise<Uint8Array> {
  const key = await crypto.subtle.importKey("raw", keyBytes, "AES-CBC", false, ["encrypt"]);
  const out = new Uint8Array(plain.length);
  let feedback = new Uint8Array(16);
  for (let off = 0; off < plain.length; off += 16) {
    const ks = await aesBlock(key, feedback);
    const n = Math.min(16, plain.length - off);
    for (let i = 0; i < n; i++) out[off + i] = plain[off + i] ^ ks[i];
    feedback = out.slice(off, off + 16);
  }
  return out;
}

function derLen(n: number): Uint8Array {
  if (n < 0x80) return new Uint8Array([n]);
  if (n < 0x100) return new Uint8Array([0x81, n]);
  return new Uint8Array([0x82, n >> 8, n & 0xff]);
}

/** Rebuilds the SPKI with rsaEncryption, since some EK certificates label the key id-RSAES-OAEP. */
function rsaSpki(publicKey: Uint8Array): Uint8Array {
  const algId = new Uint8Array([0x30, 0x0d, 0x06, 0x09, 0x2a, 0x86, 0x48, 0x86, 0xf7, 0x0d, 0x01, 0x01, 0x01, 0x05, 0x00]);
  const bitString = concat(new Uint8Array([0x03]), derLen(publicKey.length + 1), new Uint8Array([0]), publicKey);
  const body = concat(algId, bitString);
  return concat(new Uint8Array([0x30]), derLen(body.length), body);
}

export interface Challenge {
  /** TPM2B_ID_OBJECT */
  credentialBlob: Uint8Array;
  /** TPM2B_ENCRYPTED_SECRET */
  encryptedSecret: Uint8Array;
}

/** TPM2_MakeCredential against an RSA-2048 EK with the default EK template (AES-128-CFB, SHA-256). */
export async function makeCredential(
  ek: Certificate,
  name: Uint8Array,
  secret: Uint8Array,
  seed: Uint8Array = randomBytes(32),
): Promise<Challenge> {
  if (ek.spkiAlg !== OID.rsaEncryption && ek.spkiAlg !== "1.2.840.113549.1.1.7") {
    throw new Error("tpm: only RSA endorsement keys are supported");
  }
  const oaepKey = await crypto.subtle.importKey("spki", rsaSpki(ek.publicKey), { name: "RSA-OAEP", hash: "SHA-256" }, false, [
    "encrypt",
  ]);
  const encSeed = new Uint8Array(
    await crypto.subtle.encrypt({ name: "RSA-OAEP", label: utf8("IDENTITY\0") }, oaepKey, seed),
  );
  const symKey = await kdfa(seed, "STORAGE", name, new Uint8Array(0), 128);
  const encIdentity = await aesCfbEncrypt(symKey, concat(u16be(secret.length), secret));
  const hmacKey = await kdfa(seed, "INTEGRITY", new Uint8Array(0), new Uint8Array(0), 256);
  const outerHmac = await hmacSha256(hmacKey, concat(encIdentity, name));
  const idObject = concat(u16be(outerHmac.length), outerHmac, encIdentity);
  return {
    credentialBlob: concat(u16be(idObject.length), idObject),
    encryptedSecret: concat(u16be(encSeed.length), encSeed),
  };
}

export interface TpmEvidence {
  ekCert: Certificate;
  name: Uint8Array;
}

/** Round 1: check the EK certificate chain and the AK's attributes. */
export async function verifyEvidence(env: Env, ekCertB64: string, chainB64: string[], akPublicB64: string): Promise<TpmEvidence> {
  const ekCert = parseCertificate(b64Decode(ekCertB64));
  const chain = [ekCert, ...chainB64.slice(0, 5).map((c) => parseCertificate(b64Decode(c)))];
  const roots = parseRoots(env.TPM_ROOTS_OVERRIDE ? env.TPM_ROOTS_OVERRIDE.split(",") : TPM_ROOTS);
  await verifyChain(chain, roots);
  const name = await akName(unwrapPublic(b64Decode(akPublicB64)));
  return { ekCert, name };
}

