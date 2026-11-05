<script setup lang="ts">
import { computed, nextTick, onBeforeUnmount, onMounted, ref, watch } from "vue";
import ItemDetail from "../components/ItemDetail.vue";
import ItemEditor from "../components/ItemEditor.vue";
import ItemIcon from "../components/ItemIcon.vue";
import { t } from "../i18n";
import { formatRelative, nameCollator } from "../lib/format";
import { creatableTypes, itemSubtitle, typeByKey } from "../lib/items";
import { type Route, go, searchQuery } from "../lib/router";
import { confirm, toastError } from "../lib/ui";
import { deleteFolder, emptyTrash, renameFolder, vault, type VaultItem } from "../lib/vault";

const props = defineProps<{ route: Extract<Route, { name: "vault" }> }>();

// Rows have a fixed height, so only the rows in view (plus a margin) are rendered. A vault
// with thousands of items costs the same to scroll, filter and select as one with twenty.
const ROW_HEIGHT = 64;
const ROW_STRIDE = ROW_HEIGHT + 2; // .item-list gap
const OVERSCAN = 6;

type Editing = { mode: "new" } | { mode: "edit"; id: string };

const selectedId = ref<string | null>(null);
const editing = ref<Editing | null>(null);
const newType = ref(1);
const sort = ref<"name" | "updated">("name");
const editor = ref<{ dirty: boolean } | null>(null);
const pane = ref<HTMLElement | null>(null);
const header = ref<HTMLElement | null>(null);
const listEl = ref<HTMLElement | null>(null);
const scrollTop = ref(0);
const viewport = ref(800);
/** Distance from the top of the scrolling pane to the first row (the sticky header). */
const listTop = ref(0);

const folder = computed(() => (props.route.filter === "folder" ? vault.folders.find((f) => f.id === props.route.folderId) : undefined));
const filterType = computed(() => typeByKey(props.route.filter));

const title = computed(() => {
  const filter = props.route.filter;
  if (folder.value) return folder.value.name;
  if (filterType.value) return t(filterType.value.labelKey);
  if (filter === "favorites") return t("nav.favorites");
  if (filter === "archive") return t("nav.archive");
  if (filter === "trash") return t("nav.trash");
  return t("nav.allItems");
});

const query = computed(() => searchQuery.value.trim().toLowerCase());
const searching = computed(() => query.value !== "");

/** Items of the current view in display order. Typing in the search box does not re-sort. */
const sorted = computed(() => {
  const filter = props.route.filter;
  const typeId = filterType.value?.id;
  const items = vault.items.filter((item) => {
    if (filter === "trash") return item.trashed;
    if (item.trashed) return false;
    if (filter === "archive") return item.archived;
    if (item.archived && !searching.value) return false;
    if (filter === "favorites") return item.favorite;
    if (filter === "folder") return item.folderId === props.route.folderId;
    if (typeId !== undefined) return item.type === typeId;
    return true;
  });
  if (sort.value === "updated") return items.sort((a, b) => (a.revisionDate < b.revisionDate ? 1 : a.revisionDate > b.revisionDate ? -1 : 0));
  const collator = nameCollator.value;
  return items.sort((a, b) => collator.compare(a.payload.name, b.payload.name));
});

const list = computed(() => (query.value ? sorted.value.filter((item) => item.search.includes(query.value)) : sorted.value));

const range = computed(() => {
  const offset = Math.max(0, scrollTop.value - listTop.value);
  const start = Math.max(0, Math.floor(offset / ROW_STRIDE) - OVERSCAN);
  const end = Math.min(list.value.length, Math.ceil((offset + viewport.value) / ROW_STRIDE) + OVERSCAN);
  return { start, end };
});
const rows = computed(() => list.value.slice(range.value.start, range.value.end).map((item, i) => ({ item, index: range.value.start + i })));
const listStyle = computed(() => ({
  paddingTop: `${range.value.start * ROW_STRIDE}px`,
  paddingBottom: `${(list.value.length - range.value.end) * ROW_STRIDE + 12}px`
}));

const selected = computed(() => (selectedId.value ? vault.items.find((item) => item.id === selectedId.value) ?? null : null));
const editingItem = computed(() => {
  const current = editing.value;
  return current?.mode === "edit" ? vault.items.find((item) => item.id === current.id) ?? null : null;
});
const editorKey = computed(() => (editing.value?.mode === "edit" ? `edit-${editing.value.id}` : `new-${newType.value}`));
const canCreate = computed(() => props.route.filter !== "trash" && props.route.filter !== "archive");

function measure() {
  if (!pane.value) return;
  viewport.value = pane.value.clientHeight;
  listTop.value = header.value?.offsetHeight ?? 0;
}

function onScroll() {
  if (pane.value) scrollTop.value = pane.value.scrollTop;
}

let resizeObserver: ResizeObserver | undefined;
onMounted(() => {
  measure();
  resizeObserver = new ResizeObserver(measure);
  if (pane.value) resizeObserver.observe(pane.value);
  if (header.value) resizeObserver.observe(header.value);
});
onBeforeUnmount(() => resizeObserver?.disconnect());

function scrollToTop() {
  if (pane.value) pane.value.scrollTop = 0;
  scrollTop.value = 0;
}

/** Scroll the pane just enough to show row `index` below the sticky header. */
function reveal(index: number) {
  const el = pane.value;
  if (!el || index < 0) return;
  const top = listTop.value + index * ROW_STRIDE;
  if (top - listTop.value < el.scrollTop) el.scrollTop = top - listTop.value;
  else if (top + ROW_HEIGHT > el.scrollTop + el.clientHeight) el.scrollTop = top + ROW_HEIGHT - el.clientHeight;
  scrollTop.value = el.scrollTop;
}

async function focusRow(index: number) {
  if (!list.value.length) return;
  const target = Math.max(0, Math.min(list.value.length - 1, index));
  reveal(target);
  await nextTick();
  listEl.value?.querySelector<HTMLElement>(`[data-index="${target}"]`)?.focus({ preventScroll: true });
}

function onRowKeydown(event: KeyboardEvent, item: VaultItem, index: number) {
  const moves: Record<string, number> = { ArrowDown: index + 1, ArrowUp: index - 1, Home: 0, End: list.value.length - 1, PageDown: index + 8, PageUp: index - 8 };
  if (event.key === "Enter" || event.key === " ") {
    event.preventDefault();
    select(item);
  } else if (event.key in moves) {
    event.preventDefault();
    void focusRow(moves[event.key]);
  }
}

watch(
  () => [props.route.filter, props.route.folderId],
  () => {
    if (editing.value?.mode === "edit") editing.value = null;
    if (selectedId.value && !list.value.some((i) => i.id === selectedId.value)) selectedId.value = null;
    scrollToTop();
  }
);
// A new result set starts at its first row.
watch([query, sort], scrollToTop);
// On phones the list is hidden while an item is open, which resets its scroll position;
// put the user back where they were.
watch(
  () => Boolean(selected.value || editing.value),
  (open) => {
    if (!open && pane.value) pane.value.scrollTop = scrollTop.value;
  },
  { flush: "post" }
);

/** Leaving the editor with unsaved changes needs confirmation. */
function mayLeaveEditor() {
  return !editing.value || !editor.value?.dirty || window.confirm(t("item.discardChanges"));
}

function select(item: VaultItem) {
  if (editing.value && selectedId.value === item.id && editing.value.mode === "edit") return;
  if (!mayLeaveEditor()) return;
  editing.value = null;
  selectedId.value = item.id;
}

function startNew(type?: number) {
  if (!mayLeaveEditor()) return;
  newType.value = type ?? (filterType.value && filterType.value.key !== "passkey" ? filterType.value.id : 1);
  selectedId.value = null;
  editing.value = { mode: "new" };
}

function startEdit() {
  if (selectedId.value) editing.value = { mode: "edit", id: selectedId.value };
}

async function onSaved(item: VaultItem) {
  editing.value = null;
  selectedId.value = item.id;
  await nextTick();
  reveal(list.value.findIndex((i) => i.id === item.id));
}

function onGone() {
  selectedId.value = null;
  editing.value = null;
}

async function clearTrash() {
  if (!(await confirm(t("trash.emptyTitle"), t("trash.emptyBody", { count: list.value.length }), { danger: true, confirmLabel: t("trash.empty") }))) return;
  try {
    await emptyTrash();
    selectedId.value = null;
  } catch (e) {
    toastError(e);
  }
}

async function renameCurrentFolder() {
  if (!folder.value) return;
  const name = window.prompt(t("folder.rename"), folder.value.name)?.trim();
  if (!name || name === folder.value.name) return;
  try {
    await renameFolder(folder.value.id, name);
  } catch (e) {
    toastError(e);
  }
}

async function deleteCurrentFolder() {
  if (!folder.value) return;
  if (!(await confirm(t("folder.deleteTitle"), t("folder.deleteBody", { name: folder.value.name }), { danger: true, confirmLabel: t("common.delete") }))) return;
  try {
    await deleteFolder(folder.value.id);
    go("vault/all");
  } catch (e) {
    toastError(e);
  }
}

const emptyIcon = computed(() => (props.route.filter === "trash" ? "delete" : props.route.filter === "archive" ? "archive" : filterType.value?.icon ?? "inventory_2"));
</script>

<template>
  <div class="vault" :class="{ 'has-detail': selected || editing }">
    <section ref="pane" class="list-pane" @scroll.passive="onScroll">
      <header ref="header" class="pane-header">
        <div class="pane-title">
          <h1>{{ title }}</h1>
          <span class="count">{{ list.length }}</span>
        </div>
        <div class="pane-actions">
          <m3e-icon-button
            :aria-label="t('list.sort')"
            :title="sort === 'name' ? t('list.sortName') : t('list.sortUpdated')"
            @click="sort = sort === 'name' ? 'updated' : 'name'"
          >
            <m3e-icon :name="sort === 'name' ? 'sort_by_alpha' : 'schedule'"></m3e-icon>
          </m3e-icon-button>
          <template v-if="folder">
            <m3e-icon-button :aria-label="t('folder.rename')" :title="t('folder.rename')" @click="renameCurrentFolder"><m3e-icon name="edit"></m3e-icon></m3e-icon-button>
            <m3e-icon-button :aria-label="t('common.delete')" :title="t('common.delete')" @click="deleteCurrentFolder"><m3e-icon name="folder_delete"></m3e-icon></m3e-icon-button>
          </template>
          <m3e-button v-if="route.filter === 'trash' && list.length" variant="text" @click="clearTrash">{{ t("trash.empty") }}</m3e-button>
          <m3e-button v-if="canCreate" variant="filled" @click="startNew()">
            <m3e-icon slot="icon" name="add"></m3e-icon>{{ t("item.new") }}
          </m3e-button>
        </div>
      </header>

      <div v-if="vault.syncing && !vault.items.length" class="list-skeleton" aria-hidden="true">
        <div v-for="i in 6" :key="i" class="skeleton-row"><span></span><div><i></i><i></i></div></div>
      </div>
      <ul v-else-if="list.length" ref="listEl" class="item-list" role="listbox" :aria-label="title" :style="listStyle">
        <li
          v-for="{ item, index } in rows"
          :key="item.id"
          role="option"
          :data-index="index"
          :aria-selected="item.id === selectedId"
          :aria-setsize="list.length"
          :aria-posinset="index + 1"
          :class="{ selected: item.id === selectedId, broken: item.error }"
          tabindex="0"
          @click="select(item)"
          @keydown="onRowKeydown($event, item, index)"
        >
          <ItemIcon :type="item.type" :name="item.payload.name" />
          <div class="item-text">
            <strong>{{ item.payload.name || t("item.untitled") }}</strong>
            <small>{{ item.error ? t("item.decryptFailed") : itemSubtitle(item.type, item.payload) || formatRelative(item.revisionDate) }}</small>
          </div>
          <m3e-icon v-if="item.attachments.length" class="row-badge" name="attach_file"></m3e-icon>
          <m3e-icon v-if="item.favorite" class="row-badge star" name="star" filled></m3e-icon>
        </li>
      </ul>
      <div v-else class="empty-state">
        <span class="empty-icon"><m3e-icon :name="searchQuery ? 'search_off' : emptyIcon"></m3e-icon></span>
        <strong>{{ searchQuery ? t("list.noResults") : t("list.empty") }}</strong>
        <p v-if="!searchQuery && canCreate">{{ t("list.emptyHint") }}</p>
      </div>
    </section>

    <section class="detail-pane">
      <ItemEditor
        v-if="editing"
        ref="editor"
        :key="editorKey"
        :item="editingItem"
        :initial-type="newType"
        :initial-folder="route.filter === 'folder' ? route.folderId ?? null : null"
        :initial-favorite="route.filter === 'favorites'"
        @saved="onSaved"
        @cancel="editing = null"
      />
      <ItemDetail v-else-if="selected" :key="selected.id" :item="selected" @edit="startEdit" @gone="onGone" @back="selectedId = null" />
      <div v-else class="detail-empty">
        <img src="/monica-logo.png" alt="" />
        <p>{{ t("detail.pick") }}</p>
        <div v-if="canCreate" class="type-shortcuts">
          <button v-for="type in creatableTypes.slice(0, 6)" :key="type.id" type="button" @click="startNew(type.id)">
            <m3e-icon :name="type.icon"></m3e-icon>{{ t(type.labelKey) }}
          </button>
        </div>
      </div>
    </section>
  </div>
</template>
