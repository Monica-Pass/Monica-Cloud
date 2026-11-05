<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref, watch } from "vue";
import { t } from "../i18n";
import { otpKind, parseTotp, secondsRemaining, totpCode } from "../lib/totp";
import { copy } from "../lib/ui";

const props = defineProps<{ secret: string; digits?: number; period?: number; algorithm?: string; otpType?: string }>();
const kind = computed(() => otpKind(props.secret, props.otpType));
const params = computed(() =>
  parseTotp(props.secret, { digits: props.digits, period: props.period, algorithm: props.algorithm, otpType: props.otpType })
);
const code = ref("");
const now = ref(Date.now());
let timer: number | undefined;
/** Time step the shown code belongs to; the HMAC only runs when it changes. */
let step = -1;
let generation = 0;

const remaining = computed(() => (params.value ? secondsRemaining(params.value.period, now.value) : 0));
const progress = computed(() => (params.value ? remaining.value / params.value.period : 0));
const pretty = computed(() => {
  const value = code.value;
  if (value.length === 6) return `${value.slice(0, 3)} ${value.slice(3)}`;
  if (value.length === 8) return `${value.slice(0, 4)} ${value.slice(4)}`;
  return value;
});

async function tick() {
  now.value = Date.now();
  const current = params.value;
  if (!current) {
    code.value = "";
    return;
  }
  const nextStep = Math.floor(now.value / 1000 / current.period);
  if (nextStep === step) return;
  step = nextStep;
  const mine = ++generation;
  const value = await totpCode(current, now.value);
  // A newer secret or time step may have finished first.
  if (mine === generation) code.value = value;
}

watch(params, () => {
  step = -1;
  void tick();
});
onMounted(() => {
  void tick();
  timer = window.setInterval(tick, 1000);
});
onUnmounted(() => window.clearInterval(timer));
</script>

<template>
  <div v-if="params" class="totp">
    <svg class="totp-ring" viewBox="0 0 36 36" :class="{ urgent: remaining <= 5 }" aria-hidden="true">
      <circle cx="18" cy="18" r="15" class="track" />
      <circle cx="18" cy="18" r="15" class="bar" :style="{ strokeDashoffset: `${94.25 * (1 - progress)}` }" />
      <text x="18" y="22" text-anchor="middle">{{ remaining }}</text>
    </svg>
    <span class="totp-code mono" :aria-label="code">{{ pretty }}</span>
    <m3e-icon-button :aria-label="t('common.copy')" :disabled="!code" @click="copy(code)"><m3e-icon name="content_copy"></m3e-icon></m3e-icon-button>
  </div>
  <span v-else-if="kind !== 'TOTP' && kind !== 'STEAM'" class="muted">{{ t("item.otpUnsupported", { type: kind }) }}</span>
  <span v-else class="muted">{{ t("item.totpInvalid") }}</span>
</template>
