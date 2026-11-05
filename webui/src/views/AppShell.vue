<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref, watch } from "vue";
import { t, type MessageKey } from "../i18n";
import { formatRelative } from "../lib/format";
import { itemTypes } from "../lib/items";
import { go, route, routeKey, searchQuery } from "../lib/router";
import { account, isAdmin, lock, logout } from "../lib/session";
import { toastError } from "../lib/ui";
import { createFolder, syncIfChanged, syncVault, vault } from "../lib/vault";
import ActivityView from "./ActivityView.vue";
import AdminServerView from "./AdminServerView.vue";
import AdminUsersView from "./AdminUsersView.vue";
import DevicesView from "./DevicesView.vue";
import SendView from "./SendView.vue";
import SettingsView from "./SettingsView.vue";
import VaultView from "./VaultView.vue";

const navOpen = ref(false);
const menuOpen = ref(false);
const newFolder = ref<string | null>(null);
const tick = ref(0);

// One pass over the vault for every navigation badge.
const counts = computed(() => {
  const byType = new Map<number, number>();
  const folders = new Map<string, number>();
  let all = 0;
  let favorites = 0;
  let archive = 0;
  let trash = 0;
  for (const item of vault.items) {
    if (item.trashed) trash++;
    else if (item.archived) archive++;
    else {
      all++;
      if (item.favorite) favorites++;
      byType.set(item.type, (byType.get(item.type) ?? 0) + 1);
      if (item.folderId) folders.set(item.folderId, (folders.get(item.folderId) ?? 0) + 1);
    }
  }
  return { all, favorites, archive, trash, byType, folders };
});
const visibleTypes = computed(() => itemTypes.filter((type) => type.key === "login" || (counts.value.byType.get(type.id) ?? 0) > 0));
const current = computed(() => routeKey(route.value));
const syncLabel = computed(() => {
  void tick.value;
  if (vault.syncing) return t("sync.syncing");
  if (vault.error) return t("sync.failed");
  return vault.lastSyncAt ? t("sync.lastSynced", { time: formatRelative(vault.lastSyncAt) }) : t("sync.never");
});
const initials = computed(() => (account.value?.name || account.value?.email || "?").slice(0, 1).toUpperCase());

function navigate(path: string) {
  go(path);
  navOpen.value = false;
}

interface NavEntry {
  path: string;
  icon: string;
  label: string;
  count?: number;
}

const vaultNav = computed<NavEntry[]>(() => [
  { path: "vault/all", icon: "inventory_2", label: t("nav.allItems"), count: counts.value.all },
  { path: "vault/favorites", icon: "star", label: t("nav.favorites"), count: counts.value.favorites },
  ...visibleTypes.value.map((type) => ({
    path: `vault/${type.key}`,
    icon: type.icon,
    label: t(type.labelKey),
    count: counts.value.byType.get(type.id) ?? 0
  }))
]);

const accountNav = computed<NavEntry[]>(() => [
  { path: "send", icon: "send", label: t("nav.send"), count: vault.sends.length || undefined },
  { path: "devices", icon: "devices", label: t("nav.devices") },
  { path: "activity", icon: "history", label: t("nav.activity") },
  { path: "settings", icon: "settings", label: t("nav.settings") }
]);

const adminNav: { path: string; icon: string; labelKey: MessageKey }[] = [
  { path: "admin/users", icon: "group", labelKey: "nav.adminUsers" },
  { path: "admin/server", icon: "dns", labelKey: "nav.adminServer" },
  { path: "admin/activity", icon: "policy", labelKey: "nav.adminActivity" }
];

async function addFolder() {
  const name = newFolder.value?.trim();
  newFolder.value = null;
  if (!name) return;
  try {
    const id = await createFolder(name);
    navigate(`vault/folder/${id}`);
  } catch (e) {
    toastError(e);
  }
}

async function manualSync() {
  try {
    await syncVault();
  } catch (e) {
    toastError(e);
  }
}

watch(searchQuery, (value) => {
  if (value && route.value.name !== "vault") go("vault/all");
});

let poll: number | undefined;
let clock: number | undefined;
function onFocus() {
  syncIfChanged().catch(() => undefined);
}
const searchInput = ref<HTMLInputElement | null>(null);
function onKeydown(event: KeyboardEvent) {
  if (event.key === "Escape") {
    menuOpen.value = false;
    navOpen.value = false;
    return;
  }
  // "/" or Ctrl/⌘+K jumps to the search box, as in most web apps.
  const target = event.target as HTMLElement | null;
  const typing = !!target && (target.isContentEditable || /^(INPUT|TEXTAREA|SELECT)$/.test(target.tagName));
  if ((event.key === "/" && !typing) || (event.key.toLowerCase() === "k" && (event.ctrlKey || event.metaKey))) {
    event.preventDefault();
    searchInput.value?.focus();
    searchInput.value?.select();
  }
}
onMounted(() => {
  manualSync();
  poll = window.setInterval(() => {
    if (document.visibilityState === "visible") onFocus();
  }, 60_000);
  clock = window.setInterval(() => tick.value++, 30_000);
  window.addEventListener("focus", onFocus);
  window.addEventListener("keydown", onKeydown);
});
onUnmounted(() => {
  window.clearInterval(poll);
  window.clearInterval(clock);
  window.removeEventListener("focus", onFocus);
  window.removeEventListener("keydown", onKeydown);
});
</script>

<template>
  <div class="shell" :class="{ 'nav-open': navOpen }">
    <div class="scrim" @click="navOpen = false"></div>
    <aside class="nav">
      <div class="nav-brand">
        <img src="/monica-logo.png" alt="" />
        <span>Monica Cloud</span>
      </div>
      <nav class="nav-scroll">
        <p class="nav-label">{{ t("nav.vault") }}</p>
        <button
          v-for="entry in vaultNav"
          :key="entry.path"
          type="button"
          class="nav-item"
          :class="{ selected: current === entry.path }"
          @click="navigate(entry.path)"
        >
          <m3e-icon :name="entry.icon" :filled="current === entry.path"></m3e-icon>
          <span class="nav-text">{{ entry.label }}</span>
          <span v-if="entry.count" class="nav-count">{{ entry.count }}</span>
        </button>

        <div class="nav-label with-action">
          <span>{{ t("nav.folders") }}</span>
          <m3e-icon-button :aria-label="t('folder.new')" @click="newFolder = ''"><m3e-icon name="create_new_folder"></m3e-icon></m3e-icon-button>
        </div>
        <form v-if="newFolder !== null" class="nav-new-folder" @submit.prevent="addFolder">
          <input v-model="newFolder" :placeholder="t('folder.namePlaceholder')" autofocus @blur="addFolder" @keydown.esc="newFolder = null" />
        </form>
        <button
          v-for="folder in vault.folders"
          :key="folder.id"
          type="button"
          class="nav-item"
          :class="{ selected: current === `vault/folder/${folder.id}` }"
          @click="navigate(`vault/folder/${folder.id}`)"
        >
          <m3e-icon name="folder"></m3e-icon>
          <span class="nav-text">{{ folder.name }}</span>
          <span v-if="counts.folders.get(folder.id)" class="nav-count">{{ counts.folders.get(folder.id) }}</span>
        </button>
        <p v-if="!vault.folders.length && newFolder === null" class="nav-empty">{{ t("nav.noFolders") }}</p>

        <button type="button" class="nav-item" :class="{ selected: current === 'vault/archive' }" @click="navigate('vault/archive')">
          <m3e-icon name="archive"></m3e-icon>
          <span class="nav-text">{{ t("nav.archive") }}</span>
          <span v-if="counts.archive" class="nav-count">{{ counts.archive }}</span>
        </button>
        <button type="button" class="nav-item" :class="{ selected: current === 'vault/trash' }" @click="navigate('vault/trash')">
          <m3e-icon name="delete"></m3e-icon>
          <span class="nav-text">{{ t("nav.trash") }}</span>
          <span v-if="counts.trash" class="nav-count">{{ counts.trash }}</span>
        </button>

        <p class="nav-label">{{ t("nav.account") }}</p>
        <button
          v-for="entry in accountNav"
          :key="entry.path"
          type="button"
          class="nav-item"
          :class="{ selected: current === entry.path.replace('/', '-') || current === entry.path }"
          @click="navigate(entry.path)"
        >
          <m3e-icon :name="entry.icon"></m3e-icon>
          <span class="nav-text">{{ entry.label }}</span>
          <span v-if="entry.count" class="nav-count">{{ entry.count }}</span>
        </button>

        <template v-if="isAdmin">
          <p class="nav-label">{{ t("nav.admin") }}</p>
          <button
            v-for="entry in adminNav"
            :key="entry.path"
            type="button"
            class="nav-item"
            :class="{ selected: current === entry.path.replace('/', '-') }"
            @click="navigate(entry.path)"
          >
            <m3e-icon :name="entry.icon"></m3e-icon>
            <span class="nav-text">{{ t(entry.labelKey) }}</span>
          </button>
        </template>
      </nav>
      <div class="nav-footer">
        <m3e-icon name="encrypted"></m3e-icon>
        <span>{{ t("nav.zeroKnowledge") }}</span>
      </div>
    </aside>

    <div class="main">
      <header class="topbar">
        <m3e-icon-button class="menu-button" :aria-label="t('common.menu')" @click="navOpen = true"><m3e-icon name="menu"></m3e-icon></m3e-icon-button>
        <label class="search">
          <m3e-icon name="search"></m3e-icon>
          <input ref="searchInput" v-model="searchQuery" type="search" :placeholder="t('common.searchVault')" :aria-label="t('common.searchVault')" aria-keyshortcuts="/ Control+K" />
          <m3e-icon-button v-if="searchQuery" :aria-label="t('common.clear')" @click="searchQuery = ''"><m3e-icon name="close"></m3e-icon></m3e-icon-button>
        </label>
        <button type="button" class="sync-pill" :class="{ error: vault.error }" :title="vault.error || syncLabel" @click="manualSync">
          <m3e-icon :name="vault.error ? 'sync_problem' : 'sync'" :class="{ spinning: vault.syncing }"></m3e-icon>
          <span>{{ syncLabel }}</span>
        </button>
        <m3e-icon-button :aria-label="t('common.lock')" :title="t('common.lock')" @click="lock"><m3e-icon name="lock"></m3e-icon></m3e-icon-button>
        <div class="account-menu">
          <button type="button" class="avatar-button" :aria-label="t('common.account')" @click="menuOpen = !menuOpen">{{ initials }}</button>
          <div v-if="menuOpen" class="popover" @click="menuOpen = false">
            <div class="popover-head">
              <strong>{{ account?.name || account?.email }}</strong>
              <small>{{ account?.email }}</small>
              <span class="role-chip">{{ t(`role.${account?.role ?? 'user'}` as MessageKey) }}</span>
            </div>
            <button type="button" @click="navigate('settings')"><m3e-icon name="settings"></m3e-icon>{{ t("nav.settings") }}</button>
            <button type="button" @click="lock"><m3e-icon name="lock"></m3e-icon>{{ t("common.lock") }}</button>
            <button type="button" @click="logout"><m3e-icon name="logout"></m3e-icon>{{ t("common.signOut") }}</button>
          </div>
          <div v-if="menuOpen" class="popover-scrim" @click="menuOpen = false"></div>
        </div>
      </header>

      <main class="content">
        <VaultView v-if="route.name === 'vault'" :route="route" />
        <SendView v-else-if="route.name === 'send'" />
        <DevicesView v-else-if="route.name === 'devices'" />
        <ActivityView v-else-if="route.name === 'activity'" :admin="false" />
        <SettingsView v-else-if="route.name === 'settings'" />
        <AdminUsersView v-else-if="route.name === 'admin-users' && isAdmin" />
        <AdminServerView v-else-if="route.name === 'admin-server' && isAdmin" />
        <ActivityView v-else-if="route.name === 'admin-activity' && isAdmin" :admin="true" />
        <VaultView v-else :route="{ name: 'vault', filter: 'all' }" />
      </main>
    </div>
  </div>
</template>
