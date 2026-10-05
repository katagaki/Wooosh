export interface Env {
  ROOM: DurableObjectNamespace;
  QUOTA: DurableObjectNamespace;

  /** HMAC key for admission tokens, App Attest key credentials and quota keys. Secret. */
  SERVER_SECRET: string;

  /** Comma-separated App IDs, `TEAMID.bundle.id`. */
  APPLE_APP_IDS: string;
  /** "true" accepts `appattestdevelop` keys. Development environment only. */
  APPLE_ALLOW_DEVELOPMENT?: string;

  PLAY_PACKAGE_NAME: string;
  /** Comma-separated release signing-certificate SHA-256 digests, base64url. */
  PLAY_CERT_SHA256: string;
  /** Google service account with the Play Integrity scope. Secrets. */
  PLAY_SA_EMAIL?: string;
  PLAY_SA_PRIVATE_KEY?: string;
  /** "true" accepts non-Play builds. Development environment only. */
  PLAY_ALLOW_UNRECOGNIZED?: string;

  /** "true" lets unverified clients relay. Development environment only. */
  ANON_RELAY?: string;

  /** Test hooks: replace the bundled roots. Never set in a deployed environment. */
  APPLE_ROOT_OVERRIDE?: string;
  TPM_ROOTS_OVERRIDE?: string;
}

export type Tier = "app" | "hardware" | "unverified";

export const QUOTA_ROOMS_PER_DAY: Record<Tier, number> = {
  app: 50,
  hardware: 50,
  unverified: 5,
};

export const ADMISSION_TOKEN_TTL_MS = 24 * 60 * 60 * 1000;
export const KEY_CREDENTIAL_TTL_MS = 365 * 24 * 60 * 60 * 1000;
export const LONE_OCCUPANT_TTL_MS = 120 * 1000;
