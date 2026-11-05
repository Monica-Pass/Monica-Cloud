/**
 * Login state. Tokens survive a reload (sessionStorage, this tab only) and token rotations are
 * shared with other tabs signed in to the same account, but the decrypted account key lives in
 * memory only, so a reload or inactivity locks the vault until the master password is entered again.
 */

import { computed, ref, shallowRef } from "vue";
import { ApiError, MonicaClient, type ServerConfig, type TokenResponse } from "./client";
import type { UserKey } from "./crypto";
import { readStorage, removeStorage, writeStorage } from "./storage";

export type SessionState = "loading" | "offline" | "auth" | "locked" | "unlocked";

const SESSION_KEY = "monica.session";
const DEVICE_KEY = "monica.deviceId";
const EMAIL_KEY = "monica.lastEmail";
const AUTO_LOCK_MS = 15 * 60 * 1000;
/** How long a failed refresh waits for another tab's newer tokens before giving up. */
const PEER_REFRESH_WAIT_MS = 1500;

export const client = new MonicaClient("");
export const serverConfig = ref<ServerConfig | null>(null);
export const sessionState = ref<SessionState>("loading");
export const userKey = shallowRef<UserKey | null>(null);
export const account = ref(client.session?.account ?? null);
export const isAdmin = computed(() => account.value?.role === "owner" || account.value?.role === "admin");
export const lastEmail = ref(readStorage("local", EMAIL_KEY) ?? "");

const lockListeners: (() => void)[] = [];

/** Register cleanup (e.g. clearing decrypted vault data) that runs on lock and logout. */
export function onLock(listener: () => void) {
  lockListeners.push(listener);
}

// Tabs that share a device session --------------------------------------------
//
// Every tab of this browser signs in as the same device, and the server rotates that device's
// refresh token on each login and refresh. A tab that obtains new tokens therefore broadcasts
// them, the other tabs signed in to the same account take them over, and refreshes run under a
// Web Lock; otherwise a login or refresh in one tab would sign the others out.

type PeerMessage = { type: "session"; session: TokenResponse; at: number } | { type: "signed-out"; accountId: string };

const channel = typeof BroadcastChannel === "function" ? new BroadcastChannel("monica.session") : null;
/** When the current tokens were issued (in whichever tab); orders concurrent broadcasts. */
let sessionAt = 0;

function broadcast(message: PeerMessage) {
  try {
    channel?.postMessage(message);
  } catch {
    /* channel closed */
  }
}

channel?.addEventListener("message", (event: MessageEvent<PeerMessage>) => {
  const message = event.data;
  if (message?.type === "session") adopt(message.session, message.at);
  else if (message?.type === "signed-out" && client.session?.account.id === message.accountId) dropSession();
});

/** Take over newer tokens another tab obtained for the account signed in here. Never re-broadcasts. */
function adopt(session: TokenResponse, at: number) {
  const current = client.session;
  if (!current || current.account.id !== session.account.id || at <= sessionAt) return;
  client.session = session;
  account.value = session.account;
  persist(session, at);
}

function waitFor(condition: () => boolean, ms: number): Promise<boolean> {
  return new Promise((resolve) => {
    const deadline = Date.now() + ms;
    const check = () => {
      if (condition()) resolve(true);
      else if (Date.now() >= deadline) resolve(false);
      else window.setTimeout(check, 50);
    };
    check();
  });
}

client.coordinator = {
  exclusive: <T>(task: () => Promise<T>) => (navigator.locks ? (navigator.locks.request("monica.refresh", task) as Promise<T>) : task()),
  replaced: (stale) => waitFor(() => client.session !== null && client.session.refreshToken !== stale, PEER_REFRESH_WAIT_MS)
};

function persist(session: TokenResponse, at: number) {
  sessionAt = at;
  writeStorage("session", SESSION_KEY, JSON.stringify({ session, at }));
}

function restore(): boolean {
  const stored = readStorage("session", SESSION_KEY);
  if (!stored) return false;
  try {
    const parsed = JSON.parse(stored) as { session?: TokenResponse; at?: number } & Partial<TokenResponse>;
    // Older builds stored the token response itself.
    const session = parsed.session ?? (parsed as TokenResponse);
    if (!session.refreshToken || !session.account) throw new Error("incomplete session");
    client.session = session;
    sessionAt = parsed.at ?? 0;
    account.value = session.account;
    return true;
  } catch {
    removeStorage("session", SESSION_KEY);
    return false;
  }
}

client.onSession = (session) => {
  account.value = session?.account ?? null;
  if (session) {
    persist(session, Date.now());
    broadcast({ type: "session", session, at: sessionAt });
  } else {
    sessionAt = 0;
    removeStorage("session", SESSION_KEY);
    if (sessionState.value === "unlocked" || sessionState.value === "locked") {
      clearKeys();
      sessionState.value = "auth";
    }
  }
};

export async function initSession() {
  sessionState.value = "loading";
  restore();
  try {
    serverConfig.value = await client.config();
  } catch {
    sessionState.value = "offline";
    return;
  }
  sessionState.value = client.session ? "locked" : "auth";
}

function deviceInfo() {
  let identifier = readStorage("local", DEVICE_KEY);
  if (!identifier) {
    identifier = crypto.randomUUID();
    writeStorage("local", DEVICE_KEY, identifier);
  }
  const ua = navigator.userAgent;
  const browser = /Edg\//.test(ua) ? "Edge" : /Firefox\//.test(ua) ? "Firefox" : /Chrome\//.test(ua) ? "Chrome" : /Safari\//.test(ua) ? "Safari" : "Browser";
  const os = /Windows/.test(ua) ? "Windows" : /Android/.test(ua) ? "Android" : /iPhone|iPad/.test(ua) ? "iOS" : /Mac OS X/.test(ua) ? "macOS" : /Linux/.test(ua) ? "Linux" : "";
  return { identifier, name: [browser, os].filter(Boolean).join(" · "), type: "web" as const, appVersion: "webui" };
}

export async function login(email: string, password: string) {
  userKey.value = await client.login(email.trim(), password, deviceInfo());
  lastEmail.value = email.trim();
  writeStorage("local", EMAIL_KEY, lastEmail.value);
  sessionState.value = "unlocked";
  touch();
}

export async function register(email: string, name: string, password: string) {
  const iterations = serverConfig.value?.kdfIterations ?? 600_000;
  await client.register(email.trim(), name.trim(), password, iterations);
  serverConfig.value = await client.config();
  await login(email, password);
}

export async function unlock(password: string) {
  const key = await client.unlock(password);
  // Make sure the session is still valid before showing the vault.
  try {
    await client.revisionDate();
  } catch (error) {
    if (error instanceof ApiError && (error.status === 401 || error.status === 403)) {
      key.raw.fill(0);
      dropSession();
      throw error;
    }
    // Offline: the vault opens and syncs once the server is reachable again.
  }
  userKey.value = key;
  sessionState.value = "unlocked";
  touch();
}

export function lock() {
  if (sessionState.value !== "unlocked") return;
  clearKeys();
  sessionState.value = "locked";
}

export async function logout() {
  const accountId = client.session?.account.id;
  clearKeys();
  try {
    await client.logout();
  } catch {
    // Already invalid on the server; local state is cleared either way.
  }
  dropSession();
  // The other tabs share this device session, which the server just revoked.
  if (accountId) broadcast({ type: "signed-out", accountId });
}

/** Change the master password. The server signs out every session, so this one ends too. */
export async function changeMasterPassword(current: string, next: string) {
  if (!userKey.value) throw new Error("Vault is locked");
  const accountId = client.session?.account.id;
  await client.changePassword(current, next, userKey.value);
  endSession(accountId);
}

export async function deleteAccount(masterPassword: string) {
  const accountId = client.session?.account.id;
  await client.deleteAccount(masterPassword);
  endSession(accountId);
}

/** The server already revoked every session (password change, account deletion). */
function endSession(accountId?: string) {
  dropSession();
  if (accountId) broadcast({ type: "signed-out", accountId });
}

/** Keep the account menu in step with a renamed profile. Tokens are unchanged. */
export function updateAccountName(name: string) {
  const session = client.session;
  if (!session) return;
  client.session = { ...session, account: { ...session.account, name } };
  account.value = client.session.account;
  persist(client.session, sessionAt);
}

function dropSession() {
  clearKeys();
  client.session = null;
  account.value = null;
  sessionAt = 0;
  removeStorage("session", SESSION_KEY);
  if (sessionState.value !== "loading" && sessionState.value !== "offline") sessionState.value = "auth";
}

function clearKeys() {
  userKey.value?.raw.fill(0);
  userKey.value = null;
  for (const listener of lockListeners) listener();
}

// Auto-lock after inactivity ------------------------------------------------

let lastActivity = Date.now();
function touch() {
  lastActivity = Date.now();
}
for (const event of ["pointerdown", "keydown", "wheel", "touchstart"]) {
  window.addEventListener(event, touch, { passive: true });
}
window.setInterval(() => {
  if (sessionState.value === "unlocked" && Date.now() - lastActivity > AUTO_LOCK_MS) lock();
}, 30_000);
