<script setup lang="ts">
import "@m3e/web/dialog";
import { computed, reactive, ref } from "vue";
import PasswordInput from "../components/PasswordInput.vue";
import { t } from "../i18n";
import { encryptedBlobSize } from "../lib/crypto";
import { formatBytes, formatRelative } from "../lib/format";
import { serverConfig } from "../lib/session";
import { confirm, copy, toastError } from "../lib/ui";
import { createSend, deleteSend, setSendDisabled, vault, type SendDraft, type VaultSend } from "../lib/vault";

const creating = ref(false);
const saving = ref(false);
const created = ref<VaultSend | null>(null);
const kind = ref<"text" | "file">("text");
const draft = reactive<SendDraft>(blank());
const fileInput = ref<HTMLInputElement | null>(null);
const formError = ref("");

function blank(): SendDraft {
  return { name: "", notes: "", text: "", file: null, password: "", maxAccessCount: null, deletionDays: 7, expirationDays: null, hideEmail: false };
}

const sends = computed(() => [...vault.sends].sort((a, b) => b.record.creationDate.localeCompare(a.record.creationDate)));

function status(send: VaultSend): { label: string; tone: string } {
  const r = send.record;
  if (r.disabled) return { label: t("send.statusDisabled"), tone: "muted" };
  if (r.expirationDate && new Date(r.expirationDate) < new Date()) return { label: t("send.statusExpired"), tone: "muted" };
  if (r.maxAccessCount !== null && r.accessCount >= r.maxAccessCount) return { label: t("send.statusUsedUp"), tone: "muted" };
  if (r.type === 1 && !r.uploaded) return { label: t("send.statusPending"), tone: "warn" };
  return { label: t("send.statusActive"), tone: "ok" };
}

function open() {
  Object.assign(draft, blank());
  kind.value = "text";
  created.value = null;
  formError.value = "";
  creating.value = true;
}

function onFile(event: Event) {
  draft.file = (event.target as HTMLInputElement).files?.[0] ?? null;
  if (draft.file && !draft.name) draft.name = draft.file.name;
  formError.value = "";
}

/** `v-model.number` yields "" for a cleared field; the server only accepts a positive integer or null. */
function maxOpens(): number | null | undefined {
  const raw = draft.maxAccessCount as number | string | null;
  if (raw === null || raw === "") return null;
  const value = Number(raw);
  return Number.isInteger(value) && value >= 1 ? value : undefined;
}

async function submit() {
  if (saving.value) return;
  formError.value = "";
  if (kind.value === "text" && !draft.text.trim()) return (formError.value = t("send.textRequired"));
  if (kind.value === "file" && !draft.file) return (formError.value = t("send.fileRequired"));
  const limit = serverConfig.value?.attachmentMaxBytes;
  if (kind.value === "file" && draft.file && limit && encryptedBlobSize(draft.file.size) > limit) {
    return (formError.value = t("attachment.tooLarge", { size: formatBytes(limit) }));
  }
  const maxAccessCount = maxOpens();
  if (maxAccessCount === undefined) return (formError.value = t("send.maxAccessInvalid"));
  saving.value = true;
  try {
    created.value = await createSend({ ...draft, file: kind.value === "file" ? draft.file : null, maxAccessCount });
  } catch (e) {
    toastError(e);
  } finally {
    saving.value = false;
  }
}

async function remove(send: VaultSend) {
  if (!(await confirm(t("send.deleteTitle"), t("send.deleteBody", { name: send.name }), { danger: true, confirmLabel: t("common.delete") }))) return;
  try {
    await deleteSend(send);
  } catch (e) {
    toastError(e);
  }
}

async function toggle(send: VaultSend) {
  try {
    await setSendDisabled(send, !send.record.disabled);
  } catch (e) {
    toastError(e);
  }
}
</script>

<template>
  <div class="page">
    <header class="page-header">
      <div>
        <h1>{{ t("nav.send") }}</h1>
        <p class="muted">{{ t("send.subtitle") }}</p>
      </div>
      <m3e-button variant="filled" :disabled="serverConfig?.sendsAllowed === false" @click="open"><m3e-icon slot="icon" name="add"></m3e-icon>{{ t("send.new") }}</m3e-button>
    </header>

    <p v-if="serverConfig?.sendsAllowed === false" class="callout"><m3e-icon name="block"></m3e-icon>{{ t("send.disabledByAdmin") }}</p>

    <div v-if="sends.length" class="card-list">
      <article v-for="send in sends" :key="send.record.id" class="send-card">
        <span class="send-icon"><m3e-icon :name="send.record.type === 1 ? 'description' : 'notes'"></m3e-icon></span>
        <div class="send-main">
          <div class="send-title">
            <strong>{{ send.name }}</strong>
            <span class="status-dot" :class="status(send).tone">{{ status(send).label }}</span>
            <m3e-icon v-if="send.record.passwordProtected" name="key" :title="t('send.passwordProtected')"></m3e-icon>
          </div>
          <small class="muted">
            {{ send.record.type === 1 ? `${send.fileName} · ${formatBytes(send.record.size)}` : t("send.textSend") }}
            · {{ t("send.accessCount", { count: send.record.accessCount, max: send.record.maxAccessCount ?? "∞" }) }}
            · {{ t("send.deletesAt", { time: formatRelative(send.record.deletionDate) }) }}
          </small>
        </div>
        <div class="send-actions">
          <m3e-icon-button :aria-label="t('send.copyLink')" :title="t('send.copyLink')" @click="copy(send.shareUrl, 'toast.linkCopied')"><m3e-icon name="link"></m3e-icon></m3e-icon-button>
          <m3e-icon-button :aria-label="send.record.disabled ? t('send.enable') : t('send.disable')" :title="send.record.disabled ? t('send.enable') : t('send.disable')" @click="toggle(send)">
            <m3e-icon :name="send.record.disabled ? 'toggle_off' : 'toggle_on'"></m3e-icon>
          </m3e-icon-button>
          <m3e-icon-button :aria-label="t('common.delete')" @click="remove(send)"><m3e-icon name="delete"></m3e-icon></m3e-icon-button>
        </div>
      </article>
    </div>
    <div v-else class="empty-state tall">
      <span class="empty-icon"><m3e-icon name="send"></m3e-icon></span>
      <strong>{{ t("send.empty") }}</strong>
      <p>{{ t("send.emptyHint") }}</p>
    </div>

    <m3e-dialog :open="creating" dismissible @closed="creating = false">
      <span slot="header">{{ created ? t("send.readyTitle") : t("send.new") }}</span>
      <div v-if="created" class="dialog-body">
        <p class="muted">{{ t("send.readyBody") }}</p>
        <div class="share-link mono">{{ created.shareUrl }}</div>
        <p class="notice"><m3e-icon name="info"></m3e-icon>{{ t("send.keyInLink") }}</p>
      </div>
      <form v-else class="dialog-body form-group" @submit.prevent="submit">
        <div class="segmented">
          <button type="button" :class="{ selected: kind === 'text' }" @click="kind = 'text'"><m3e-icon name="notes"></m3e-icon>{{ t("send.text") }}</button>
          <button type="button" :class="{ selected: kind === 'file' }" @click="kind = 'file'"><m3e-icon name="upload_file"></m3e-icon>{{ t("send.file") }}</button>
        </div>
        <label class="field"><span>{{ t("send.name") }}</span><input v-model="draft.name" /></label>
        <label v-if="kind === 'text'" class="field"><span>{{ t("send.text") }}</span><textarea v-model="draft.text" rows="5" required></textarea></label>
        <div v-else class="file-drop" role="button" tabindex="0" @click="fileInput?.click()" @keydown.enter.prevent="fileInput?.click()" @keydown.space.prevent="fileInput?.click()">
          <m3e-icon name="upload_file"></m3e-icon>
          <span>{{ draft.file ? `${draft.file.name} · ${formatBytes(draft.file.size)}` : t("send.pickFile") }}</span>
          <input ref="fileInput" type="file" hidden @change="onFile" />
        </div>
        <div class="field-pair">
          <label class="field">
            <span>{{ t("send.deleteAfter") }}</span>
            <select v-model.number="draft.deletionDays">
              <option v-for="d in [1, 2, 3, 7, 14, 30]" :key="d" :value="d">{{ t("send.days", { count: d }) }}</option>
            </select>
          </label>
          <label class="field">
            <span>{{ t("send.maxAccess") }}</span>
            <input v-model.number="draft.maxAccessCount" type="number" min="1" :placeholder="t('send.unlimited')" />
          </label>
        </div>
        <label class="field"><span>{{ t("send.password") }}</span><PasswordInput v-model="draft.password" autocomplete="new-password" :placeholder="t('common.optional')" /></label>
        <label class="field"><span>{{ t("send.privateNotes") }}</span><input v-model="draft.notes" :placeholder="t('send.privateNotesHint')" /></label>
        <label class="switch-field"><input v-model="draft.hideEmail" type="checkbox" /><span>{{ t("send.hideEmail") }}</span></label>
        <p v-if="formError" class="form-error" role="alert">{{ formError }}</p>
        <button type="submit" hidden tabindex="-1"></button>
      </form>
      <div slot="actions" end>
        <template v-if="created">
          <m3e-button variant="text" @click="creating = false">{{ t("common.done") }}</m3e-button>
          <m3e-button variant="filled" @click="copy(created.shareUrl, 'toast.linkCopied')"><m3e-icon slot="icon" name="link"></m3e-icon>{{ t("send.copyLink") }}</m3e-button>
        </template>
        <template v-else>
          <m3e-button variant="text" @click="creating = false">{{ t("common.cancel") }}</m3e-button>
          <m3e-button variant="filled" :disabled="saving" @click="submit">{{ saving ? t("attachment.encrypting") : t("send.create") }}</m3e-button>
        </template>
      </div>
    </m3e-dialog>
  </div>
</template>
