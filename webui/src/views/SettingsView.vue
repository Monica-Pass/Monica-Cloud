<script setup lang="ts">
import { computed, onMounted, ref } from "vue";
import AppearancePanel from "../components/AppearancePanel.vue";
import PasswordInput from "../components/PasswordInput.vue";
import { t, type MessageKey } from "../i18n";
import type { AccountStats, Profile } from "../lib/client";
import { formatBytes, formatDateTime } from "../lib/format";
import { changeMasterPassword, client, deleteAccount as deleteOwnAccount, updateAccountName } from "../lib/session";
import { confirm, copy, errorMessage, toast, toastError } from "../lib/ui";

const profile = ref<Profile | null>(null);
const stats = ref<AccountStats | null>(null);
const name = ref("");
const current = ref("");
const next = ref("");
const repeat = ref("");
const passwordError = ref("");
const busy = ref(false);
const deletePassword = ref("");
const serverUrl = computed(() => location.origin + location.pathname.replace(/\/$/, ""));
const usage = computed(() => (stats.value?.storageQuotaBytes ? Math.min(1, stats.value.storageUsedBytes / stats.value.storageQuotaBytes) : 0));

onMounted(async () => {
  try {
    [profile.value, stats.value] = await Promise.all([client.profile(), client.stats()]);
    name.value = profile.value.name;
  } catch (e) {
    toastError(e);
  }
});

async function saveName() {
  try {
    profile.value = await client.updateProfile(name.value);
    name.value = profile.value.name;
    updateAccountName(profile.value.name);
    toast(t("settings.saved"));
  } catch (e) {
    toastError(e);
  }
}

async function changePassword() {
  passwordError.value = "";
  if (!current.value) return (passwordError.value = t("settings.currentRequired"));
  if (next.value.length < 12) return (passwordError.value = t("auth.errorShort"));
  if (next.value !== repeat.value) return (passwordError.value = t("auth.errorMismatch"));
  if (!(await confirm(t("settings.changePassword"), t("settings.changePasswordConfirm"), { confirmLabel: t("settings.changePassword") }))) return;
  busy.value = true;
  try {
    await changeMasterPassword(current.value, next.value);
    toast(t("settings.passwordChanged"));
  } catch (e) {
    passwordError.value = errorMessage(e);
  } finally {
    busy.value = false;
    current.value = next.value = repeat.value = "";
  }
}

async function deleteAccount() {
  if (!deletePassword.value) return;
  if (!(await confirm(t("settings.deleteTitle"), t("settings.deleteBody"), { danger: true, confirmLabel: t("settings.deleteAccount") }))) return;
  try {
    await deleteOwnAccount(deletePassword.value);
  } catch (e) {
    toastError(e);
  } finally {
    deletePassword.value = "";
  }
}
</script>

<template>
  <div class="page">
    <header class="page-header">
      <div>
        <h1>{{ t("nav.settings") }}</h1>
        <p class="muted">{{ profile?.email }}</p>
      </div>
    </header>

    <div class="settings-grid">
      <section class="settings-card">
        <h2>{{ t("settings.profile") }}</h2>
        <form class="input-row" @submit.prevent="saveName">
          <label class="field grow"><span>{{ t("auth.name") }}</span><input v-model="name" /></label>
          <m3e-button variant="tonal" type="submit">{{ t("common.save") }}</m3e-button>
        </form>
        <dl class="facts">
          <div><dt>{{ t("auth.email") }}</dt><dd>{{ profile?.email }}</dd></div>
          <div><dt>{{ t("settings.role") }}</dt><dd>{{ profile ? t(`role.${profile.role}` as MessageKey) : "" }}</dd></div>
          <div><dt>{{ t("settings.memberSince") }}</dt><dd>{{ formatDateTime(profile?.creationDate) }}</dd></div>
          <div><dt>{{ t("settings.kdf") }}</dt><dd>PBKDF2-SHA256 · {{ profile?.kdfIterations?.toLocaleString() }}</dd></div>
        </dl>
      </section>

      <section class="settings-card">
        <h2>{{ t("settings.usage") }}</h2>
        <div class="stat-row">
          <div><strong>{{ stats?.items ?? "—" }}</strong><small>{{ t("settings.items") }}</small></div>
          <div><strong>{{ stats?.devices ?? "—" }}</strong><small>{{ t("nav.devices") }}</small></div>
          <div><strong>{{ formatBytes(stats?.storageUsedBytes ?? 0) }}</strong><small>{{ t("settings.storage") }}</small></div>
        </div>
        <div v-if="stats?.storageQuotaBytes" class="meter"><span :style="{ width: `${usage * 100}%` }"></span></div>
        <small class="muted">{{ stats?.storageQuotaBytes ? t("settings.quota", { quota: formatBytes(stats.storageQuotaBytes) }) : t("settings.noQuota") }}</small>
      </section>

      <section class="settings-card">
        <h2>{{ t("settings.connectApp") }}</h2>
        <p class="muted">{{ t("settings.connectAppBody") }}</p>
        <div class="input-row">
          <code class="share-link grow">{{ serverUrl }}</code>
          <m3e-icon-button :aria-label="t('common.copy')" @click="copy(serverUrl)"><m3e-icon name="content_copy"></m3e-icon></m3e-icon-button>
        </div>
      </section>

      <AppearancePanel />

      <section class="settings-card">
        <h2>{{ t("settings.changePassword") }}</h2>
        <form class="form-group" @submit.prevent="changePassword">
          <label class="field"><span>{{ t("settings.currentPassword") }}</span><PasswordInput v-model="current" autocomplete="current-password" /></label>
          <label class="field"><span>{{ t("settings.newPassword") }}</span><PasswordInput v-model="next" autocomplete="new-password" /></label>
          <label class="field"><span>{{ t("auth.confirmPassword") }}</span><PasswordInput v-model="repeat" autocomplete="new-password" /></label>
          <p v-if="passwordError" class="form-error">{{ passwordError }}</p>
          <p class="notice"><m3e-icon name="info"></m3e-icon>{{ t("settings.changePasswordHint") }}</p>
          <m3e-button variant="filled" type="submit" :disabled="busy">{{ t("settings.changePassword") }}</m3e-button>
          <button type="submit" hidden tabindex="-1"></button>
        </form>
      </section>

      <section class="settings-card danger-zone">
        <h2>{{ t("settings.dangerZone") }}</h2>
        <p class="muted">{{ t("settings.deleteHint") }}</p>
        <form class="form-group" @submit.prevent="deleteAccount">
          <label class="field"><span>{{ t("auth.masterPassword") }}</span><PasswordInput v-model="deletePassword" autocomplete="current-password" /></label>
          <m3e-button variant="outlined" type="submit" class="danger">{{ t("settings.deleteAccount") }}</m3e-button>
          <button type="submit" hidden tabindex="-1"></button>
        </form>
      </section>
    </div>
  </div>
</template>
