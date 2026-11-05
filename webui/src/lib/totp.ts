/** RFC 6238 TOTP (and Steam Guard) from a base32 secret or an otpauth:// / steam:// URI, using WebCrypto. */

export interface TotpParams {
  secret: Uint8Array<ArrayBuffer>;
  digits: number;
  period: number;
  algorithm: "SHA-1" | "SHA-256" | "SHA-512";
  /** Steam Guard: 5 characters from Steam's alphabet instead of decimal digits. */
  steam?: boolean;
}

export interface TotpDefaults {
  digits?: number;
  period?: number;
  algorithm?: string;
  /** `totp.otpType` written by Monica for Android: TOTP, STEAM, HOTP, YANDEX or MOTP. */
  otpType?: string;
}

const BASE32 = "ABCDEFGHIJKLMNOPQRSTUVWXYZ234567";
const STEAM_CHARS = "23456789BCDFGHJKMNPQRTVWXY";
const ALGORITHMS: Record<string, TotpParams["algorithm"]> = { SHA1: "SHA-1", SHA256: "SHA-256", SHA512: "SHA-512" };

function base32Decode(input: string): Uint8Array<ArrayBuffer> | null {
  const clean = input.toUpperCase().replace(/[\s=-]/g, "");
  if (!clean || [...clean].some((c) => !BASE32.includes(c))) return null;
  const out: number[] = [];
  let bits = 0;
  let value = 0;
  for (const char of clean) {
    value = (value << 5) | BASE32.indexOf(char);
    bits += 5;
    if (bits >= 8) {
      out.push((value >>> (bits - 8)) & 0xff);
      bits -= 8;
    }
  }
  return new Uint8Array(out);
}

/** Steam shared secrets are usually base64 (`steam://cnOgv/Kd…=`); Monica stores them as base32. */
function steamSecret(input: string): Uint8Array<ArrayBuffer> | null {
  const clean = input.replace(/\s/g, "");
  if (/^[A-Z2-7]+=*$/.test(clean)) return base32Decode(clean);
  try {
    const binary = atob(clean.replace(/-/g, "+").replace(/_/g, "/"));
    return Uint8Array.from(binary, (c) => c.charCodeAt(0));
  } catch {
    return null;
  }
}

/**
 * The kind of one-time code an item holds. Only time-based codes can be computed here;
 * HOTP needs the counter, Yandex and mOTP need a PIN, all kept by the Monica app.
 */
export function otpKind(raw: string | undefined, otpType?: string): string {
  const declared = otpType?.trim().toUpperCase();
  if (declared) return declared;
  const value = raw?.trim().toLowerCase() ?? "";
  if (value.startsWith("steam://")) return "STEAM";
  if (value.startsWith("otpauth://hotp")) return "HOTP";
  if (value.startsWith("otpauth://yaotp")) return "YANDEX";
  if (value.startsWith("motp://")) return "MOTP";
  return "TOTP";
}

export function parseTotp(raw: string | undefined, defaults: TotpDefaults = {}): TotpParams | null {
  if (!raw?.trim()) return null;
  const kind = otpKind(raw, defaults.otpType);
  if (kind !== "TOTP" && kind !== "STEAM") return null;
  let secret = raw.trim();
  let digits = defaults.digits ?? 6;
  let period = defaults.period ?? 30;
  let algorithm = (defaults.algorithm || "SHA1").toUpperCase();
  let steam = kind === "STEAM";
  let bytes: Uint8Array<ArrayBuffer> | null;
  if (secret.toLowerCase().startsWith("otpauth://")) {
    try {
      const url = new URL(secret);
      secret = url.searchParams.get("secret") ?? "";
      digits = Number(url.searchParams.get("digits") ?? digits);
      period = Number(url.searchParams.get("period") ?? period);
      algorithm = (url.searchParams.get("algorithm") ?? algorithm).toUpperCase();
      steam ||= url.searchParams.get("encoder")?.toLowerCase() === "steam";
    } catch {
      return null;
    }
    bytes = base32Decode(secret);
  } else if (secret.toLowerCase().startsWith("steam://")) {
    steam = true;
    bytes = steamSecret(secret.slice("steam://".length));
  } else {
    bytes = steam ? steamSecret(secret) : base32Decode(secret);
  }
  if (!bytes || !bytes.length) return null;
  if (steam) return { secret: bytes, digits: 5, period: 30, algorithm: "SHA-1", steam: true };
  // Accept "SHA1", "SHA-256", "sha512", "HmacSHA256"…; an unknown name must not silently become SHA-1.
  const algo = ALGORITHMS[algorithm.replace(/^HMAC/, "").replace(/[^A-Z0-9]/g, "")];
  if (!algo) return null;
  return {
    secret: bytes,
    digits: digits >= 6 && digits <= 10 ? digits : 6,
    period: period >= 1 && period <= 300 ? period : 30,
    algorithm: algo
  };
}

export async function totpCode(params: TotpParams, now = Date.now()): Promise<string> {
  const counter = Math.floor(now / 1000 / params.period);
  const message = new Uint8Array(8);
  new DataView(message.buffer).setBigUint64(0, BigInt(counter));
  const key = await crypto.subtle.importKey("raw", params.secret, { name: "HMAC", hash: params.algorithm }, false, ["sign"]);
  const hmac = new Uint8Array(await crypto.subtle.sign("HMAC", key, message));
  const offset = hmac[hmac.length - 1] & 0x0f;
  let binary = ((hmac[offset] & 0x7f) << 24) | (hmac[offset + 1] << 16) | (hmac[offset + 2] << 8) | hmac[offset + 3];
  if (params.steam) {
    let code = "";
    for (let i = 0; i < 5; i++) {
      code += STEAM_CHARS[binary % STEAM_CHARS.length];
      binary = Math.floor(binary / STEAM_CHARS.length);
    }
    return code;
  }
  return String(binary % 10 ** params.digits).padStart(params.digits, "0");
}

export function secondsRemaining(period: number, now = Date.now()): number {
  return period - (Math.floor(now / 1000) % period);
}
