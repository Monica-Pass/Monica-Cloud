<script setup lang="ts">
import { computed, onMounted, ref } from "vue";
import { t, type MessageKey } from "../i18n";
import type { AdminUser } from "../lib/client";
import { formatBytes, formatRelative } from "../lib/format";
import { account, client } from "../lib/session";
import { confirm, toast, toastError } from "../lib/ui";

const users = ref<AdminUser[]>([]);
const loading = ref(true);
const filter = ref("");
const isOwner = computed(() => account.value?.role === "owner");
/** Admins cannot change, sign out or delete owner accounts; the server refuses those requests. */
const canManage = (user: AdminUser) => isOwner.value || user.role !== "owner";
const shown = computed(() => {
  const q = filter.value.trim().toLowerCase();
  return q ? users.value.filter((u) => `${u.email} ${u.name}`.toLowerCase().includes(q)) : users.value;
});

async function load() {
  loading.value = true;
  try {
    users.value = await client.adminUsers();
  } catch (e) {
    toastError(e);
  } finally {
    loading.value = false;
  }
}

async function patch(user: AdminUser, change: Parameters<typeof client.adminUpdateUser>[1]) {
  try {
    const updated = await client.adminUpdateUser(user.id, change);
    users.value = users.value.map((u) => (u.id === user.id ? updated : u));
  } catch (e) {
    toastError(e);
    // Re-render the row so a rejected role choice snaps back to the stored role.
    users.value = users.value.map((u) => (u.id === user.id ? { ...u } : u));
  }
}

async function setQuota(user: AdminUser) {
  const current = user.quotaBytes ? String(Math.round(user.quotaBytes / 1048576)) : "";
  const input = window.prompt(t("admin.quotaPrompt", { email: user.email }), current);
  if (input === null) return;
  const mb = input.trim() === "" ? null : Number(input);
  if (mb !== null && (!Number.isFinite(mb) || mb < 0)) return toast(t("admin.quotaInvalid"), "error");
  await patch(user, { quotaBytes: mb === null ? null : Math.round(mb * 1048576) });
}

async function toggle(user: AdminUser) {
  if (user.enabled && !(await confirm(t("admin.disableTitle"), t("admin.disableBody", { email: user.email }), { danger: true, confirmLabel: t("admin.disable") }))) return;
  await patch(user, { enabled: !user.enabled });
}

async function deauth(user: AdminUser) {
  if (!(await confirm(t("admin.deauthTitle"), t("admin.deauthBody", { email: user.email }), { confirmLabel: t("admin.deauth") }))) return;
  try {
    await client.adminDeauthUser(user.id);
    toast(t("admin.deauthDone"));
    await load();
  } catch (e) {
    toastError(e);
  }
}

async function remove(user: AdminUser) {
  if (!(await confirm(t("admin.deleteTitle"), t("admin.deleteBody", { email: user.email }), { danger: true, confirmLabel: t("common.delete") }))) return;
  try {
    await client.adminDeleteUser(user.id);
    users.value = users.value.filter((u) => u.id !== user.id);
  } catch (e) {
    toastError(e);
  }
}

onMounted(load);
</script>

<template>
  <div class="page wide">
    <header class="page-header">
      <div>
        <h1>{{ t("nav.adminUsers") }}</h1>
        <p class="muted">{{ t("admin.usersSubtitle", { count: users.length }) }}</p>
      </div>
      <label class="search compact">
        <m3e-icon name="search"></m3e-icon>
        <input v-model="filter" type="search" :placeholder="t('admin.filterUsers')" />
      </label>
    </header>

    <div class="table-card">
      <table class="table">
        <thead>
          <tr>
            <th>{{ t("admin.user") }}</th>
            <th>{{ t("settings.role") }}</th>
            <th class="num">{{ t("settings.items") }}</th>
            <th>{{ t("settings.storage") }}</th>
            <th class="num">{{ t("nav.devices") }}</th>
            <th>{{ t("admin.lastLogin") }}</th>
            <th></th>
          </tr>
        </thead>
        <tbody>
          <tr v-for="user in shown" :key="user.id" :class="{ disabled: !user.enabled }">
            <td>
              <div class="user-cell">
                <span class="avatar-small">{{ (user.name || user.email).slice(0, 1).toUpperCase() }}</span>
                <div>
                  <strong>{{ user.name || user.email }}</strong>
                  <small class="muted">{{ user.email }}<template v-if="!user.enabled"> · {{ t("admin.disabled") }}</template></small>
                </div>
              </div>
            </td>
            <td>
              <select v-if="isOwner && user.id !== account?.id" class="inline-select" :value="user.role" @change="patch(user, { role: ($event.target as HTMLSelectElement).value as AdminUser['role'] })">
                <option v-for="role in ['owner', 'admin', 'user']" :key="role" :value="role">{{ t(`role.${role}` as MessageKey) }}</option>
              </select>
              <span v-else class="role-chip">{{ t(`role.${user.role}` as MessageKey) }}</span>
            </td>
            <td class="num">{{ user.itemCount }}</td>
            <td>
              <button v-if="canManage(user)" type="button" class="link-button" @click="setQuota(user)">
                {{ formatBytes(user.storageUsedBytes) }} / {{ user.effectiveQuotaBytes ? formatBytes(user.effectiveQuotaBytes) : "∞" }}
              </button>
              <span v-else>{{ formatBytes(user.storageUsedBytes) }} / {{ user.effectiveQuotaBytes ? formatBytes(user.effectiveQuotaBytes) : "∞" }}</span>
            </td>
            <td class="num">{{ user.deviceCount }}</td>
            <td>{{ formatRelative(user.lastLoginDate) }}</td>
            <td class="actions">
              <template v-if="user.id !== account?.id && canManage(user)">
                <m3e-icon-button :aria-label="user.enabled ? t('admin.disable') : t('admin.enable')" :title="user.enabled ? t('admin.disable') : t('admin.enable')" @click="toggle(user)">
                  <m3e-icon :name="user.enabled ? 'person_off' : 'person_check'"></m3e-icon>
                </m3e-icon-button>
                <m3e-icon-button :aria-label="t('admin.deauth')" :title="t('admin.deauth')" @click="deauth(user)"><m3e-icon name="logout"></m3e-icon></m3e-icon-button>
                <m3e-icon-button :aria-label="t('common.delete')" :title="t('common.delete')" @click="remove(user)"><m3e-icon name="delete"></m3e-icon></m3e-icon-button>
              </template>
              <span v-else-if="user.id === account?.id" class="muted small">{{ t("admin.you") }}</span>
            </td>
          </tr>
        </tbody>
      </table>
      <p v-if="!loading && !shown.length" class="muted pad">{{ t("list.noResults") }}</p>
    </div>
  </div>
</template>
