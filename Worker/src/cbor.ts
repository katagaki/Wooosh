// Minimal CBOR decoder (RFC 8949) for App Attest objects. Definite lengths
// only, which is all Apple emits; anything else is rejected rather than guessed.

export type CborValue =
  | number
  | bigint
  | string
  | boolean
  | null
  | undefined
  | Uint8Array
  | CborValue[]
  | Map<CborValue, CborValue>;

export function decodeCbor(buf: Uint8Array): CborValue {
  const r = { p: 0 };
  const v = item(buf, r, 0);
  if (r.p !== buf.length) throw new Error("cbor: trailing bytes");
  return v;
}

function readLen(buf: Uint8Array, r: { p: number }, info: number): number {
  if (info < 24) return info;
  const n = info === 24 ? 1 : info === 25 ? 2 : info === 26 ? 4 : info === 27 ? 8 : -1;
  if (n < 0) throw new Error("cbor: indefinite or reserved length");
  if (r.p + n > buf.length) throw new Error("cbor: truncated");
  let v = 0;
  for (let i = 0; i < n; i++) v = v * 256 + buf[r.p++];
  if (!Number.isSafeInteger(v)) throw new Error("cbor: length too large");
  return v;
}

function item(buf: Uint8Array, r: { p: number }, depth: number): CborValue {
  if (depth > 16) throw new Error("cbor: nesting too deep");
  if (r.p >= buf.length) throw new Error("cbor: truncated");
  const b = buf[r.p++];
  const major = b >> 5;
  const info = b & 0x1f;
  switch (major) {
    case 0:
      return readLen(buf, r, info);
    case 1:
      return -1 - readLen(buf, r, info);
    case 2:
    case 3: {
      const n = readLen(buf, r, info);
      if (r.p + n > buf.length) throw new Error("cbor: truncated");
      const bytes = buf.subarray(r.p, r.p + n);
      r.p += n;
      return major === 2 ? bytes : new TextDecoder("utf-8", { fatal: true, ignoreBOM: false }).decode(bytes);
    }
    case 4: {
      const n = readLen(buf, r, info);
      const out: CborValue[] = [];
      for (let i = 0; i < n; i++) out.push(item(buf, r, depth + 1));
      return out;
    }
    case 5: {
      const n = readLen(buf, r, info);
      const m = new Map<CborValue, CborValue>();
      for (let i = 0; i < n; i++) {
        const k = item(buf, r, depth + 1);
        m.set(k, item(buf, r, depth + 1));
      }
      return m;
    }
    case 6:
      readLen(buf, r, info);
      return item(buf, r, depth + 1);
    default:
      if (info === 20) return false;
      if (info === 21) return true;
      if (info === 22) return null;
      if (info === 23) return undefined;
      throw new Error("cbor: unsupported simple/float value");
  }
}

export function mapGet(m: CborValue, key: string): CborValue {
  if (!(m instanceof Map)) throw new Error("cbor: expected map");
  return m.get(key);
}

export function asBytes(v: CborValue, what: string): Uint8Array {
  if (!(v instanceof Uint8Array)) throw new Error(`cbor: ${what} is not a byte string`);
  return v;
}
