<script setup lang="ts">
import { computed, onBeforeUnmount, reactive, ref } from "vue";
import { t } from "../i18n";
import {
  type FieldDescriptor,
  creatableTypes,
  emptyPayload,
  generatePassword,
  getPath,
  isFieldVisible,
  optionLabel,
  selectOptions,
  setPath,
  typeFields,
  typeInfo,
  typesWithUris,
  type ItemPayload
} from "../lib/items";
import { confirm, toast, toastError } from "../lib/ui";
import { ConflictError, saveItem, vault, type VaultItem } from "../lib/vault";
import ItemIcon from "./ItemIcon.vue";
import PasswordInput from "./PasswordInput.vue";

const props = defineProps<{ item: VaultItem | null; initialType: number; initialFolder: string | null; initialFavorite: boolean }>();
const emit = defineEmits<{ saved: [item: VaultItem]; cancel: [] }>();

// The item as it was when editing started. Its revision is what the server must still have,
// so a change synced from another device meanwhile is reported as a conflict instead of
// being overwritten silently.
const base = props.item ?? undefined;
const isNew = !base;
const type = ref(base?.type ?? props.initialType);
const folderId = ref(base?.folderId ?? props.initialFolder ?? "");
const favorite = ref(base?.favorite ?? props.initialFavorite);
// Deep copy so cancelling leaves the original untouched; unknown keys are kept.
const payload = reactive<ItemPayload>(base ? JSON.parse(JSON.stringify(base.payload)) : emptyPayload());
payload.uris ??= [];
payload.fields ??= [];
const saving = ref(false);
const error = ref("");

const snapshot = () => JSON.stringify([type.value, folderId.value, favorite.value, payload]);
const pristine = snapshot();
const dirty = computed(() => snapshot() !== pristine);
defineExpose({ dirty });

function warnBeforeUnload(event: BeforeUnloadEvent) {
  if (dirty.value && !saving.value) event.preventDefault();
}
window.addEventListener("beforeunload", warnBeforeUnload);
onBeforeUnmount(() => window.removeEventListener("beforeunload", warnBeforeUnload));

const info = computed(() => typeInfo(type.value));
const fields = computed(() => typeFields[info.value.key].filter((field) => isFieldVisible(field, payload)));
const showUris = computed(() => typesWithUris.includes(info.value.key));

function value(field: FieldDescriptor) {
  const v = getPath(payload, field.path);
  if (field.kind === "boolean") return Boolean(v);
  if (field.defaultValue && (v === undefined || v === null || v === "")) return field.defaultValue;
  return (v as string | undefined) ?? "";
}

function update(field: FieldDescriptor, v: unknown) {
  setPath(payload, field.path, v);
}

function generate(field: FieldDescriptor) {
  const previous = getPath(payload, field.path) as string | undefined;
  if (previous && field.path === "login.password") {
    payload.passwordHistory = [{ password: previous, lastUsedAt: new Date().toISOString() }, ...(payload.passwordHistory ?? [])].slice(0, 10);
  }
  update(field, generatePassword(20));
}

async function save(force = false) {
  error.value = "";
  if (!payload.name.trim()) {
    error.value = t("editor.nameRequired");
    return;
  }
  payload.name = payload.name.trim();
  payload.uris = (payload.uris ?? []).map((u) => u.trim()).filter(Boolean);
  payload.fields = (payload.fields ?? []).filter((f) => f.name.trim() || f.value);
  saving.value = true;
  try {
    // Overwriting after a conflict builds on the server's copy (its item key keeps attachments readable).
    const target = force ? (props.item ?? base) : base;
    const saved = await saveItem(
      { id: base?.id, type: type.value, folderId: folderId.value || null, favorite: favorite.value, archived: target?.archived ?? false, payload },
      target,
      force
    );
    toast(isNew ? t("editor.created") : t("editor.saved"));
    emit("saved", saved);
  } catch (e) {
    if (e instanceof ConflictError) {
      const overwrite = await confirm(t("conflict.title"), t("conflict.body"), { confirmLabel: t("conflict.overwrite"), danger: true });
      if (overwrite) {
        saving.value = false;
        return save(true);
      }
      emit("cancel");
    } else {
      toastError(e);
    }
  } finally {
    saving.value = false;
  }
}
</script>

<template>
  <form class="detail editor" @submit.prevent="save()">
    <header class="detail-header">
      <m3e-icon-button class="back-button" type="button" :aria-label="t('common.back')" @click="emit('cancel')"><m3e-icon name="arrow_back"></m3e-icon></m3e-icon-button>
      <ItemIcon :type="type" :name="payload.name" size="large" />
      <div class="detail-title">
        <h2>{{ isNew ? t("editor.newTitle", { type: t(info.labelKey) }) : t("editor.editTitle") }}</h2>
        <p class="muted small">{{ t("editor.encryptedHint") }}</p>
      </div>
      <div class="detail-actions">
        <m3e-button variant="text" type="button" @click="emit('cancel')">{{ t("common.cancel") }}</m3e-button>
        <m3e-button variant="filled" type="submit" :disabled="saving"><m3e-icon slot="icon" name="check"></m3e-icon>{{ t("common.save") }}</m3e-button>
      </div>
    </header>

    <div class="detail-body">
      <div v-if="isNew" class="type-picker" role="radiogroup" :aria-label="t('editor.type')">
        <button
          v-for="option in creatableTypes"
          :key="option.id"
          type="button"
          role="radio"
          :aria-checked="option.id === type"
          :class="{ selected: option.id === type }"
          @click="type = option.id"
        >
          <m3e-icon :name="option.icon"></m3e-icon>{{ t(option.labelKey) }}
        </button>
      </div>

      <section class="form-group">
        <label class="field">
          <span>{{ t("editor.name") }}</span>
          <input v-model="payload.name" required :autofocus="isNew" />
        </label>
        <div class="field-pair">
          <label class="field">
            <span>{{ t("item.folder") }}</span>
            <select v-model="folderId">
              <option value="">{{ t("item.noFolder") }}</option>
              <option v-for="folder in vault.folders" :key="folder.id" :value="folder.id">{{ folder.name }}</option>
            </select>
          </label>
          <label class="switch-field">
            <input v-model="favorite" type="checkbox" />
            <m3e-icon name="star" :filled="favorite"></m3e-icon>
            <span>{{ t("item.favorite") }}</span>
          </label>
        </div>
      </section>

      <section v-if="fields.length" class="form-group">
        <template v-for="field in fields" :key="field.path">
          <label v-if="field.kind === 'boolean'" class="switch-field">
            <input type="checkbox" :checked="Boolean(value(field))" @change="update(field, ($event.target as HTMLInputElement).checked)" />
            <span>{{ t(field.labelKey) }}</span>
          </label>
          <label v-else class="field">
            <span>{{ t(field.labelKey) }}</span>
            <select v-if="field.kind === 'select'" :value="value(field)" @change="update(field, ($event.target as HTMLSelectElement).value)">
              <option v-if="!field.defaultValue" value=""></option>
              <option v-for="option in selectOptions(field, String(value(field)))" :key="option" :value="option">{{ optionLabel(field, option) }}</option>
            </select>
            <textarea
              v-else-if="field.kind.includes('multiline')"
              :value="String(value(field))"
              rows="4"
              :class="{ mono: field.mono }"
              spellcheck="false"
              @input="update(field, ($event.target as HTMLTextAreaElement).value)"
            ></textarea>
            <div v-else-if="field.kind === 'secret'" class="input-row">
              <PasswordInput :model-value="String(value(field))" mono autocomplete="new-password" @update:model-value="update(field, $event)" />
              <m3e-icon-button v-if="field.generate" type="button" :aria-label="t('editor.generate')" :title="t('editor.generate')" @click="generate(field)">
                <m3e-icon name="casino"></m3e-icon>
              </m3e-icon-button>
            </div>
            <input
              v-else
              :value="String(value(field))"
              :class="{ mono: field.mono || field.kind === 'totp' }"
              :placeholder="field.kind === 'totp' ? 'otpauth://… / BASE32' : ''"
              spellcheck="false"
              @input="update(field, ($event.target as HTMLInputElement).value)"
            />
          </label>
        </template>
      </section>

      <section v-if="showUris" class="form-group">
        <h3 class="group-title">{{ t("item.websites") }}</h3>
        <div v-for="(_, index) in payload.uris" :key="index" class="input-row">
          <input v-model="payload.uris![index]" placeholder="https://" inputmode="url" />
          <m3e-icon-button type="button" :aria-label="t('common.remove')" @click="payload.uris!.splice(index, 1)"><m3e-icon name="remove_circle_outline"></m3e-icon></m3e-icon-button>
        </div>
        <m3e-button variant="text" type="button" @click="payload.uris!.push('')"><m3e-icon slot="icon" name="add_link"></m3e-icon>{{ t("editor.addWebsite") }}</m3e-button>
      </section>

      <section class="form-group">
        <h3 class="group-title">{{ t("item.notes") }}</h3>
        <textarea v-model="payload.notes" rows="5"></textarea>
      </section>

      <section class="form-group">
        <h3 class="group-title">{{ t("item.customFields") }}</h3>
        <div v-for="(custom, index) in payload.fields" :key="index" class="custom-row">
          <input v-model="custom.name" :placeholder="t('editor.fieldName')" />
          <input v-model="custom.value" :type="custom.hidden ? 'password' : 'text'" :placeholder="t('editor.fieldValue')" />
          <m3e-icon-button type="button" :aria-label="t('editor.hiddenField')" :title="t('editor.hiddenField')" @click="custom.hidden = !custom.hidden">
            <m3e-icon :name="custom.hidden ? 'visibility_off' : 'visibility'"></m3e-icon>
          </m3e-icon-button>
          <m3e-icon-button type="button" :aria-label="t('common.remove')" @click="payload.fields!.splice(index, 1)"><m3e-icon name="remove_circle_outline"></m3e-icon></m3e-icon-button>
        </div>
        <m3e-button variant="text" type="button" @click="payload.fields!.push({ name: '', value: '', hidden: false })">
          <m3e-icon slot="icon" name="add"></m3e-icon>{{ t("editor.addField") }}
        </m3e-button>
      </section>

      <p v-if="error" class="form-error" role="alert">{{ error }}</p>
      <button type="submit" hidden tabindex="-1"></button>
    </div>
  </form>
</template>
