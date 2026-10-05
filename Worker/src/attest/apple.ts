// App Attest verification, following Apple's "Validating apps that connect to
// your server". Attestation runs once per install; assertions after that.

import { asBytes, decodeCbor, mapGet } from "../cbor";
import { b64Decode, bytesEqual, concat, sha256, utf8 } from "../bytes";
import type { Env } from "../env";
import { APPLE_APP_ATTEST_ROOT } from "../roots/apple";
import { ecdsaDerToRaw, OID, parseCertificate, parseRoots, readDer, derChildren, verifyChain } from "../x509";

const NONCE_EXTENSION = "1.2.840.113635.100.8.2";
const AAGUID_PRODUCTION = utf8("appattest\0\0\0\0\0\0\0");
const AAGUID_DEVELOPMENT = utf8("appattestdevelop");

export interface AttestedKey {
  keyId: Uint8Array;
  /** Uncompressed P-256 point. */
  publicKey: Uint8Array;
  appId: string;
}

function appIds(env: Env): string[] {
  return env.APPLE_APP_IDS.split(",").map((s) => s.trim()).filter(Boolean);
}

async function matchAppId(env: Env, rpIdHash: Uint8Array): Promise<string> {
  for (const id of appIds(env)) {
    if (bytesEqual(rpIdHash, await sha256(utf8(id)))) return id;
  }
  throw new Error("app attest: unknown App ID");
}

/**
 * `clientDataHash` is SHA-256 of the server nonce; the client passes the same
 * value to `attestKey`.
 */
export async function verifyAttestation(
  env: Env,
  keyIdB64: string,
  attestationB64: string,
  clientDataHash: Uint8Array,
): Promise<AttestedKey> {
  const keyId = b64Decode(keyIdB64);
  const obj = decodeCbor(b64Decode(attestationB64));
  if (mapGet(obj, "fmt") !== "apple-appattest") throw new Error("app attest: wrong fmt");
  const stmt = mapGet(obj, "attStmt");
  const x5c = mapGet(stmt, "x5c");
  if (!Array.isArray(x5c) || x5c.length < 2) throw new Error("app attest: missing x5c");
  const authData = asBytes(mapGet(obj, "authData"), "authData");

  // 1. Chain to Apple's App Attestation root.
  const chain = x5c.map((c) => parseCertificate(asBytes(c, "x5c entry")));
  const roots = parseRoots([env.APPLE_ROOT_OVERRIDE ?? APPLE_APP_ATTEST_ROOT]);
  await verifyChain(chain, roots);
  const leaf = chain[0];

  // 2-4. The leaf commits to SHA-256(authData || clientDataHash).
  const nonce = await sha256(concat(authData, clientDataHash));
  const ext = leaf.extensions.get(NONCE_EXTENSION);
  if (!ext) throw new Error("app attest: missing nonce extension");
  // SEQUENCE { [1] EXPLICIT OCTET STRING }
  const tagged = derChildren(readDer(ext))[0];
  const committed = derChildren(tagged)[0]?.value;
  if (!committed || !bytesEqual(committed, nonce)) throw new Error("app attest: nonce mismatch");

  // 5. Key ID is the hash of the leaf's public key.
  if (leaf.spkiAlg !== OID.ecPublicKey || leaf.spkiCurve !== OID.p256) throw new Error("app attest: leaf is not P-256");
  if (!bytesEqual(await sha256(leaf.publicKey), keyId)) throw new Error("app attest: key ID mismatch");

  // 6-9. authData: rpIdHash(32) flags(1) counter(4) aaguid(16) credIdLen(2) credId
  if (authData.length < 55) throw new Error("app attest: authData too short");
  const appId = await matchAppId(env, authData.subarray(0, 32));
  const counter = new DataView(authData.buffer, authData.byteOffset + 33, 4).getUint32(0);
  if (counter !== 0) throw new Error("app attest: counter is not zero");
  const aaguid = authData.subarray(37, 53);
  const dev = env.APPLE_ALLOW_DEVELOPMENT === "true";
  if (!bytesEqual(aaguid, AAGUID_PRODUCTION) && !(dev && bytesEqual(aaguid, AAGUID_DEVELOPMENT))) {
    throw new Error("app attest: not a production key");
  }
  const credLen = (authData[53] << 8) | authData[54];
  const credId = authData.subarray(55, 55 + credLen);
  if (!bytesEqual(credId, keyId)) throw new Error("app attest: credential ID mismatch");

  return { keyId, publicKey: leaf.publicKey, appId };
}

/** P-256 uncompressed point to SPKI, for WebCrypto import. */
function p256Spki(point: Uint8Array): Uint8Array {
  const prefix = new Uint8Array([
    0x30, 0x59, 0x30, 0x13, 0x06, 0x07, 0x2a, 0x86, 0x48, 0xce, 0x3d, 0x02, 0x01, 0x06, 0x08, 0x2a, 0x86, 0x48, 0xce,
    0x3d, 0x03, 0x01, 0x07, 0x03, 0x42, 0x00,
  ]);
  return concat(prefix, point);
}

/**
 * Assertions carry no counter check: the Worker keeps no per-key state, and
 * the single-use server nonce already makes each assertion fresh.
 */
export async function verifyAssertion(
  publicKey: Uint8Array,
  appId: string,
  assertionB64: string,
  clientDataHash: Uint8Array,
): Promise<void> {
  const obj = decodeCbor(b64Decode(assertionB64));
  const signature = asBytes(mapGet(obj, "signature"), "signature");
  const authData = asBytes(mapGet(obj, "authenticatorData"), "authenticatorData");
  if (authData.length < 37) throw new Error("app attest: authenticatorData too short");
  const nonce = await sha256(concat(authData, clientDataHash));
  const key = await crypto.subtle.importKey("spki", p256Spki(publicKey), { name: "ECDSA", namedCurve: "P-256" }, false, [
    "verify",
  ]);
  const ok = await crypto.subtle.verify({ name: "ECDSA", hash: "SHA-256" }, key, ecdsaDerToRaw(signature, 32), nonce);
  if (!ok) throw new Error("app attest: bad assertion signature");
  if (!bytesEqual(authData.subarray(0, 32), await sha256(utf8(appId)))) throw new Error("app attest: App ID mismatch");
}

