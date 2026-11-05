<script setup lang="ts">
import { computed, reactive, ref, watch } from "vue";
import { t } from "../i18n";
import type { AttachmentRecord } from "../lib/client";
import { encryptedBlobSize } from "../lib/crypto";
import { formatBytes, formatDateTime } from "../lib/format";
import { type FieldDescriptor, getPath, hostOf, isFieldVisible, optionLabel, typeFields, typeInfo } from "../lib/items";
import { serverConfig } from "../lib/session";
import { confirm, copy, toast, toastError } from "../lib/ui";
import {
  attachmentName,
  deleteAttachment,
  deleteItem,
  downloadAttachment,
  moveItem,
  restoreItem,
  setArchived,
  setFavorite,
  trashItem,
  uploadAttachment,
  vault,
  type VaultItem
} from "../lib/vault";
import ItemIcon from "./ItemIcon.vue";
import TotpCode from "./TotpCode.vue";

const props = defineProps<{ item: VaultItem }>();
const emit = defineEmits<{ edit: []; gone: []; back: [] }>();

const info = computed(() => typeInfo(props.item.type));
const revealed = reactive(new Set<string>());
const busy = ref(false);
const uploading = ref(false);
const fileInput = ref<HTMLInputElement | null>(null);
const names = reactive(new Map<string, string>());

const fields = computed(() =>
  typeFields[info.value.key]
    .filter((field) => isFieldVisible(field, props.item.payload))
    .map((field) => ({ field, value: getPath(props.item.payload, field.path) }))
    .filter(({ field, value }) => value !== undefined && value !== null && value !== "" && value !== false && value !== field.defaultValue)
);
const history = computed(() => (props.item.payload.passwordHistory ?? []).filter((entry) => entry?.password));
const showHistory = ref(false);
const folderId = computed({
  get: () => props.item.folderId ?? "",
  set: (value: string) => move(value || null)
});

// Attachment names are encrypted with the item key; decrypt the ones not seen yet whenever
// the item changes (e.g. after an upload).
watch(
  () => props.item,
  async (item) => {
    for (const attachment of item.attachments) {
      if (!names.has(attachment.id)) names.set(attachment.id, await attachmentName(item, attachment));
    }
  },
  { immediate: true }
);

function display(field: FieldDescriptor, value: unknown): string {
  if (field.kind === "boolean") return value ? t("common.yes") : t("common.no");
  if (field.kind === "select") return optionLabel(field, String(value));
  return String(value);
}

function isSecret(field: FieldDescriptor) {
  return field.kind === "secret" || field.kind === "secret-multiline";
}

function toggle(path: string) {
  if (revealed.has(path)) revealed.delete(path);
  else revealed.add(path);
}

/** Runs an action with the buttons disabled; resolves to whether it succeeded. */
async function run(action: () => Promise<unknown>, message?: string): Promise<boolean> {
  busy.value = true;
  try {
    await action();
    if (message) toast(message);
    return true;
  } catch (e) {
    toastError(e);
    return false;
  } finally {
    busy.value = false;
  }
}

function move(target: string | null) {
  const item = props.item;
  return run(() => moveItem(item, target));
}

async function trash() {
  await run(() => trashItem(props.item), t("item.trashed"));
}

async function remove() {
  if (!(await confirm(t("item.deleteTitle"), t("item.deleteBody", { name: props.item.payload.name }), { danger: true, confirmLabel: t("item.deleteForever") }))) return;
  if (await run(() => deleteItem(props.item), t("item.deleted"))) emit("gone");
}

async function onFile(event: Event) {
  const file = (event.target as HTMLInputElement).files?.[0];
  (event.target as HTMLInputElement).value = "";
  if (!file) return;
  // Check the limit before reading and encrypting a file the server will refuse anyway.
  const limit = serverConfig.value?.attachmentMaxBytes;
  if (limit && encryptedBlobSize(file.size) > limit) {
    toast(t("attachment.tooLarge", { size: formatBytes(limit) }), "error");
    return;
  }
  uploading.value = true;
  await run(() => uploadAttachment(props.item, file), t("attachment.uploaded"));
  uploading.value = false;
}

async function removeAttachment(attachment: AttachmentRecord) {
  const name = names.get(attachment.id) ?? "";
  if (!(await confirm(t("attachment.deleteTitle"), t("attachment.deleteBody", { name }), { danger: true, confirmLabel: t("common.delete") }))) return;
  await run(() => deleteAttachment(props.item, attachment));
}

function openUri(uri: string) {
  const url = uri.includes("://") ? uri : `https://${uri}`;
  window.open(url, "_blank", "noopener,noreferrer");
}
</script>

<template>
  <article class="detail">
    <header class="detail-header">
      <m3e-icon-button class="back-button" :aria-label="t('common.back')" @click="emit('back')"><m3e-icon name="arrow_back"></m3e-icon></m3e-icon-button>
      <ItemIcon :type="item.type" :name="item.payload.name" size="large" />
      <div class="detail-title">
        <h2>{{ item.payload.name || t("item.untitled") }}</h2>
        <div class="detail-meta">
          <span class="chip"><m3e-icon :name="info.icon"></m3e-icon>{{ t(info.labelKey) }}</span>
          <label v-if="!item.trashed" class="chip select-chip">
            <m3e-icon name="folder"></m3e-icon>
            <select v-model="folderId" :aria-label="t('item.folder')">
              <option value="">{{ t("item.noFolder") }}</option>
              <option v-for="folder in vault.folders" :key="folder.id" :value="folder.id">{{ folder.name }}</option>
            </select>
          </label>
          <span v-if="item.archived" class="chip tonal"><m3e-icon name="archive"></m3e-icon>{{ t("nav.archive") }}</span>
        </div>
      </div>
      <div class="detail-actions">
        <template v-if="!item.trashed">
          <m3e-icon-button
            :aria-label="item.favorite ? t('item.unfavorite') : t('item.favorite')"
            :title="item.favorite ? t('item.unfavorite') : t('item.favorite')"
            :disabled="busy || item.error"
            @click="run(() => setFavorite(item, !item.favorite))"
          >
            <m3e-icon name="star" :filled="item.favorite" :class="{ star: item.favorite }"></m3e-icon>
          </m3e-icon-button>
          <m3e-icon-button
            :aria-label="item.archived ? t('item.unarchive') : t('item.archive')"
            :title="item.archived ? t('item.unarchive') : t('item.archive')"
            :disabled="busy || item.error"
            @click="run(() => setArchived(item, !item.archived), item.archived ? t('item.unarchived') : t('item.archived'))"
          >
            <m3e-icon :name="item.archived ? 'unarchive' : 'archive'"></m3e-icon>
          </m3e-icon-button>
          <m3e-icon-button :aria-label="t('item.trash')" :title="t('item.trash')" :disabled="busy" @click="trash"><m3e-icon name="delete"></m3e-icon></m3e-icon-button>
          <m3e-button variant="tonal" :disabled="busy || item.error" @click="emit('edit')"><m3e-icon slot="icon" name="edit"></m3e-icon>{{ t("common.edit") }}</m3e-button>
        </template>
        <template v-else>
          <m3e-button variant="text" :disabled="busy" @click="remove">{{ t("item.deleteForever") }}</m3e-button>
          <m3e-button variant="tonal" :disabled="busy" @click="run(() => restoreItem(item), t('item.restored'))"><m3e-icon slot="icon" name="restore_from_trash"></m3e-icon>{{ t("item.restore") }}</m3e-button>
        </template>
      </div>
    </header>

    <div class="detail-body">
      <div v-if="item.error" class="callout error">
        <m3e-icon name="lock_reset"></m3e-icon>
        <div><strong>{{ t("item.decryptFailed") }}</strong><p>{{ t("item.decryptFailedHint") }}</p></div>
      </div>

      <section v-if="fields.length" class="field-group">
        <div v-for="{ field, value } in fields" :key="field.path" class="field-row" :class="{ multiline: field.kind.includes('multiline') }">
          <span class="field-label">{{ t(field.labelKey) }}</span>
          <TotpCode
            v-if="field.kind === 'totp' && field.path === 'totp.secret'"
            :secret="String(value)"
            :digits="item.payload.totp?.digits"
            :period="item.payload.totp?.period"
            :algorithm="item.payload.totp?.algorithm"
            :otp-type="item.payload.totp?.otpType"
          />
          <!-- login.totp holds a secret or URI of its own; the authenticator settings belong to type 5 items. -->
          <TotpCode v-else-if="field.kind === 'totp'" :secret="String(value)" />
          <template v-else>
            <span class="field-value" :class="{ mono: field.mono || isSecret(field) }">
              <template v-if="isSecret(field) && !revealed.has(field.path)">••••••••••••</template>
              <template v-else>{{ display(field, value) }}</template>
            </span>
            <span class="field-tools">
              <m3e-icon-button v-if="isSecret(field)" :aria-label="t(revealed.has(field.path) ? 'common.hide' : 'common.show')" @click="toggle(field.path)">
                <m3e-icon :name="revealed.has(field.path) ? 'visibility_off' : 'visibility'"></m3e-icon>
              </m3e-icon-button>
              <m3e-icon-button v-if="field.kind !== 'boolean'" :aria-label="t('common.copy')" @click="copy(String(value))"><m3e-icon name="content_copy"></m3e-icon></m3e-icon-button>
            </span>
          </template>
        </div>
      </section>

      <section v-if="item.payload.uris?.length" class="field-group">
        <div v-for="(uri, index) in item.payload.uris" :key="index" class="field-row">
          <span class="field-label">{{ t("item.website") }}</span>
          <span class="field-value link" :title="uri">{{ hostOf(uri) || uri }}</span>
          <span class="field-tools">
            <m3e-icon-button :aria-label="t('common.open')" @click="openUri(uri)"><m3e-icon name="open_in_new"></m3e-icon></m3e-icon-button>
            <m3e-icon-button :aria-label="t('common.copy')" @click="copy(uri)"><m3e-icon name="content_copy"></m3e-icon></m3e-icon-button>
          </span>
        </div>
      </section>

      <section v-if="item.payload.fields?.length" class="field-group">
        <h3 class="group-title">{{ t("item.customFields") }}</h3>
        <div v-for="(custom, index) in item.payload.fields" :key="index" class="field-row">
          <span class="field-label">{{ custom.name }}</span>
          <span class="field-value" :class="{ mono: custom.hidden }">
            {{ custom.hidden && !revealed.has(`custom.${index}`) ? "••••••••" : custom.value }}
          </span>
          <span class="field-tools">
            <m3e-icon-button v-if="custom.hidden" :aria-label="t('common.show')" @click="toggle(`custom.${index}`)">
              <m3e-icon :name="revealed.has(`custom.${index}`) ? 'visibility_off' : 'visibility'"></m3e-icon>
            </m3e-icon-button>
            <m3e-icon-button :aria-label="t('common.copy')" @click="copy(custom.value)"><m3e-icon name="content_copy"></m3e-icon></m3e-icon-button>
          </span>
        </div>
      </section>

      <section v-if="item.payload.notes" class="field-group">
        <h3 class="group-title">{{ t("item.notes") }}</h3>
        <p class="notes">{{ item.payload.notes }}</p>
      </section>

      <section class="field-group">
        <div class="group-head">
          <h3 class="group-title">{{ t("item.attachments") }}</h3>
          <m3e-button v-if="!item.trashed && !item.error" variant="text" :disabled="uploading" @click="fileInput?.click()">
            <m3e-icon slot="icon" :name="uploading ? 'hourglass_top' : 'upload_file'"></m3e-icon>{{ uploading ? t("attachment.encrypting") : t("attachment.add") }}
          </m3e-button>
          <input ref="fileInput" type="file" hidden @change="onFile" />
        </div>
        <p v-if="!item.attachments.length" class="muted small">{{ t("attachment.none") }}</p>
        <div v-for="attachment in item.attachments" :key="attachment.id" class="attachment-row">
          <m3e-icon name="description"></m3e-icon>
          <div>
            <strong>{{ names.get(attachment.id) ?? "…" }}</strong>
            <small>{{ formatBytes(attachment.size) }}</small>
          </div>
          <m3e-icon-button :aria-label="t('common.download')" @click="run(() => downloadAttachment(item, attachment))"><m3e-icon name="download"></m3e-icon></m3e-icon-button>
          <m3e-icon-button v-if="!item.trashed" :aria-label="t('common.delete')" @click="removeAttachment(attachment)"><m3e-icon name="delete"></m3e-icon></m3e-icon-button>
        </div>
      </section>

      <section v-if="history.length" class="field-group">
        <button type="button" class="group-toggle" :aria-expanded="showHistory" @click="showHistory = !showHistory">
          <span class="group-title">{{ t("item.passwordHistory", { count: history.length }) }}</span>
          <m3e-icon :name="showHistory ? 'expand_less' : 'expand_more'"></m3e-icon>
        </button>
        <template v-if="showHistory">
          <div v-for="(entry, index) in history" :key="index" class="field-row">
            <span class="field-label">{{ t("item.usedUntil", { time: formatDateTime(entry.lastUsedAt) }) }}</span>
            <span class="field-value mono">{{ revealed.has(`history.${index}`) ? entry.password : "••••••••••••" }}</span>
            <span class="field-tools">
              <m3e-icon-button :aria-label="t(revealed.has(`history.${index}`) ? 'common.hide' : 'common.show')" @click="toggle(`history.${index}`)">
                <m3e-icon :name="revealed.has(`history.${index}`) ? 'visibility_off' : 'visibility'"></m3e-icon>
              </m3e-icon-button>
              <m3e-icon-button :aria-label="t('common.copy')" @click="copy(entry.password)"><m3e-icon name="content_copy"></m3e-icon></m3e-icon-button>
            </span>
          </div>
        </template>
      </section>

      <footer class="detail-footer">
        <span>{{ t("item.created") }} {{ formatDateTime(item.creationDate) }}</span>
        <span>{{ t("item.updated") }} {{ formatDateTime(item.revisionDate) }}</span>
      </footer>
    </div>
  </article>
</template>
