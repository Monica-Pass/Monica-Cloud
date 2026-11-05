# Monica Cloud

Self-hosted, zero-knowledge sync server and web vault for Monica Pass — what vaultwarden is to
Bitwarden, but with Monica's own protocol. Monica apps log in with a custom server URL (like a
self-hosted Bitwarden), and items are end-to-end encrypted on each device.

```text
.
├── docs/protocol.zh-CN.md        sync protocol v1 (crypto + API), the contract for server, WebUI and apps
├── docs/ce-integration.zh-CN.md  how Monica for Android (CE) syncs with Monica Cloud
├── server/                       Rust server (axum + diesel/SQLite), vaultwarden-style layout
├── webui/                        Vue 3 + m3e web vault, decrypts in the browser
└── Dockerfile                    single image: server + built WebUI, data in /data
```

## Quick start

```bash
cd webui && npm ci && npm run build && cd ..
cd server && WEB_VAULT_FOLDER=../webui/dist cargo run
# open http://127.0.0.1:8000 — the first account becomes the server owner
```

Docker:

```bash
docker build -t monica-cloud .
docker run -d --name monica-cloud -v monica-data:/data -p 8000:80 monica-cloud
```

Put it behind HTTPS (Caddy, nginx, Traefik) before using it from other devices: WebCrypto only
runs on secure origins, so the WebUI works on `localhost` or `https://` only.

Have the proxy pass the client address, which the login rate limit and the audit log use:

```caddyfile
cloud.example.com {
    reverse_proxy 127.0.0.1:8000 {
        header_up X-Real-IP {remote_host}
    }
}
```

With nginx, add `proxy_set_header X-Real-IP $remote_addr;`; Traefik sets the header itself. The
header is only believed from `TRUSTED_PROXIES` (loopback and private networks by default), see
`server/.env.example`.

## What works

| Area | Status |
| --- | --- |
| Accounts: register (first user = owner), PBKDF2 prelogin, login, refresh-token rotation, logout | done |
| Zero-knowledge keys: authHash + Argon2id, wrapped account key, per-item keys, password change without re-encryption | done |
| Items: 10 Monica types, folders, favorites, archive, trash, bulk actions, bulk import for first upload | done |
| Sync: full and incremental (`since` + tombstones), per-user monotonic revisions, optimistic concurrency (409 + server copy) | done |
| Attachments: chunked AES-GCM files, quota, streaming upload/download | done |
| Send: text/file, password, open limit, expiry, public receive page with key in the URL fragment | done |
| Devices, audit log, admin console (users, roles, quotas, policies, diagnostics) | done |
| WebUI: three-pane vault, live TOTP, editor, Send, settings, admin, zh-CN/en, light/dark, phone layout | done |
| Monica for Android (CE): custom-server login, two-way sync of the Monica-local vault, background sync | done on CE branch `feat/monica-cloud` — see `docs/ce-integration.zh-CN.md` |
| Attachments in the Android app (both directions) | done |
| Send in the Android app | next |
| Two-factor login, organizations/sharing, push notifications, Argon2id KDF | planned (protocol §6) |

## Tests

```bash
cd server && cargo clippy --all-targets -- -D warnings && cargo test
cd webui && npm run typecheck
# protocol end-to-end against a throw-away server:
cd server && DATA_FOLDER=/tmp/mc LOGIN_RATELIMIT_PER_MINUTE=200 cargo run &
cd webui && MONICA_URL=http://127.0.0.1:8000 npm run e2e
```

## License

AGPL-3.0. See `LICENSE`.
