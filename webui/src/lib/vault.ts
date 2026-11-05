/**
 * Decrypted, in-memory vault. Fetches incremental sync from the server,
 * decrypts with the account key and exposes reactive lists to the views.
 */

import { markRaw, reactive, toRaw } from "vue";
import { ApiError, type AttachmentRecord, type FolderRecord, type ItemRecord, type SendRecord, type SendWrite, sendShareUrl } from "./client";
import * as mc from "./crypto";
import { nameCollator } from "./format";
import { emptyPayload, searchText, type ItemPayload } from "./items";
import { client, onLock, userKey } from "./session";

/**
 * A decrypted item. Items are immutable and marked raw: Vue tracks the `vault.items` array,
 * not every field of thousands of payloads, and every change replaces the item object.
 */
export interface VaultItem {
  readonly id: string;
  readonly type: number;
  readonly folderId: string | null;
  readonly favorite: boolean;
  readonly trashed: boolean;
  readonly archived: boolean;
  readonly creationDate: string;
  readonly revisionDate: string;
  readonly attachments: AttachmentRecord[];
  readonly payload: ItemPayload;
  readonly itemKeyRaw: Uint8Array<ArrayBuffer> | null;
  /** Lower-cased text the list search matches. */
  readonly search: string;
  /** Set when the item could not be decrypted (e.g. written by a newer client). */
  readonly error?: string;
}

export interface VaultFolder {
  id: string;
  name: string;
}

export interface VaultSend {
  record: SendRecord;
  name: string;
  notes: string;
  text: string;
  fileName: string;
  shareUrl: string;
}

export interface ItemDraft {
  id?: string;
  type: number;
  folderId: string | null;
  favorite: boolean;
  archived: boolean;
  payload: ItemPayload;
}

export class ConflictError extends Error {
  constructor(readonly server: VaultItem) {
    super("conflict");
  }
}

export const vault = reactive({
  items: [] as VaultItem[],
  folders: [] as VaultFolder[],
  sends: [] as VaultSend[],
  revisionDate: "",
  syncing: false,
  lastSyncAt: null as Date | null,
  error: ""
});

/**
 * Bumped on every lock. Work that started before a lock (a sync, a save) finishes after the
 * keys are gone and must not write decrypted data back into the cleared vault.
 */
let generation = 0;
let pending: Promise<void> | null = null;
let syncRuns = 0;

onLock(() => {
  generation++;
  pending = null;
  vault.items = [];
  vault.folders = [];
  vault.sends = [];
  vault.revisionDate = "";
  vault.lastSyncAt = null;
  vault.syncing = false;
  vault.error = "";
});

function key(): CryptoKey {
  if (!userKey.value) throw new Error("Vault is locked");
  return userKey.value.key;
}

async function decryptItem(record: ItemRecord): Promise<VaultItem> {
  const base = {
    id: record.id,
    type: record.type,
    folderId: record.folderId,
    favorite: record.favorite,
    trashed: Boolean(record.deletedDate),
    archived: Boolean(record.archivedDate),
    creationDate: record.creationDate,
    revisionDate: record.revisionDate,
    attachments: record.attachments
  };
  try {
    const decrypted = await mc.decryptItem<ItemPayload>(key(), record);
    const payload = { ...emptyPayload(), ...decrypted.payload };
    return markRaw({ ...base, payload, itemKeyRaw: decrypted.itemKeyRaw, search: searchText(payload) });
  } catch (error) {
    return markRaw({ ...base, payload: { ...emptyPayload(), name: "—" }, itemKeyRaw: null, search: "", error: String(error) });
  }
}

async function decryptFolder(record: FolderRecord): Promise<VaultFolder> {
  try {
    return { id: record.id, name: await mc.decryptString(key(), record.name) };
  } catch {
    return { id: record.id, name: "—" };
  }
}

async function decryptSend(record: SendRecord): Promise<VaultSend> {
  const sendKey = await mc.decryptBytes(key(), record.key);
  const { contentKey } = await mc.deriveSendKeys(sendKey);
  const dec = (value: string | null) => (value ? mc.decryptString(contentKey, value) : Promise.resolve(""));
  return {
    record,
    name: await dec(record.name),
    notes: await dec(record.notes),
    text: await dec(record.text),
    fileName: await dec(record.fileName),
    shareUrl: sendShareUrl(location.origin + location.pathname, record.accessId, sendKey)
  };
}

/** Replace changed entries and drop removed ones in one pass; returns a new array. */
function merge<T>(list: readonly T[], id: (value: T) => string, changed: readonly T[], removed: ReadonlySet<string> = new Set()): T[] {
  const index = new Map<string, number>();
  list.forEach((value, i) => index.set(id(value), i));
  const next = list.slice();
  for (const value of changed) {
    const i = index.get(id(value));
    if (i === undefined) {
      index.set(id(value), next.length);
      next.push(value);
    } else next[i] = value;
  }
  return removed.size ? next.filter((value) => !removed.has(id(value))) : next;
}

const itemId = (item: VaultItem) => item.id;
const folderId = (folder: VaultFolder) => folder.id;
const sendId = (send: VaultSend) => send.record.id;

function setItems(changed: readonly VaultItem[], removed?: ReadonlySet<string>) {
  vault.items = merge(toRaw(vault.items), itemId, changed, removed);
}

function sortFolders(folders: VaultFolder[]) {
  const collator = nameCollator.value;
  return folders.sort((a, b) => collator.compare(a.name, b.name));
}

function setFolders(changed: readonly VaultFolder[], removed?: ReadonlySet<string>) {
  vault.folders = sortFolders(merge(toRaw(vault.folders), folderId, changed, removed));
}

function setSends(changed: readonly VaultSend[], removed?: ReadonlySet<string>) {
  vault.sends = merge(toRaw(vault.sends), sendId, changed, removed);
}

/** Pull changes since the last sync (or everything on first run). */
export function syncVault(): Promise<void> {
  if (pending) return pending;
  const gen = generation;
  const id = ++syncRuns;
  pending = (async () => {
    vault.syncing = true;
    vault.error = "";
    try {
      const sync = await client.sync(vault.revisionDate || undefined);
      const [items, folders, sends] = await Promise.all([
        Promise.all(sync.items.map(decryptItem)),
        Promise.all(sync.folders.map(decryptFolder)),
        Promise.all(sync.sends.map((s) => decryptSend(s).catch(() => null)))
      ]);
      if (gen !== generation) return;
      const decryptedSends = sends.filter((s): s is VaultSend => s !== null);
      if (sync.full) {
        vault.items = items;
        vault.folders = sortFolders(folders);
        vault.sends = decryptedSends;
      } else {
        const gone = (type: string) => new Set(sync.deleted.filter((d) => d.type === type).map((d) => d.id));
        if (items.length || sync.deleted.length) setItems(items, gone("item"));
        if (folders.length || sync.deleted.length) setFolders(folders, gone("folder"));
        if (decryptedSends.length || sync.deleted.length) setSends(decryptedSends, gone("send"));
      }
      vault.revisionDate = sync.revisionDate;
      vault.lastSyncAt = new Date();
    } catch (error) {
      if (gen === generation) vault.error = error instanceof Error ? error.message : String(error);
      throw error;
    } finally {
      if (gen === generation) vault.syncing = false;
      // A lock may have started a newer sync already; leave that one in place.
      if (id === syncRuns) pending = null;
    }
  })();
  return pending;
}

/** Sync again after whatever sync is already running, so it sees our latest write. */
function syncAfterWrite() {
  const previous = pending ?? Promise.resolve();
  previous
    .catch(() => undefined)
    .then(() => syncVault())
    .catch(() => undefined);
}

/** Cheap check used by the background poll and window focus. */
export async function syncIfChanged() {
  if (!userKey.value || vault.syncing) return;
  const { revisionDate } = await client.revisionDate();
  if (revisionDate !== vault.revisionDate) await syncVault();
}

/** Record a write we made ourselves without waiting for the next sync. */
async function applyItem(record: ItemRecord) {
  const gen = generation;
  const item = await decryptItem(record);
  if (gen === generation) setItems([item]);
  return item;
}

// Items ---------------------------------------------------------------------

export async function saveItem(draft: ItemDraft, base?: VaultItem, force = false): Promise<VaultItem> {
  const encrypted = await mc.encryptItem(key(), draft.payload, base?.itemKeyRaw ?? undefined);
  const body = {
    type: draft.type,
    folderId: draft.folderId,
    favorite: draft.favorite,
    archived: draft.archived,
    key: encrypted.key,
    data: encrypted.data,
    lastKnownRevisionDate: force ? undefined : base?.revisionDate
  };
  try {
    const record = draft.id ? await client.updateItem(draft.id, body) : await client.createItem(body);
    return applyItem(record);
  } catch (error) {
    if (error instanceof ApiError && error.status === 409 && error.current) {
      const server = await applyItem(error.current as ItemRecord);
      throw new ConflictError(server);
    }
    throw error;
  }
}

export async function setFavorite(item: VaultItem, favorite: boolean) {
  return saveItem({ id: item.id, type: item.type, folderId: item.folderId, favorite, archived: item.archived, payload: item.payload }, item);
}

export async function setArchived(item: VaultItem, archived: boolean) {
  return saveItem({ id: item.id, type: item.type, folderId: item.folderId, favorite: item.favorite, archived, payload: item.payload }, item);
}

export async function moveItem(item: VaultItem, folderId: string | null) {
  return saveItem({ id: item.id, type: item.type, folderId, favorite: item.favorite, archived: item.archived, payload: item.payload }, item);
}

export async function trashItem(item: VaultItem) {
  return applyItem(await client.trashItem(item.id));
}

export async function restoreItem(item: VaultItem) {
  return applyItem(await client.restoreItem(item.id));
}

export async function deleteItem(item: VaultItem) {
  const gen = generation;
  await client.deleteItem(item.id);
  if (gen === generation) setItems([], new Set([item.id]));
}

export async function emptyTrash() {
  const gen = generation;
  const ids = vault.items.filter((i) => i.trashed).map((i) => i.id);
  for (let i = 0; i < ids.length; i += 1000) await client.bulk("delete", ids.slice(i, i + 1000));
  if (gen === generation) setItems([], new Set(ids));
}

// Attachments ---------------------------------------------------------------

async function itemKey(item: VaultItem) {
  if (!item.itemKeyRaw) throw new Error("Item key unavailable");
  return mc.importAesKey(item.itemKeyRaw);
}

export async function uploadAttachment(item: VaultItem, file: File) {
  const content = new Uint8Array(await file.arrayBuffer());
  return applyItem(await client.uploadAttachment(item.id, await itemKey(item), file.name, content));
}

export async function downloadAttachment(item: VaultItem, attachment: AttachmentRecord) {
  const { fileName, content } = await client.downloadAttachment(item.id, await itemKey(item), attachment);
  saveFile(fileName, content);
}

export async function deleteAttachment(item: VaultItem, attachment: AttachmentRecord) {
  return applyItem(await client.deleteAttachment(item.id, attachment.id));
}

export async function attachmentName(item: VaultItem, attachment: AttachmentRecord) {
  try {
    return await mc.decryptString(await itemKey(item), attachment.fileName);
  } catch {
    return "—";
  }
}

export function saveFile(fileName: string, content: Uint8Array<ArrayBuffer>) {
  const url = URL.createObjectURL(new Blob([content]));
  const link = document.createElement("a");
  link.href = url;
  link.download = fileName;
  link.rel = "noopener";
  // Firefox ignores clicks on links that are not in the document.
  document.body.append(link);
  link.click();
  link.remove();
  // Revoking right away can cancel a large download that has not started yet.
  setTimeout(() => URL.revokeObjectURL(url), 60_000);
}

// Folders -------------------------------------------------------------------

export async function createFolder(name: string) {
  const gen = generation;
  const record = await client.createFolder(await mc.encryptString(key(), name));
  const folder = await decryptFolder(record);
  if (gen === generation) setFolders([folder]);
  return record.id;
}

export async function renameFolder(id: string, name: string) {
  const gen = generation;
  const record = await client.updateFolder(id, await mc.encryptString(key(), name));
  const folder = await decryptFolder(record);
  if (gen === generation) setFolders([folder]);
}

export async function deleteFolder(id: string) {
  const gen = generation;
  await client.deleteFolder(id);
  if (gen !== generation) return;
  setFolders([], new Set([id]));
  // The server moved the folder's items out with a new revision. Show that now, and fetch the
  // new revisions so editing one of those items next is not reported as a conflict.
  setItems(toRaw(vault.items).filter((i) => i.folderId === id).map((i) => markRaw({ ...i, folderId: null })));
  syncAfterWrite();
}

// Send ----------------------------------------------------------------------

export interface SendDraft {
  name: string;
  notes: string;
  text: string;
  file: File | null;
  password: string;
  maxAccessCount: number | null;
  deletionDays: number;
  expirationDays: number | null;
  hideEmail: boolean;
}

export async function createSend(draft: SendDraft): Promise<VaultSend> {
  const gen = generation;
  const { sendKey, contentKey } = await mc.deriveSendKeys();
  const now = Date.now();
  const body: SendWrite = {
    type: draft.file ? 1 : 0,
    key: await mc.encryptBytes(key(), sendKey),
    name: await mc.encryptString(contentKey, draft.name || draft.file?.name || "Send"),
    notes: draft.notes ? await mc.encryptString(contentKey, draft.notes) : null,
    passwordHash: draft.password ? await mc.sendPasswordHash(draft.password, sendKey) : null,
    maxAccessCount: draft.maxAccessCount,
    deletionDate: new Date(now + draft.deletionDays * 86_400_000).toISOString(),
    expirationDate: draft.expirationDays ? new Date(now + draft.expirationDays * 86_400_000).toISOString() : null,
    hideEmail: draft.hideEmail
  };
  let encrypted: Uint8Array<ArrayBuffer> | null = null;
  if (draft.file) {
    encrypted = await mc.encryptBlob(contentKey, new Uint8Array(await draft.file.arrayBuffer()));
    body.fileName = await mc.encryptString(contentKey, draft.file.name);
    body.size = encrypted.length;
  } else {
    body.text = await mc.encryptString(contentKey, draft.text);
  }
  let record: SendRecord = await client.createSend(body);
  if (encrypted) record = await client.uploadSendFile(record.id, encrypted);
  const send = await decryptSend(record);
  if (gen === generation) setSends([send]);
  return send;
}

export async function setSendDisabled(send: VaultSend, disabled: boolean) {
  const gen = generation;
  const r = send.record;
  const record = await client.updateSend(r.id, {
    type: r.type,
    key: r.key,
    name: r.name,
    notes: r.notes,
    text: r.text,
    fileName: r.fileName,
    size: r.size,
    maxAccessCount: r.maxAccessCount,
    expirationDate: r.expirationDate,
    deletionDate: r.deletionDate,
    hideEmail: r.hideEmail,
    disabled
  });
  const updated = await decryptSend(record);
  if (gen === generation) setSends([updated]);
}

export async function deleteSend(send: VaultSend) {
  const gen = generation;
  await client.deleteSend(send.record.id);
  if (gen === generation) setSends([], new Set([send.record.id]));
}
