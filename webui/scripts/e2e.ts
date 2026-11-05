/**
 * End-to-end test of the Monica Cloud protocol against a running server, using
 * the same crypto and client code as the WebUI.
 *
 *   MONICA_URL=http://127.0.0.1:8000 node scripts/e2e.ts
 *
 * Use a throw-away server: the test registers accounts and deletes data.
 */

import assert from "node:assert/strict";
import * as mc from "../src/lib/crypto.ts";
import { ApiError, MonicaClient, encryptSendText } from "../src/lib/client.ts";

const baseUrl = process.env.MONICA_URL ?? "http://127.0.0.1:8000";
const ITERATIONS = 100_000; // the server minimum keeps the test fast
const run = Date.now().toString(36);
const ownerEmail = `owner-${run}@example.com`;
const memberEmail = `member-${run}@example.com`;
const password = "correct horse battery staple";
const device = (name: string) => ({ identifier: `e2e-${name}-${run}`, name, type: "cli" as const, appVersion: "e2e" });

let passed = 0;
async function step(name: string, fn: () => Promise<void>) {
  try {
    await fn();
    passed++;
    console.log(`  ok  ${name}`);
  } catch (error) {
    console.error(`  FAIL ${name}`);
    throw error;
  }
}

async function expectError(promise: Promise<unknown>, status: number, code?: string) {
  try {
    await promise;
  } catch (error) {
    assert.ok(error instanceof ApiError, `expected ApiError, got ${error}`);
    assert.equal(error.status, status, `${error.code}: ${error.message}`);
    if (code) assert.equal(error.code, code);
    return error;
  }
  assert.fail(`expected HTTP ${status}`);
}

console.log(`Monica Cloud e2e against ${baseUrl}`);

// Crypto self-checks ---------------------------------------------------------
await step("crypto: chunked blob round trip and tamper detection", async () => {
  const key = await mc.importAesKey(mc.randomBytes(32));
  for (const size of [0, 1, 1024, 65536, 65537, 200_000]) {
    const plain = mc.randomBytes(size);
    const sealed = await mc.encryptBlob(key, plain);
    assert.equal(sealed.length, mc.encryptedBlobSize(size));
    assert.deepEqual(await mc.decryptBlob(key, sealed), plain);
  }
  const sealed = await mc.encryptBlob(key, mc.randomBytes(140_000));
  // Dropping the final chunk must be detected (truncation).
  await assert.rejects(mc.decryptBlob(key, sealed.subarray(0, 8 + 2 * (12 + 65536 + 16))));
  sealed[100] ^= 1;
  await assert.rejects(mc.decryptBlob(key, sealed));
});

await step("crypto: KDF is deterministic and email-normalised", async () => {
  const a = await mc.deriveMasterKeys(" Me@Example.com ", password, ITERATIONS);
  const b = await mc.deriveMasterKeys("me@example.com", password, ITERATIONS);
  assert.equal(a.authHash, b.authHash);
  assert.equal(mc.b64decode(a.authHash).length, 32);
});

// Accounts -------------------------------------------------------------------
const owner = new MonicaClient(baseUrl);
const member = new MonicaClient(baseUrl);
let ownerKey: mc.UserKey;

await step("config is public", async () => {
  const config = await owner.config();
  assert.equal(config.protocolVersion, 1);
  assert.equal(config.kdf, 0);
});

await step("register owner and member", async () => {
  await owner.register(ownerEmail, "Owner", password, ITERATIONS);
  await member.register(memberEmail, "Member", password, ITERATIONS);
  await expectError(owner.register(ownerEmail, "Again", password, ITERATIONS), 400);
});

await step("prelogin returns stored KDF and defaults for unknown users", async () => {
  assert.equal((await owner.prelogin(ownerEmail)).kdfIterations, ITERATIONS);
  const unknown = await owner.prelogin(`nobody-${run}@example.com`);
  assert.equal(unknown.kdf, 0);
});

await step("wrong master password is rejected", async () => {
  await expectError(new MonicaClient(baseUrl).login(ownerEmail, "wrong", device("bad")), 401);
});

await step("login unwraps the account key", async () => {
  ownerKey = await owner.login(ownerEmail, password, device("owner"));
  assert.equal(ownerKey.raw.length, 32);
  await member.login(memberEmail, password, device("member"));
  // Unlocking locally must produce the same key.
  assert.deepEqual((await owner.unlock(password)).raw, ownerKey.raw);
});

// Vault ----------------------------------------------------------------------
let folderId = "";
let itemId = "";
let itemRevision = "";
let itemKeyRaw: Uint8Array<ArrayBuffer>;
let cursor = "";

await step("full sync of an empty vault", async () => {
  const sync = await owner.sync();
  assert.equal(sync.full, true);
  assert.equal(sync.items.length, 0);
  cursor = sync.revisionDate;
});

await step("create folder and item; server only sees ciphertext", async () => {
  const folder = await owner.createFolder(await mc.encryptString(ownerKey.key, "开发"));
  folderId = folder.id;
  const payload = { v: 1, name: "GitHub", login: { username: "octocat", password: "hunter2" }, uris: ["https://github.com"] };
  const enc = await mc.encryptItem(ownerKey.key, payload);
  itemKeyRaw = enc.itemKeyRaw;
  const item = await owner.createItem({ type: 1, folderId, favorite: true, key: enc.key, data: enc.data, archived: false });
  itemId = item.id;
  itemRevision = item.revisionDate;
  assert.ok(!item.data.includes("hunter2"));
  const roundTrip = await mc.decryptItem<typeof payload>(ownerKey.key, item);
  assert.equal(roundTrip.payload.login.password, "hunter2");
});

await step("plaintext is rejected by the server", async () => {
  await expectError(owner.createItem({ type: 1, folderId: null, favorite: false, key: "plain", data: "plain", archived: false }), 400);
});

await step("incremental sync returns only changes", async () => {
  const sync = await owner.sync(cursor);
  assert.equal(sync.full, false);
  assert.equal(sync.items.length, 1);
  assert.equal(sync.folders.length, 1);
  assert.equal(await mc.decryptString(ownerKey.key, sync.folders[0].name), "开发");
  cursor = sync.revisionDate;
  const again = await owner.sync(cursor);
  assert.equal(again.items.length + again.folders.length + again.deleted.length, 0);
});

await step("users cannot see each other's items", async () => {
  const sync = await member.sync();
  assert.equal(sync.items.length, 0);
  await expectError(member.trashItem(itemId), 404);
});

await step("optimistic concurrency: stale update gets 409 with current", async () => {
  const update = async (name: string, lastKnownRevisionDate: string) => {
    const enc = await mc.encryptItem(ownerKey.key, { v: 1, name }, itemKeyRaw);
    return owner.updateItem(itemId, { type: 1, folderId, favorite: true, key: enc.key, data: enc.data, archived: false, lastKnownRevisionDate });
  };
  const first = await update("GitHub (device A)", itemRevision);
  const conflict = await expectError(update("GitHub (device B)", itemRevision), 409, "conflict");
  const current = conflict.current as { revisionDate: string };
  assert.equal(current.revisionDate, first.revisionDate);
  itemRevision = first.revisionDate;
});

let attachmentId = "";
await step("attachment upload, download and decrypt", async () => {
  const content = mc.utf8("恢复码: 1234-5678\n".repeat(5000));
  const itemKey = await mc.importAesKey(itemKeyRaw);
  const item = await owner.uploadAttachment(itemId, itemKey, "recovery.txt", content);
  assert.equal(item.attachments.length, 1);
  attachmentId = item.attachments[0].id;
  const download = await owner.downloadAttachment(itemId, itemKey, item.attachments[0]);
  assert.equal(download.fileName, "recovery.txt");
  assert.deepEqual(download.content, content);
});

await step("trash, restore, permanent delete and tombstones", async () => {
  const trashed = await owner.trashItem(itemId);
  assert.ok(trashed.deletedDate);
  const restored = await owner.restoreItem(itemId);
  assert.equal(restored.deletedDate, null);
  await owner.deleteItem(itemId);
  const sync = await owner.sync(cursor);
  assert.deepEqual(sync.deleted.map((d) => [d.type, d.id]), [["item", itemId]]);
  await expectError(owner.downloadAttachment(itemId, await mc.importAesKey(itemKeyRaw), { id: attachmentId } as never), 404);
  cursor = sync.revisionDate;
});

await step("bulk import maps folder refs", async () => {
  const name = await mc.encryptString(ownerKey.key, "导入");
  const items = await Promise.all(
    [1, 2, 3].map(async (n) => {
      const enc = await mc.encryptItem(ownerKey.key, { v: 1, name: `Item ${n}` });
      return { type: n === 3 ? 2 : 1, folderId: null, favorite: false, key: enc.key, data: enc.data, archived: false, ref: `local-${n}`, folderRef: "f1" };
    })
  );
  const result = await owner.importItems([{ ref: "f1", name }], items);
  assert.equal(result.items.length, 3);
  assert.deepEqual(result.items.map((i) => i.ref), ["local-1", "local-2", "local-3"]);
  assert.ok(result.items.every((i) => i.folderId === result.folders[0].folder.id));
  const stats = await owner.stats();
  assert.equal(stats.items, 3);
  const moved = await owner.bulk("move", result.items.map((i) => i.id), folderId);
  assert.equal(moved.affected, 3);
  await owner.deleteFolder(folderId);
  const sync = await owner.sync(cursor);
  assert.ok(sync.items.every((i) => i.folderId === null));
  assert.ok(sync.deleted.some((d) => d.type === "folder" && d.id === folderId));
});

// Send -----------------------------------------------------------------------
await step("text Send with password and access limit", async () => {
  const { keys, fields } = await encryptSendText(ownerKey.key, "Wi-Fi", "password: 12345678", "for the guest");
  const deletionDate = new Date(Date.now() + 86_400_000).toISOString();
  const send = await owner.createSend({
    type: 0,
    ...fields,
    passwordHash: await mc.sendPasswordHash("open sesame", keys.sendKey),
    maxAccessCount: 1,
    deletionDate
  });
  const anonymous = new MonicaClient(baseUrl);
  await expectError(anonymous.accessSend(send.accessId), 401);
  await expectError(anonymous.accessSend(send.accessId, await mc.sendPasswordHash("wrong", keys.sendKey)), 401);
  const opened = await anonymous.accessSend(send.accessId, await mc.sendPasswordHash("open sesame", keys.sendKey));
  // The recipient derives the content key from the link fragment alone.
  const recipientKeys = await mc.deriveSendKeys(mc.b64urlDecode(mc.b64urlEncode(keys.sendKey)));
  assert.equal(await mc.decryptString(recipientKeys.contentKey, opened.text!), "password: 12345678");
  assert.equal(opened.creatorIdentifier, ownerEmail);
  assert.equal((opened as Record<string, unknown>).notes, undefined, "owner notes must not leak");
  await expectError(anonymous.accessSend(send.accessId, await mc.sendPasswordHash("open sesame", keys.sendKey)), 404);
});

await step("file Send upload and anonymous download", async () => {
  const keys = await mc.deriveSendKeys();
  const content = mc.randomBytes(100_000);
  const encrypted = await mc.encryptBlob(keys.contentKey, content);
  const send = await owner.createSend({
    type: 1,
    key: await mc.encryptBytes(ownerKey.key, keys.sendKey),
    name: await mc.encryptString(keys.contentKey, "photo"),
    fileName: await mc.encryptString(keys.contentKey, "photo.jpg"),
    size: encrypted.length,
    hideEmail: true,
    deletionDate: new Date(Date.now() + 3_600_000).toISOString()
  });
  const anonymous = new MonicaClient(baseUrl);
  await expectError(anonymous.accessSend(send.accessId), 404); // not uploaded yet
  await owner.uploadSendFile(send.id, encrypted);
  const opened = await anonymous.accessSend(send.accessId);
  assert.equal(opened.creatorIdentifier, null);
  const blob = await anonymous.downloadSendFile(send.accessId, opened.downloadToken!);
  assert.deepEqual(await mc.decryptBlob(keys.contentKey, blob), content);
  const sends = await owner.sends();
  const stored = sends.find((s) => s.id === send.id)!;
  assert.equal(stored.accessCount, 1);
  // Disabling a file Send re-submits its metadata unchanged.
  await owner.updateSend(send.id, { ...stored, notes: stored.notes, disabled: true });
  await expectError(anonymous.accessSend(send.accessId), 404);
});

// Sessions -------------------------------------------------------------------
await step("refresh tokens rotate and old ones die", async () => {
  const before = owner.session!.refreshToken;
  await owner.refresh();
  assert.notEqual(owner.session!.refreshToken, before);
  const stale = new MonicaClient(baseUrl);
  stale.session = { ...owner.session!, refreshToken: before };
  await expectError(stale.refresh(), 401);
});

await step("device list and revoke", async () => {
  const second = new MonicaClient(baseUrl);
  await second.login(ownerEmail, password, device("second"));
  const devices = await owner.devices();
  assert.equal(devices.length, 2);
  const other = devices.find((d) => !d.current)!;
  await owner.revokeDevice(other.id);
  second.session!.refreshToken = "x"; // force: access token must already be dead
  await expectError(second.sync(), 401);
});

// Admin ----------------------------------------------------------------------
const ownerIsAdmin = owner.session!.account.role !== "user";
await step("admin endpoints are role-protected", async () => {
  await expectError(member.adminUsers(), 403);
  if (!ownerIsAdmin) return; // server already had an owner before this run
  const users = await owner.adminUsers();
  assert.ok(users.some((u) => u.email === memberEmail));
  const diagnostics = await owner.adminDiagnostics();
  assert.ok(diagnostics.counts.users >= 2);
  const settings = await owner.adminSettings();
  await owner.adminSaveSettings({ ...settings, eventRetentionDays: 30 });
  assert.equal((await owner.adminSettings()).eventRetentionDays, 30);
  await owner.adminSaveSettings(settings);
});

await step("admin can disable a member, which kills their session", async () => {
  if (!ownerIsAdmin) return;
  const target = (await owner.adminUsers()).find((u) => u.email === memberEmail)!;
  await owner.adminUpdateUser(target.id, { enabled: false, quotaBytes: 1024 });
  await expectError(member.sync(), 401);
  await expectError(new MonicaClient(baseUrl).login(memberEmail, password, device("member")), 403);
  await owner.adminUpdateUser(target.id, { enabled: true });
});

await step("admins cannot sign out, disable or delete an owner", async () => {
  if (!ownerIsAdmin) return;
  const target = (await owner.adminUsers()).find((u) => u.email === memberEmail)!;
  await owner.adminUpdateUser(target.id, { role: "admin" });
  const admin = new MonicaClient(baseUrl);
  await admin.login(memberEmail, password, device("member-admin"));
  const ownerId = owner.session!.account.id;
  await expectError(admin.adminDeauthUser(ownerId), 403);
  await expectError(admin.adminUpdateUser(ownerId, { enabled: false }), 403);
  await expectError(admin.adminDeleteUser(ownerId), 403);
  await owner.sync(); // the owner's session survived
  await owner.adminUpdateUser(target.id, { role: "user" });
});

await step("audit log records actions", async () => {
  const events = await owner.events(50);
  const actions = new Set(events.map((e) => e.action));
  for (const action of ["auth.login", "item.create", "item.update", "send.create"]) assert.ok(actions.has(action), action);
});

// Password change ------------------------------------------------------------
await step("password change re-wraps the same account key", async () => {
  await owner.changePassword(password, "a brand new passphrase", ownerKey);
  await expectError(new MonicaClient(baseUrl).login(ownerEmail, password, device("old")), 401);
  const fresh = new MonicaClient(baseUrl);
  const key = await fresh.login(ownerEmail, "a brand new passphrase", device("owner"));
  assert.deepEqual(key.raw, ownerKey.raw);
  const sync = await fresh.sync();
  const first = sync.items[0];
  assert.ok((await mc.decryptItem<{ name: string }>(key.key, first)).payload.name.startsWith("Item"));
  await fresh.deleteAccount("a brand new passphrase").catch((e) => {
    // The first owner cannot delete itself while other users exist.
    if (!(e instanceof ApiError && e.status === 403)) throw e;
  });
});

console.log(`\n${passed} checks passed`);
