import { computed } from "vue";
import { locale, t, type MessageKey } from "../i18n";

/** Name order for lists: locale-aware, case-insensitive, and "Item 9" before "Item 10". */
export const nameCollator = computed(() => new Intl.Collator(locale.value, { numeric: true, sensitivity: "base" }));

export function formatBytes(value: number | null | undefined): string {
  if (value == null) return "—";
  if (value < 1024) return `${value} B`;
  const units = ["KB", "MB", "GB", "TB"];
  let size = value / 1024;
  let index = 0;
  while (size >= 1024 && index < units.length - 1) {
    size /= 1024;
    index += 1;
  }
  return `${size.toFixed(size >= 10 ? 1 : 2)} ${units[index]}`;
}

export function formatRelative(value: string | Date | null | undefined): string {
  if (!value) return "—";
  const date = typeof value === "string" ? new Date(value) : value;
  const diffMs = Date.now() - date.getTime();
  const seconds = Math.abs(diffMs) / 1000;
  if (seconds < 45) return t("relative.now");
  let unit: MessageKey = "relative.minute";
  let amount = Math.round(seconds / 60);
  if (amount >= 60) {
    const hours = Math.round(amount / 60);
    if (hours < 48) {
      unit = "relative.hour";
      amount = hours;
    } else {
      unit = "relative.day";
      amount = Math.round(hours / 24);
    }
  }
  const text = t(unit, { value: amount });
  return t(diffMs < 0 ? "relative.future" : "relative.past", { value: text });
}

// Creating an Intl.DateTimeFormat is far slower than using one; long lists format many dates.
const dateTime = computed(() => new Intl.DateTimeFormat(locale.value, { dateStyle: "medium", timeStyle: "short" }));

export function formatDateTime(value: string | null | undefined): string {
  if (!value) return "—";
  return dateTime.value.format(new Date(value));
}

export function formatDuration(seconds: number): string {
  const days = Math.floor(seconds / 86400);
  const hours = Math.floor((seconds % 86400) / 3600);
  const minutes = Math.floor((seconds % 3600) / 60);
  return days ? `${days}d ${hours}h` : hours ? `${hours}h ${minutes}m` : `${minutes}m`;
}
