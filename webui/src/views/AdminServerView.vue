<script setup lang="ts">
import { onMounted, ref } from "vue";
import { t } from "../i18n";
import type { Diagnostics, RuntimeSettings } from "../lib/client";
import { formatBytes, formatDateTime, formatDuration } from "../lib/format";
import { client, serverConfig } from "../lib/session";
import { toast, toastError } from "../lib/ui";

const settings = ref<RuntimeSettings | null>(null);
const quotaMb = ref<number | null>(null);
const diagnostics = ref<Diagnostics | null>(null);
const saving = ref(false);

async function load() {
  try {
    const [s, d] = await Promise.all([client.adminSettings(), client.adminDiagnostics()]);
    settings.value = s;
    quotaMb.value = s.defaultQuotaBytes ? Math.round(s.defaultQuotaBytes / 1048576) : null;
    diagnostics.value = d;
  } catch (e) {
    toastError(e);
  }
}

async function save() {
  if (!settings.value) return;
  // `v-model.number` leaves "" in a cleared field, which the server rejects.
  const retention = Number(settings.value.eventRetentionDays);
  if (!Number.isInteger(retention) || retention < 1) return toast(t("admin.retentionInvalid"), "error");
  const mb = Number(quotaMb.value);
  if (String(quotaMb.value ?? "") !== "" && (!Number.isFinite(mb) || mb < 0)) return toast(t("admin.quotaInvalid"), "error");
  saving.value = true;
  try {
    const quota = mb > 0 ? Math.round(mb * 1048576) : null;
    settings.value = await client.adminSaveSettings({ ...settings.value, eventRetentionDays: retention, defaultQuotaBytes: quota });
    serverConfig.value = await client.config();
    toast(t("settings.saved"));
  } catch (e) {
    toastError(e);
  } finally {
    saving.value = false;
  }
}

onMounted(load);
</script>

<template>
  <div class="page">
    <header class="page-header">
      <div>
        <h1>{{ t("nav.adminServer") }}</h1>
        <p class="muted">{{ diagnostics ? `Monica Cloud ${diagnostics.version}` : "" }}</p>
      </div>
      <m3e-icon-button :aria-label="t('common.refresh')" @click="load"><m3e-icon name="refresh"></m3e-icon></m3e-icon-button>
    </header>

    <div v-if="diagnostics" class="metric-grid">
      <div class="metric"><m3e-icon name="group"></m3e-icon><strong>{{ diagnostics.counts.users }}</strong><small>{{ t("admin.metricUsers") }}</small></div>
      <div class="metric"><m3e-icon name="key"></m3e-icon><strong>{{ diagnostics.counts.items }}</strong><small>{{ t("admin.metricItems") }}</small></div>
      <div class="metric"><m3e-icon name="attach_file"></m3e-icon><strong>{{ formatBytes(diagnostics.storage.attachmentBytes) }}</strong><small>{{ t("admin.metricFiles", { count: diagnostics.storage.attachments }) }}</small></div>
      <div class="metric"><m3e-icon name="database"></m3e-icon><strong>{{ formatBytes(diagnostics.database.sizeBytes) }}</strong><small>{{ t("admin.metricDatabase") }}</small></div>
      <div class="metric"><m3e-icon name="timer"></m3e-icon><strong>{{ formatDuration(diagnostics.uptimeSeconds) }}</strong><small>{{ t("admin.metricUptime") }}</small></div>
      <div class="metric"><m3e-icon name="send"></m3e-icon><strong>{{ diagnostics.counts.sends }}</strong><small>{{ t("admin.metricSends") }}</small></div>
    </div>

    <div class="settings-grid">
      <section v-if="settings" class="settings-card">
        <h2>{{ t("admin.policies") }}</h2>
        <form class="form-group" @submit.prevent="save">
          <label class="switch-field"><input v-model="settings.signupsAllowed" type="checkbox" /><span>{{ t("admin.signupsAllowed") }}</span></label>
          <label class="switch-field"><input v-model="settings.sendsAllowed" type="checkbox" /><span>{{ t("admin.sendsAllowed") }}</span></label>
          <div class="field-pair">
            <label class="field"><span>{{ t("admin.defaultQuota") }}</span><input v-model.number="quotaMb" type="number" min="0" :placeholder="t('send.unlimited')" /></label>
            <label class="field"><span>{{ t("admin.eventRetention") }}</span><input v-model.number="settings.eventRetentionDays" type="number" min="1" /></label>
          </div>
          <m3e-button variant="filled" type="submit" :disabled="saving">{{ t("common.save") }}</m3e-button>
          <button type="submit" hidden tabindex="-1"></button>
        </form>
      </section>

      <section v-if="diagnostics" class="settings-card">
        <h2>{{ t("admin.diagnostics") }}</h2>
        <dl class="facts">
          <div><dt>{{ t("admin.serverTime") }}</dt><dd>{{ formatDateTime(diagnostics.serverTime) }}</dd></div>
          <div><dt>{{ t("admin.database") }}</dt><dd class="mono">{{ diagnostics.database.backend }} · {{ diagnostics.database.path }}</dd></div>
          <div><dt>{{ t("admin.dataFolder") }}</dt><dd class="mono">{{ diagnostics.storage.dataFolder }}</dd></div>
          <div><dt>{{ t("admin.webVault") }}</dt><dd class="mono">{{ diagnostics.webVault.folder }}</dd></div>
          <div v-for="(value, key) in diagnostics.config" :key="key">
            <dt class="mono">{{ key }}</dt>
            <dd class="mono">{{ Array.isArray(value) ? value.join(", ") || "—" : value ?? "—" }}</dd>
          </div>
        </dl>
      </section>
    </div>
  </div>
</template>
