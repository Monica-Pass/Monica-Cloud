/** Minimal hash router: the server falls back to index.html, but hash routes keep share links simple. */

import { ref } from "vue";

export type Route =
  | { name: "vault"; filter: string; folderId?: string }
  | { name: "send" }
  | { name: "devices" }
  | { name: "activity" }
  | { name: "settings" }
  | { name: "admin-users" }
  | { name: "admin-server" }
  | { name: "admin-activity" }
  | { name: "receive"; accessId: string; key: string };

function decode(part: string): string {
  try {
    return decodeURIComponent(part);
  } catch {
    return part; // a malformed escape in a pasted link must not crash the app
  }
}

export function parseRoute(hash: string): Route {
  const parts = hash.replace(/^#\/?/, "").split("/").filter(Boolean).map(decode);
  switch (parts[0]) {
    case "send":
      return parts.length >= 3 ? { name: "receive", accessId: parts[1], key: parts[2] } : { name: "send" };
    case "sends":
      return { name: "send" };
    case "devices":
      return { name: "devices" };
    case "activity":
      return { name: "activity" };
    case "settings":
      return { name: "settings" };
    case "admin":
      if (parts[1] === "server") return { name: "admin-server" };
      if (parts[1] === "activity") return { name: "admin-activity" };
      return { name: "admin-users" };
    case "vault":
      if (parts[1] === "folder" && parts[2]) return { name: "vault", filter: "folder", folderId: parts[2] };
      return { name: "vault", filter: parts[1] ?? "all" };
    default:
      return { name: "vault", filter: "all" };
  }
}

export const route = ref<Route>(parseRoute(location.hash));

window.addEventListener("hashchange", () => {
  route.value = parseRoute(location.hash);
});

export function go(path: string) {
  location.hash = `#/${path}`;
}

export function routeKey(r: Route): string {
  if (r.name === "vault") return r.filter === "folder" ? `vault/folder/${r.folderId}` : `vault/${r.filter}`;
  return r.name;
}

/** Global vault search, typed in the top bar. */
export const searchQuery = ref("");
