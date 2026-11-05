/**
 * Web Storage access that never throws. Reading `window.localStorage` itself throws in some
 * browsers when site data is blocked, and writes can fail in private mode or when full.
 */

type Area = "local" | "session";

function area(kind: Area): Storage | null {
  try {
    return kind === "local" ? window.localStorage : window.sessionStorage;
  } catch {
    return null;
  }
}

export function readStorage(kind: Area, key: string): string | null {
  try {
    return area(kind)?.getItem(key) ?? null;
  } catch {
    return null;
  }
}

export function writeStorage(kind: Area, key: string, value: string) {
  try {
    area(kind)?.setItem(key, value);
  } catch {
    /* storage unavailable: the value only lives for this page */
  }
}

export function removeStorage(kind: Area, key: string) {
  try {
    area(kind)?.removeItem(key);
  } catch {
    /* ignore */
  }
}
