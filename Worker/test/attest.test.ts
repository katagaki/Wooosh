import { env } from "cloudflare:workers";
import { describe, expect, it } from "vitest";
import { verifyAssertion, verifyAttestation } from "../src/attest/apple";
import { checkVerdict, expectedRequestHash } from "../src/attest/android";
import { akName, makeCredential } from "../src/attest/tpm";
import { b64Decode, b64urlEncode, concat, sha256, u16be, u32be, utf8 } from "../src/bytes";
import type { Env } from "../src/env";
import { APPLE_APP_ATTEST_ROOT } from "../src/roots/apple";
import { TPM_ROOTS } from "../src/roots/tpm";
import { parseCertificate, parseRoots, readDer, derChildren, OID } from "../src/x509";
import { cbor, ctx, fromHex, issue, octet, rawSigToDer, seq, toB64 } from "./helpers";

const baseEnv = env as unknown as Env;

describe("bundled roots", () => {
  it("parses the Apple App Attestation root", () => {
    const c = parseCertificate(b64Decode(APPLE_APP_ATTEST_ROOT));
    expect(c.spkiCurve).toBe(OID.p384);
  });

  it("parses every bundled TPM manufacturer root", () => {
    expect(parseRoots(TPM_ROOTS).length).toBe(TPM_ROOTS.length);
  });
});

describe("TPM credential activation", () => {
  // Expected output computed with tpm2-pytss's make_credential algorithm
  // (KBKDFHMAC + AES-CFB + HMAC from pyca/cryptography) for the same seed,
  // secret and name. An independent implementation, not this one replayed.
  const REFERENCE =
    "00440020ee6db49ceb9ca26213a6cb473ce577d66a750a465656ea3191fa70dbe02106791b0f336a5f1b0a4f30e9f59505c329f96d79b616d00516376adc7ee1b1c48a0150d7";

  it("matches an independent MakeCredential implementation byte for byte", async () => {
    const keys = (await crypto.subtle.generateKey(
      { name: "RSA-OAEP", modulusLength: 2048, publicExponent: new Uint8Array([1, 0, 1]), hash: "SHA-256" },
      true,
      ["encrypt", "decrypt"],
    )) as CryptoKeyPair;
    const spki = new Uint8Array((await crypto.subtle.exportKey("spki", keys.publicKey)) as ArrayBuffer);
    const bits = derChildren(readDer(spki))[1];
    const ek = { spkiAlg: OID.rsaEncryption, publicKey: bits.value.subarray(1) } as any;

    const seed = new Uint8Array(Array.from({ length: 32 }, (_, i) => i));
    const secret = new Uint8Array(32).fill(0xab);
    const name = concat(fromHex("000b"), new Uint8Array(Array.from({ length: 32 }, (_, i) => 100 + i)));
    const ch = await makeCredential(ek, name, secret, seed);
    expect(b64urlEncode(ch.credentialBlob)).toBe(b64urlEncode(fromHex(REFERENCE)));

    // The EK private key recovers the seed: OAEP with SHA-256 and label "IDENTITY\0".
    const enc = ch.encryptedSecret.subarray(2);
    expect(((ch.encryptedSecret[0] << 8) | ch.encryptedSecret[1])).toBe(enc.length);
    const recovered = new Uint8Array(
      await crypto.subtle.decrypt({ name: "RSA-OAEP", label: utf8("IDENTITY\0") }, keys.privateKey, enc),
    );
    expect(recovered).toEqual(seed);
  });

  function tpmtPublic(attrs: number, nameAlg = 0x000b): Uint8Array {
    return concat(u16be(0x0001), u16be(nameAlg), u32be(attrs), u16be(0), new Uint8Array(16));
  }
  const good = (1 << 1) | (1 << 4) | (1 << 5) | (1 << 16) | (1 << 18);

  it("accepts a restricted, TPM-bound signing AK and names it nameAlg || H(public)", async () => {
    const pub = tpmtPublic(good);
    expect(await akName(pub)).toEqual(concat(u16be(0x000b), await sha256(pub)));
  });

  it("rejects AKs that could be exported, decrypt, or are not restricted", async () => {
    await expect(akName(tpmtPublic(good & ~(1 << 1)))).rejects.toThrow();
    await expect(akName(tpmtPublic(good | (1 << 17)))).rejects.toThrow();
    await expect(akName(tpmtPublic(good & ~(1 << 16)))).rejects.toThrow();
    await expect(akName(tpmtPublic(good, 0x0004))).rejects.toThrow();
  });
});

describe("App Attest", () => {
  const appId = "YYM4Z6MU8F.com.tsubuzaki.Wooosh";
  const NONCE_OID = "1.2.840.113635.100.8.2";

  async function fakeApple(opts: { aaguid?: string; counter?: number; appId?: string } = {}) {
    const root = await issue({ cn: "Test App Attest Root", curve: "P-384" });
    const inter = await issue({ cn: "Test App Attest CA", curve: "P-384", issuer: { cn: "Test App Attest Root", keys: root.keys, curve: "P-384" } });
    const leafKeys = (await crypto.subtle.generateKey({ name: "ECDSA", namedCurve: "P-256" }, true, ["sign", "verify"])) as CryptoKeyPair;
    const point = new Uint8Array((await crypto.subtle.exportKey("raw", leafKeys.publicKey)) as ArrayBuffer);
    const keyId = await sha256(point);
    const nonce = crypto.getRandomValues(new Uint8Array(32));
    const clientDataHash = await sha256(nonce);
    const aaguid = utf8((opts.aaguid ?? "appattest").padEnd(16, "\0"));
    const authData = concat(
      await sha256(utf8(opts.appId ?? appId)),
      new Uint8Array([0x40]),
      u32be(opts.counter ?? 0),
      aaguid,
      u16be(keyId.length),
      keyId,
    );
    const committed = await sha256(concat(authData, clientDataHash));
    const leaf = (
      await issue({
        cn: "leaf",
        curve: "P-256",
        keys: leafKeys,
        issuer: { cn: "Test App Attest CA", keys: inter.keys, curve: "P-384" },
        extensions: [[NONCE_OID, seq(ctx(1, octet(committed)))]],
      })
    ).der;
    const attestation = cbor({ fmt: "apple-appattest", attStmt: { x5c: [leaf, inter.der], receipt: new Uint8Array(0) }, authData });
    const testEnv = { ...baseEnv, APPLE_APP_IDS: appId, APPLE_ROOT_OVERRIDE: toB64(root.der) } as Env;
    return { testEnv, keyId, attestation, clientDataHash, leafKeys, point };
  }

  it("verifies a well-formed attestation and returns the attested key", async () => {
    const f = await fakeApple();
    const key = await verifyAttestation(f.testEnv, b64urlEncode(f.keyId), b64urlEncode(f.attestation), f.clientDataHash);
    expect(key.appId).toBe(appId);
    expect(key.publicKey).toEqual(f.point);
  });

  it("rejects an attestation over a different nonce", async () => {
    const f = await fakeApple();
    const other = await sha256(utf8("another nonce"));
    await expect(verifyAttestation(f.testEnv, b64urlEncode(f.keyId), b64urlEncode(f.attestation), other)).rejects.toThrow(/nonce/);
  });

  it("rejects development keys unless the environment allows them", async () => {
    const f = await fakeApple({ aaguid: "appattestdevelop" });
    await expect(verifyAttestation(f.testEnv, b64urlEncode(f.keyId), b64urlEncode(f.attestation), f.clientDataHash)).rejects.toThrow(/production/);
    const dev = { ...f.testEnv, APPLE_ALLOW_DEVELOPMENT: "true" } as Env;
    await expect(verifyAttestation(dev, b64urlEncode(f.keyId), b64urlEncode(f.attestation), f.clientDataHash)).resolves.toBeTruthy();
  });

  it("rejects another app's attestation and a chain that does not reach the root", async () => {
    const f = await fakeApple({ appId: "AAAAAAAAAA.com.example.other" });
    await expect(verifyAttestation(f.testEnv, b64urlEncode(f.keyId), b64urlEncode(f.attestation), f.clientDataHash)).rejects.toThrow(/App ID/);
    const g = await fakeApple();
    const realRoot = { ...g.testEnv, APPLE_ROOT_OVERRIDE: undefined } as Env;
    await expect(verifyAttestation(realRoot, b64urlEncode(g.keyId), b64urlEncode(g.attestation), g.clientDataHash)).rejects.toThrow(/root/);
  });

  it("verifies assertions over the server nonce only", async () => {
    const f = await fakeApple();
    const nonce = crypto.getRandomValues(new Uint8Array(32));
    const cdh = await sha256(nonce);
    const authData = concat(await sha256(utf8(appId)), new Uint8Array([0x40]), u32be(1));
    const signed = await sha256(concat(authData, cdh));
    const raw = new Uint8Array(await crypto.subtle.sign({ name: "ECDSA", hash: "SHA-256" }, f.leafKeys.privateKey, signed));
    const assertion = b64urlEncode(cbor({ signature: rawSigToDer(raw), authenticatorData: authData }));
    await expect(verifyAssertion(f.point, appId, assertion, cdh)).resolves.toBeUndefined();
    await expect(verifyAssertion(f.point, appId, assertion, await sha256(utf8("replayed")))).rejects.toThrow(/signature/);
  });
});

describe("Play Integrity verdicts", () => {
  const playEnv = { ...baseEnv, PLAY_PACKAGE_NAME: "com.tsubuzaki.WoooshGo", PLAY_CERT_SHA256: "q83vEjRWeJA" } as Env;
  const nonce = new Uint8Array(32).fill(1);
  const install = new Uint8Array(16).fill(2);

  async function verdict(over: Record<string, any> = {}) {
    return {
      requestDetails: {
        requestPackageName: "com.tsubuzaki.WoooshGo",
        requestHash: await expectedRequestHash(nonce, install),
        timestampMillis: String(Date.now()),
      },
      appIntegrity: { appRecognitionVerdict: "PLAY_RECOGNIZED", packageName: "com.tsubuzaki.WoooshGo", certificateSha256Digest: ["q83vEjRWeJA"] },
      deviceIntegrity: { deviceRecognitionVerdict: ["MEETS_DEVICE_INTEGRITY"] },
      ...over,
    };
  }

  it("accepts a Play-recognized build on an intact device", async () => {
    const v = await verdict();
    expect(() => checkVerdict(playEnv, v, v.requestDetails.requestHash)).not.toThrow();
  });

  it("rejects sideloaded builds, other signing keys, bad devices, and other nonces", async () => {
    const hash = await expectedRequestHash(nonce, install);
    const cases = [
      await verdict({ appIntegrity: { appRecognitionVerdict: "UNRECOGNIZED_VERSION", packageName: "com.tsubuzaki.WoooshGo", certificateSha256Digest: ["q83vEjRWeJA"] } }),
      await verdict({ appIntegrity: { appRecognitionVerdict: "PLAY_RECOGNIZED", packageName: "com.tsubuzaki.WoooshGo", certificateSha256Digest: ["AAAAAAAAAAA"] } }),
      await verdict({ deviceIntegrity: { deviceRecognitionVerdict: [] } }),
    ];
    for (const v of cases) expect(() => checkVerdict(playEnv, v, hash)).toThrow();
    const v = await verdict();
    const otherHash = await expectedRequestHash(new Uint8Array(32), install);
    expect(() => checkVerdict(playEnv, v, otherHash)).toThrow(/request hash/);
  });
});
