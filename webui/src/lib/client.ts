/**
 * Monica Cloud API client (protocol v1). Framework-free so the same code runs
 * in the WebUI and in the Node end-to-end test (`scripts/e2e.ts`).
 */

import * as mc from "./crypto.ts";

// ---------------------------------------------------------------------------
// Wire types

export interface ServerConfig {
  name: string;
  version: string;
  protocolVersion: number;
  domain: string;
  initialized: boolean;
  signupsAllowed: boolean;
  sendsAllowed: boolean;
  kdf: number;
  kdfIterations: number;
  attachmentMaxBytes: number;
  features: string[];
}

export interface Account {
  id: string;
  email: string;
  name: string;
  role: "owner" | "admin" | "user";
}

export interface Profile extends Account {
  kdf: number;
  kdfIterations: number;
  creationDate: string;
  revisionDate: string;
  lastLoginDate: string | null;
}

export interface TokenResponse {
  accessToken: string;
  expiresIn: number;
  tokenType: "Bearer";
  refreshToken: string;
  protectedUserKey: string;
  kdf: number;
  kdfIterations: number;
  account: Account;
  deviceId: string;
}

export interface AttachmentRecord {
  id: string;
  fileName: string;
  key: string;
  size: number;
  creationDate: string;
}

export interface ItemRecord {
  id: string;
  type: number;
  folderId: string | null;
  favorite: boolean;
  key: string;
  data: string;
  attachments: AttachmentRecord[];
  creationDate: string;
  revisionDate: string;
  deletedDate: string | null;
  archivedDate: string | null;
  ref?: string | null;
}

export interface FolderRecord {
  id: string;
  name: string;
  creationDate: string;
  revisionDate: string;
}

export interface SendRecord {
  id: string;
  accessId: string;
  type: 0 | 1;
  key: string;
  name: string;
  notes: string | null;
  text: string | null;
  fileName: string | null;
  size: number | null;
  uploaded: boolean;
  passwordProtected: boolean;
  maxAccessCount: number | null;
  accessCount: number;
  disabled: boolean;
  hideEmail: boolean;
  expirationDate: string | null;
  deletionDate: string;
  creationDate: string;
  revisionDate: string;
}

export interface PublicSend {
  id: string;
  accessId: string;
  type: 0 | 1;
  name: string;
  text: string | null;
  fileName: string | null;
  size: number | null;
  expirationDate: string | null;
  creatorIdentifier: string | null;
  downloadToken?: string;
}

export interface Tombstone {
  type: "item" | "folder" | "send";
  id: string;
  deletedAt: string;
}

export interface SyncResponse {
  full: boolean;
  revisionDate: string;
  profile: Profile;
  folders: FolderRecord[];
  items: ItemRecord[];
  sends: SendRecord[];
  deleted: Tombstone[];
}

export interface DeviceRecord {
  id: string;
  identifier: string;
  name: string;
  type: string;
  appVersion: string;
  creationDate: string;
  lastSeenDate: string;
  lastIp: string;
  current: boolean;
}

export interface EventRecord {
  id: string;
  userId: string | null;
  actor: string;
  action: string;
  target: string;
  ip: string;
  deviceName: string | null;
  date: string;
}

export interface AccountStats {
  items: number;
  itemsByType: { type: number; count: number }[];
  storageUsedBytes: number;
  storageQuotaBytes: number | null;
  devices: number;
  revisionDate: string;
}

export interface AdminUser extends Profile {
  enabled: boolean;
  quotaBytes: number | null;
  effectiveQuotaBytes: number | null;
  storageUsedBytes: number;
  itemCount: number;
  deviceCount: number;
}

export interface RuntimeSettings {
  signupsAllowed: boolean;
  sendsAllowed: boolean;
  defaultQuotaBytes: number | null;
  eventRetentionDays: number;
}

export interface Diagnostics {
  version: string;
  uptimeSeconds: number;
  serverTime: string;
  database: { backend: string; path: string; sizeBytes: number };
  storage: { dataFolder: string; attachments: number; attachmentBytes: number };
  counts: { users: number; items: number; sends: number };
  webVault: { enabled: boolean; folder: string; present: boolean };
  config: Record<string, unknown>;
}

export interface ItemWrite {
  type: number;
  folderId: string | null;
  favorite: boolean;
  key: string;
  data: string;
  archived: boolean;
  lastKnownRevisionDate?: string;
}

export interface SendWrite {
  type: 0 | 1;
  key: string;
  name: string;
  notes?: string | null;
  text?: string | null;
  fileName?: string | null;
  size?: number | null;
  passwordHash?: string | null;
  removePassword?: boolean;
  maxAccessCount?: number | null;
  expirationDate?: string | null;
  deletionDate: string;
  hideEmail?: boolean;
  disabled?: boolean;
}

export interface DeviceInfo {
  identifier: string;
  name: string;
  type: "web" | "android" | "desktop" | "cli" | "ios";
  appVersion?: string;
}

export class ApiError extends Error {
  readonly status: number;
  readonly code: string;
  /** Server copy of the object on a 409 conflict. */
  readonly current?: unknown;

  constructor(status: number, code: string, message: string, current?: unknown) {
    super(message);
    this.status = status;
    this.code = code;
    this.current = current;
  }
}

// ---------------------------------------------------------------------------
// Client

interface RequestOptions {
  body?: unknown;
  raw?: BodyInit;
  auth?: boolean;
  binary?: boolean;
}

/**
 * Optional coordination of token refreshes between clients that share one session (the
 * WebUI's tabs). The server rotates the refresh token on every use, so two tabs refreshing
 * with the same token would sign each other out.
 */
export interface RefreshCoordinator {
  /** Runs `task` while no other client refreshes, e.g. under a Web Lock. */
  exclusive<T>(task: () => Promise<T>): Promise<T>;
  /** Resolves true once another client has replaced the session that held `staleToken`. */
  replaced(staleToken: string): Promise<boolean>;
}

export class MonicaClient {
  /** Called whenever tokens change, so the app can persist the session. */
  onSession?: (session: TokenResponse | null) => void;
  session: TokenResponse | null = null;
  coordinator?: RefreshCoordinator;
  private refreshing: Promise<void> | null = null;

  readonly baseUrl: string;

  constructor(baseUrl = "") {
    this.baseUrl = baseUrl;
  }

  private async request<T>(method: string, path: string, options: RequestOptions = {}, retried = false): Promise<T> {
    const headers: Record<string, string> = {};
    let body: BodyInit | undefined;
    if (options.raw !== undefined) {
      headers["Content-Type"] = "application/octet-stream";
      body = options.raw;
    } else if (options.body !== undefined) {
      headers["Content-Type"] = "application/json";
      body = JSON.stringify(options.body);
    }
    let accessToken: string | undefined;
    if (options.auth !== false) {
      if (!this.session) throw new ApiError(401, "unauthorized", "Not logged in");
      accessToken = this.session.accessToken;
      headers.Authorization = `Bearer ${accessToken}`;
    }

    const response = await fetch(`${this.baseUrl}/api/v1${path}`, { method, headers, body });
    if (response.status === 401 && accessToken && !retried && this.session) {
      // Tokens may have been renewed (here or in another tab) while this request was in flight.
      if (this.session.accessToken === accessToken) await this.refresh();
      return this.request<T>(method, path, options, true);
    }
    if (!response.ok) {
      const error = await response.json().catch(() => ({ error: "http", message: response.statusText }));
      throw new ApiError(response.status, error.error ?? "http", error.message ?? response.statusText, error.current);
    }
    if (options.binary) return new Uint8Array(await response.arrayBuffer()) as T;
    return (await response.json()) as T;
  }

  private setSession(session: TokenResponse | null) {
    this.session = session;
    this.onSession?.(session);
  }

  // Discovery and authentication -------------------------------------------

  config() {
    return this.request<ServerConfig>("GET", "/config", { auth: false });
  }

  prelogin(email: string) {
    return this.request<{ kdf: number; kdfIterations: number }>("POST", "/accounts/prelogin", {
      body: { email },
      auth: false
    });
  }

  /** Create an account. The master password never leaves this function. */
  async register(email: string, name: string, masterPassword: string, kdfIterations: number) {
    const keys = await mc.deriveMasterKeys(email, masterPassword, kdfIterations);
    const userKey = await mc.generateUserKey();
    const protectedUserKey = await mc.encryptBytes(keys.wrapKey, userKey.raw);
    return this.request<{ id: string; email: string; role: string }>("POST", "/accounts/register", {
      auth: false,
      body: { email, name, authHash: keys.authHash, kdf: mc.KDF_PBKDF2, kdfIterations, protectedUserKey }
    });
  }

  /** Log in and unlock. Returns the decrypted account key, which must stay in memory only. */
  async login(email: string, masterPassword: string, device: DeviceInfo): Promise<mc.UserKey> {
    const { kdfIterations } = await this.prelogin(email);
    const keys = await mc.deriveMasterKeys(email, masterPassword, kdfIterations);
    const session = await this.request<TokenResponse>("POST", "/auth/token", {
      auth: false,
      body: { grantType: "password", email, authHash: keys.authHash, device }
    });
    const userKey = await mc.unwrapUserKey(keys.wrapKey, session.protectedUserKey);
    this.setSession(session);
    return userKey;
  }

  /** Unlock an existing session (e.g. after a page reload) without a server round trip. */
  async unlock(masterPassword: string): Promise<mc.UserKey> {
    if (!this.session) throw new ApiError(401, "unauthorized", "Not logged in");
    const keys = await mc.deriveMasterKeys(this.session.account.email, masterPassword, this.session.kdfIterations);
    return mc.unwrapUserKey(keys.wrapKey, this.session.protectedUserKey);
  }

  async refresh(): Promise<void> {
    if (!this.session) throw new ApiError(401, "unauthorized", "Not logged in");
    if (!this.refreshing) {
      const stale = this.session.refreshToken;
      const task = async () => {
        // Another client may have refreshed and shared new tokens while this one waited.
        if (this.session && this.session.refreshToken !== stale) return;
        try {
          const session = await this.request<TokenResponse>("POST", "/auth/token", {
            auth: false,
            body: { grantType: "refresh_token", refreshToken: stale }
          });
          this.setSession(session);
        } catch (error) {
          const rejected = error instanceof ApiError && (error.status === 401 || error.status === 403);
          if (rejected && error.status === 401 && (await this.coordinator?.replaced(stale))) return;
          // Only a server verdict ends the session; being offline for a moment must not log the user out.
          if (rejected && (!this.session || this.session.refreshToken === stale)) this.setSession(null);
          throw error;
        }
      };
      this.refreshing = (this.coordinator ? this.coordinator.exclusive(task) : task()).finally(() => {
        this.refreshing = null;
      });
    }
    return this.refreshing;
  }

  async logout() {
    try {
      if (this.session) await this.request("POST", "/auth/logout", { body: {} });
    } finally {
      this.setSession(null);
    }
  }

  // Account ----------------------------------------------------------------

  profile() {
    return this.request<Profile>("GET", "/accounts/profile");
  }

  updateProfile(name: string) {
    return this.request<Profile>("PUT", "/accounts/profile", { body: { name } });
  }

  stats() {
    return this.request<AccountStats>("GET", "/accounts/stats");
  }

  revisionDate() {
    return this.request<{ revisionDate: string }>("GET", "/accounts/revision-date");
  }

  /** Re-wrap the same userKey under a new master password; all sessions are logged out. */
  async changePassword(currentPassword: string, newPassword: string, userKey: mc.UserKey, kdfIterations?: number) {
    const email = this.session!.account.email;
    const current = await mc.deriveMasterKeys(email, currentPassword, this.session!.kdfIterations);
    const iterations = kdfIterations ?? this.session!.kdfIterations;
    const next = await mc.deriveMasterKeys(email, newPassword, iterations);
    await this.request("POST", "/accounts/password", {
      body: {
        authHash: current.authHash,
        newAuthHash: next.authHash,
        newProtectedUserKey: await mc.encryptBytes(next.wrapKey, userKey.raw),
        kdfIterations: iterations
      }
    });
    this.setSession(null);
  }

  async deleteAccount(masterPassword: string) {
    const keys = await mc.deriveMasterKeys(this.session!.account.email, masterPassword, this.session!.kdfIterations);
    await this.request("POST", "/accounts/delete", { body: { authHash: keys.authHash } });
    this.setSession(null);
  }

  events(limit = 100, before?: string) {
    const query = new URLSearchParams({ limit: String(limit), ...(before ? { before } : {}) });
    return this.request<{ data: EventRecord[] }>("GET", `/events?${query}`).then((r) => r.data);
  }

  devices() {
    return this.request<{ data: DeviceRecord[] }>("GET", "/devices").then((r) => r.data);
  }

  revokeDevice(id: string) {
    return this.request("DELETE", `/devices/${encodeURIComponent(id)}`);
  }

  // Vault ------------------------------------------------------------------

  sync(since?: string) {
    return this.request<SyncResponse>("GET", since ? `/sync?since=${encodeURIComponent(since)}` : "/sync");
  }

  createItem(item: ItemWrite) {
    return this.request<ItemRecord>("POST", "/items", { body: item });
  }

  updateItem(id: string, item: ItemWrite) {
    return this.request<ItemRecord>("PUT", `/items/${id}`, { body: item });
  }

  importItems(folders: { ref: string; name: string }[], items: (ItemWrite & { ref?: string; folderRef?: string })[]) {
    return this.request<{ folders: { ref: string; folder: FolderRecord }[]; items: ItemRecord[] }>("POST", "/items/import", {
      body: { folders, items }
    });
  }

  trashItem(id: string) {
    return this.request<ItemRecord>("POST", `/items/${id}/trash`, { body: {} });
  }

  restoreItem(id: string) {
    return this.request<ItemRecord>("POST", `/items/${id}/restore`, { body: {} });
  }

  deleteItem(id: string) {
    return this.request("DELETE", `/items/${id}`);
  }

  bulk(action: "trash" | "restore" | "delete" | "move", ids: string[], folderId?: string | null) {
    return this.request<{ affected: number }>("POST", "/items/bulk", { body: { action, ids, folderId } });
  }

  createFolder(name: string) {
    return this.request<FolderRecord>("POST", "/folders", { body: { name } });
  }

  updateFolder(id: string, name: string) {
    return this.request<FolderRecord>("PUT", `/folders/${id}`, { body: { name } });
  }

  deleteFolder(id: string) {
    return this.request("DELETE", `/folders/${id}`);
  }

  /** Encrypt and upload an attachment. `itemKey` is the item's decrypted key. */
  async uploadAttachment(itemId: string, itemKey: CryptoKey, fileName: string, content: Uint8Array<ArrayBuffer>) {
    const attachmentRaw = mc.randomBytes(32);
    const attachmentKey = await mc.importAesKey(attachmentRaw);
    const encrypted = await mc.encryptBlob(attachmentKey, content);
    const registered = await this.request<{ id: string; uploadUrl: string }>("POST", `/items/${itemId}/attachments`, {
      body: {
        fileName: await mc.encryptString(itemKey, fileName),
        key: await mc.encryptBytes(itemKey, attachmentRaw),
        size: encrypted.length
      }
    });
    return this.request<ItemRecord>("PUT", `/items/${itemId}/attachments/${registered.id}`, { raw: encrypted });
  }

  async downloadAttachment(itemId: string, itemKey: CryptoKey, attachment: AttachmentRecord) {
    const blob = await this.request<Uint8Array<ArrayBuffer>>("GET", `/items/${itemId}/attachments/${attachment.id}`, { binary: true });
    const attachmentKey = await mc.importAesKey(await mc.decryptBytes(itemKey, attachment.key));
    return {
      fileName: await mc.decryptString(itemKey, attachment.fileName),
      content: await mc.decryptBlob(attachmentKey, blob)
    };
  }

  deleteAttachment(itemId: string, attachmentId: string) {
    return this.request<ItemRecord>("DELETE", `/items/${itemId}/attachments/${attachmentId}`);
  }

  // Send -------------------------------------------------------------------

  sends() {
    return this.request<{ data: SendRecord[] }>("GET", "/sends").then((r) => r.data);
  }

  createSend(send: SendWrite) {
    return this.request<SendRecord & { uploadUrl: string | null }>("POST", "/sends", { body: send });
  }

  uploadSendFile(id: string, encrypted: Uint8Array<ArrayBuffer>) {
    return this.request<SendRecord>("PUT", `/sends/${id}/file`, { raw: encrypted });
  }

  updateSend(id: string, send: SendWrite) {
    return this.request<SendRecord>("PUT", `/sends/${id}`, { body: send });
  }

  deleteSend(id: string) {
    return this.request("DELETE", `/sends/${id}`);
  }

  accessSend(accessId: string, passwordHash?: string) {
    return this.request<PublicSend>("POST", `/public/sends/${encodeURIComponent(accessId)}`, {
      auth: false,
      body: passwordHash ? { passwordHash } : {}
    });
  }

  downloadSendFile(accessId: string, token: string) {
    return this.request<Uint8Array<ArrayBuffer>>("GET", `/public/sends/${encodeURIComponent(accessId)}/file/${encodeURIComponent(token)}`, {
      auth: false,
      binary: true
    });
  }

  // Admin ------------------------------------------------------------------

  adminUsers() {
    return this.request<{ data: AdminUser[] }>("GET", "/admin/users").then((r) => r.data);
  }

  adminUpdateUser(id: string, patch: Partial<Pick<AdminUser, "enabled" | "role" | "quotaBytes">>) {
    return this.request<AdminUser>("PATCH", `/admin/users/${id}`, { body: patch });
  }

  adminDeauthUser(id: string) {
    return this.request("POST", `/admin/users/${id}/deauth`, { body: {} });
  }

  adminDeleteUser(id: string) {
    return this.request("DELETE", `/admin/users/${id}`);
  }

  adminDiagnostics() {
    return this.request<Diagnostics>("GET", "/admin/diagnostics");
  }

  adminSettings() {
    return this.request<RuntimeSettings>("GET", "/admin/settings");
  }

  adminSaveSettings(settings: RuntimeSettings) {
    return this.request<RuntimeSettings>("PUT", "/admin/settings", { body: settings });
  }

  adminEvents(limit = 200, before?: string) {
    const query = new URLSearchParams({ limit: String(limit), ...(before ? { before } : {}) });
    return this.request<{ data: EventRecord[] }>("GET", `/admin/events?${query}`).then((r) => r.data);
  }
}

// ---------------------------------------------------------------------------
// Send helpers shared by the owner UI and the public receive page

export async function encryptSendText(userKey: CryptoKey, name: string, text: string, notes?: string) {
  const keys = await mc.deriveSendKeys();
  return {
    keys,
    fields: {
      key: await mc.encryptBytes(userKey, keys.sendKey),
      name: await mc.encryptString(keys.contentKey, name),
      notes: notes ? await mc.encryptString(keys.contentKey, notes) : null,
      text: await mc.encryptString(keys.contentKey, text)
    }
  };
}

export function sendShareUrl(origin: string, accessId: string, sendKey: Uint8Array) {
  return `${origin.replace(/\/$/, "")}/#/send/${accessId}/${mc.b64urlEncode(sendKey)}`;
}
