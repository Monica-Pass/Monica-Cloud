/** App-wide toasts and confirmation dialogs. */

import { reactive } from "vue";
import { t, type MessageKey } from "../i18n";
import { ApiError } from "./client";

export interface Toast {
  id: number;
  message: string;
  tone: "info" | "error";
}

export const toasts = reactive<Toast[]>([]);
let nextId = 1;

export function toast(message: string, tone: Toast["tone"] = "info") {
  const id = nextId++;
  toasts.push({ id, message, tone });
  setTimeout(() => {
    const index = toasts.findIndex((x) => x.id === id);
    if (index >= 0) toasts.splice(index, 1);
  }, tone === "error" ? 6000 : 3000);
}

export function errorMessage(error: unknown): string {
  if (error instanceof ApiError) {
    if (error.status === 0 || error.code === "http") return t("error.network");
    if (error.status === 429) return t("error.rateLimited");
    return error.message;
  }
  if (error instanceof TypeError) return t("error.network");
  return error instanceof Error ? error.message : String(error);
}

export function toastError(error: unknown) {
  toast(errorMessage(error), "error");
}

let clipboardTimer: number | undefined;

/** Copy and clear the clipboard after 60 s if it still holds our value. */
export async function copy(value: string, labelKey: MessageKey = "toast.copied") {
  try {
    await navigator.clipboard.writeText(value);
    toast(t(labelKey));
    window.clearTimeout(clipboardTimer);
    clipboardTimer = window.setTimeout(async () => {
      try {
        if ((await navigator.clipboard.readText()) === value) await navigator.clipboard.writeText("");
      } catch {
        /* reading the clipboard needs permission; skip */
      }
    }, 60_000);
  } catch {
    toast(t("toast.copyFailed"), "error");
  }
}

export interface ConfirmRequest {
  title: string;
  message: string;
  confirmLabel: string;
  danger: boolean;
  resolve: (ok: boolean) => void;
}

export const confirmState = reactive<{ request: ConfirmRequest | null }>({ request: null });

export function confirm(title: string, message: string, options: { confirmLabel?: string; danger?: boolean } = {}): Promise<boolean> {
  // Only one dialog can be shown; a newer request cancels the older one instead of leaving it pending forever.
  confirmState.request?.resolve(false);
  return new Promise((resolve) => {
    confirmState.request = {
      title,
      message,
      confirmLabel: options.confirmLabel ?? t("common.confirm"),
      danger: options.danger ?? false,
      resolve
    };
  });
}
