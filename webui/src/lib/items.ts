/**
 * Item types and payload schema (protocol §2). The detail view and the editor
 * are both driven by the field descriptors below, so adding a field here is
 * enough to show and edit it.
 */

import { t, type MessageKey } from "../i18n";

export type ItemTypeKey =
  | "login"
  | "note"
  | "card"
  | "document"
  | "totp"
  | "passkey"
  | "wifi"
  | "ssh-key"
  | "billing-address"
  | "payment-account";

export interface ItemTypeInfo {
  id: number;
  key: ItemTypeKey;
  icon: string;
  labelKey: MessageKey;
}

export const itemTypes: ItemTypeInfo[] = [
  { id: 1, key: "login", icon: "key", labelKey: "type.login" },
  { id: 2, key: "note", icon: "sticky_note_2", labelKey: "type.note" },
  { id: 3, key: "card", icon: "credit_card", labelKey: "type.card" },
  { id: 4, key: "document", icon: "badge", labelKey: "type.document" },
  { id: 5, key: "totp", icon: "pin", labelKey: "type.totp" },
  { id: 6, key: "passkey", icon: "passkey", labelKey: "type.passkey" },
  { id: 7, key: "wifi", icon: "wifi", labelKey: "type.wifi" },
  { id: 8, key: "ssh-key", icon: "terminal", labelKey: "type.sshKey" },
  { id: 9, key: "billing-address", icon: "home_pin", labelKey: "type.address" },
  { id: 10, key: "payment-account", icon: "account_balance_wallet", labelKey: "type.payment" }
];

const unknownType: ItemTypeInfo = { id: 0, key: "note", icon: "help", labelKey: "type.unknown" };

/** Types a person can create in the WebUI. Passkeys need a real authenticator, so only apps create them. */
export const creatableTypes = itemTypes.filter((t) => t.key !== "passkey");

export function typeInfo(id: number): ItemTypeInfo {
  return itemTypes.find((t) => t.id === id) ?? unknownType;
}

export function typeByKey(key: string): ItemTypeInfo | undefined {
  return itemTypes.find((t) => t.key === key);
}

export interface CustomField {
  name: string;
  value: string;
  hidden?: boolean;
}

/** Decrypted item content. Unknown keys from other clients are preserved on save. */
export interface ItemPayload {
  v: 1;
  name: string;
  notes?: string;
  uris?: string[];
  fields?: CustomField[];
  login?: { username?: string; password?: string; totp?: string; loginType?: string; ssoProvider?: string };
  card?: { holder?: string; number?: string; expiry?: string; cvv?: string; brand?: string };
  document?: { kind?: string; number?: string; holder?: string; issuedBy?: string; expiry?: string };
  /** `otpType` (TOTP, STEAM, HOTP, YANDEX, MOTP) is written by Monica for Android. */
  totp?: { secret?: string; issuer?: string; account?: string; digits?: number; period?: number; algorithm?: string; otpType?: string };
  /** Same shape as Bitwarden `fido2Credentials`; `keyValue` is the PKCS#8 private key (protocol §2.1). */
  passkey?: {
    credentialId?: string;
    keyType?: string;
    keyAlgorithm?: string;
    keyCurve?: string;
    keyValue?: string;
    rpId?: string;
    rpName?: string;
    userHandle?: string;
    userName?: string;
    userDisplayName?: string;
    counter?: number;
    discoverable?: boolean;
    creationDate?: string;
  };
  wifi?: { ssid?: string; password?: string; security?: string; hidden?: boolean };
  sshKey?: { privateKey?: string; publicKey?: string; fingerprint?: string };
  address?: { fullName?: string; line1?: string; line2?: string; city?: string; state?: string; zip?: string; country?: string; phone?: string; email?: string };
  payment?: { provider?: string; account?: string; holder?: string; secret?: string };
  passwordHistory?: { password: string; lastUsedAt: string }[];
  [extension: string]: unknown;
}

export type FieldKind = "text" | "secret" | "multiline" | "secret-multiline" | "totp" | "select" | "boolean";

export interface FieldDescriptor {
  /** Dotted path inside the payload, e.g. `login.username`. */
  path: string;
  labelKey: MessageKey;
  kind: FieldKind;
  mono?: boolean;
  options?: string[];
  /** Option labels are the messages `${optionPrefix}.${value}`; values without one are shown as-is. */
  optionPrefix?: string;
  /** What an unset field means. The detail view does not list a field holding this value. */
  defaultValue?: string;
  /** Shown and edited only when this returns true. */
  visible?: (payload: ItemPayload) => boolean;
  /** Offer the password generator in the editor. */
  generate?: boolean;
}

/** Display text for a select value. */
export function optionLabel(field: FieldDescriptor, value: string): string {
  if (!field.optionPrefix || !value) return value;
  const key = `${field.optionPrefix}.${value}` as MessageKey;
  const text = t(key);
  return text === key ? value : text;
}

/** The field's options, plus the stored value when another client wrote one this version does not know. */
export function selectOptions(field: FieldDescriptor, current: string): string[] {
  const options = field.options ?? [];
  return current && !options.includes(current) ? [...options, current] : options;
}

export function isFieldVisible(field: FieldDescriptor, payload: ItemPayload): boolean {
  return !field.visible || field.visible(payload);
}

/** Third-party sign-in providers, as Monica for Android stores them (`SsoProvider`). */
const ssoProviders = ["GOOGLE", "APPLE", "FACEBOOK", "MICROSOFT", "GITHUB", "TWITTER", "WECHAT", "QQ", "WEIBO", "OTHER"];

const f = (path: string, labelKey: MessageKey, kind: FieldKind = "text", extra: Partial<FieldDescriptor> = {}): FieldDescriptor => ({
  path,
  labelKey,
  kind,
  ...extra
});

export const typeFields: Record<ItemTypeKey, FieldDescriptor[]> = {
  login: [
    f("login.username", "field.username"),
    f("login.password", "field.password", "secret", { generate: true, mono: true }),
    f("login.totp", "field.totp", "totp"),
    f("login.loginType", "field.loginType", "select", { options: ["PASSWORD", "SSO"], optionPrefix: "loginType", defaultValue: "PASSWORD" }),
    f("login.ssoProvider", "field.ssoProvider", "select", {
      options: ssoProviders,
      optionPrefix: "sso",
      visible: (payload) => payload.login?.loginType === "SSO"
    })
  ],
  note: [],
  card: [
    f("card.holder", "field.cardholder"),
    f("card.number", "field.cardNumber", "secret", { mono: true }),
    f("card.brand", "field.brand"),
    f("card.expiry", "field.expiry"),
    f("card.cvv", "field.cvv", "secret", { mono: true })
  ],
  document: [
    f("document.kind", "field.documentKind", "select", {
      options: ["ID_CARD", "PASSPORT", "DRIVER_LICENSE", "SOCIAL_SECURITY", "OTHER"],
      optionPrefix: "documentKind"
    }),
    f("document.holder", "field.holder"),
    f("document.number", "field.documentNumber", "secret", { mono: true }),
    f("document.issuedBy", "field.issuedBy"),
    f("document.expiry", "field.expiry")
  ],
  totp: [
    f("totp.issuer", "field.issuer"),
    f("totp.account", "field.account"),
    f("totp.secret", "field.totpSecret", "totp")
  ],
  passkey: [
    f("passkey.rpId", "field.rpId"),
    f("passkey.userName", "field.username"),
    f("passkey.userDisplayName", "field.displayName"),
    f("passkey.credentialId", "field.credentialId", "text", { mono: true })
  ],
  wifi: [
    f("wifi.ssid", "field.ssid"),
    f("wifi.password", "field.password", "secret", { generate: true, mono: true }),
    f("wifi.security", "field.security", "select", {
      options: ["WPA2_WPA3", "WPA3", "WPA_WPA2", "WEP", "NONE", "WPA2_ENTERPRISE", "WPA3_ENTERPRISE"],
      optionPrefix: "wifiSecurity"
    }),
    f("wifi.hidden", "field.hiddenNetwork", "boolean")
  ],
  "ssh-key": [
    f("sshKey.publicKey", "field.publicKey", "multiline", { mono: true }),
    f("sshKey.privateKey", "field.privateKey", "secret-multiline", { mono: true }),
    f("sshKey.fingerprint", "field.fingerprint", "text", { mono: true })
  ],
  "billing-address": [
    f("address.fullName", "field.fullName"),
    f("address.line1", "field.addressLine1"),
    f("address.line2", "field.addressLine2"),
    f("address.city", "field.city"),
    f("address.state", "field.state"),
    f("address.zip", "field.zip"),
    f("address.country", "field.country"),
    f("address.phone", "field.phone"),
    f("address.email", "field.email")
  ],
  "payment-account": [
    f("payment.provider", "field.provider"),
    f("payment.holder", "field.holder"),
    f("payment.account", "field.account"),
    f("payment.secret", "field.password", "secret", { mono: true })
  ]
};

/** Types that carry website URIs. */
export const typesWithUris: ItemTypeKey[] = ["login", "passkey"];

export function getPath(payload: ItemPayload, path: string): unknown {
  return path.split(".").reduce<unknown>((value, key) => (value && typeof value === "object" ? (value as Record<string, unknown>)[key] : undefined), payload);
}

export function setPath(payload: ItemPayload, path: string, value: unknown) {
  const keys = path.split(".");
  let target = payload as Record<string, unknown>;
  for (const key of keys.slice(0, -1)) {
    if (!target[key] || typeof target[key] !== "object") target[key] = {};
    target = target[key] as Record<string, unknown>;
  }
  target[keys[keys.length - 1]] = value;
}

export function emptyPayload(): ItemPayload {
  return { v: 1, name: "", notes: "", uris: [], fields: [] };
}

/** One line under the item name in lists. */
export function itemSubtitle(type: number, payload: ItemPayload): string {
  switch (typeInfo(type).key) {
    case "login":
      return payload.login?.username || hostOf(payload.uris?.[0]) || "";
    case "card": {
      const number = payload.card?.number?.replace(/\s/g, "") ?? "";
      return [payload.card?.brand, number ? `•••• ${number.slice(-4)}` : ""].filter(Boolean).join(" ");
    }
    case "document": {
      const kind = typeFields.document.find((field) => field.path === "document.kind");
      return kind && payload.document?.kind ? optionLabel(kind, payload.document.kind) : "";
    }
    case "totp":
      return payload.totp?.account || payload.totp?.issuer || "";
    case "passkey":
      return payload.passkey?.userName || payload.passkey?.rpId || "";
    case "wifi":
      return payload.wifi?.ssid ?? "";
    case "ssh-key":
      return payload.sshKey?.fingerprint ?? "";
    case "billing-address":
      return [payload.address?.city, payload.address?.country].filter(Boolean).join(", ");
    case "payment-account":
      return payload.payment?.provider ?? "";
    default:
      return (payload.notes ?? "").split("\n")[0].slice(0, 80);
  }
}

export function hostOf(uri?: string): string {
  if (!uri) return "";
  try {
    return new URL(uri.includes("://") ? uri : `https://${uri}`).hostname;
  } catch {
    return uri;
  }
}

/** Text that the list search matches against. */
export function searchText(payload: ItemPayload): string {
  return [payload.name, payload.notes, payload.login?.username, ...(payload.uris ?? []), payload.wifi?.ssid, payload.totp?.issuer, payload.passkey?.rpId]
    .filter(Boolean)
    .join(" ")
    .toLowerCase();
}

// Password generator --------------------------------------------------------

const sets = {
  lower: "abcdefghijkmnopqrstuvwxyz",
  upper: "ABCDEFGHJKLMNPQRSTUVWXYZ",
  digits: "23456789",
  symbols: "!@#$%^&*-_=+?"
};

export function generatePassword(length = 20, options = { lower: true, upper: true, digits: true, symbols: true }): string {
  const pools = (Object.keys(sets) as (keyof typeof sets)[]).filter((k) => options[k]).map((k) => sets[k]);
  const all = pools.join("");
  const pick = (pool: string) => {
    // Rejection sampling avoids modulo bias.
    const limit = 256 - (256 % pool.length);
    const buf = new Uint8Array(1);
    do globalThis.crypto.getRandomValues(buf);
    while (buf[0] >= limit);
    return pool[buf[0] % pool.length];
  };
  const chars = pools.map(pick);
  while (chars.length < length) chars.push(pick(all));
  for (let i = chars.length - 1; i > 0; i--) {
    const j = Math.floor((globalThis.crypto.getRandomValues(new Uint32Array(1))[0] / 2 ** 32) * (i + 1));
    [chars[i], chars[j]] = [chars[j], chars[i]];
  }
  return chars.join("");
}
