# Monica Cloud Server

Rust server for Monica Cloud. It follows vaultwarden's architecture — one binary, environment
configuration, diesel models with embedded migrations, SQLite by default, the WebUI served from a
folder — but speaks Monica's own sync protocol ([`docs/protocol.zh-CN.md`](../docs/protocol.zh-CN.md)).

The server is zero-knowledge: it stores the client-side `authHash` (hashed again with Argon2id), the
wrapped account key and per-item ciphertext. It never receives the master password or any plaintext.

## Layout

```text
src/
├── main.rs            startup, graceful shutdown
├── config.rs          env config + runtime settings (admin panel overrides)
├── error.rs           JSON error envelope, `err!` macro, `ApiJson` extractor
├── auth.rs            JWT access tokens, `Auth` / `AdminAuth` guards, client IP
├── crypto.rs          Argon2id (bounded concurrency, never on a DB connection), random tokens, digests
├── storage.rs         blob store for attachments and Send files (tmp → fsync → rename)
├── ratelimit.rs       per-IP login limiter (IPv6 per /64; client IP only from TRUSTED_PROXIES)
├── jobs.rs            hourly purge of expired Sends, old events, tombstones, stale uploads, idle devices
├── db/
│   ├── schema.rs      diesel tables (mirror migrations/sqlite)
│   └── models/        user, device, folder, item, attachment, send, tombstone, event, setting
└── api/
    ├── identity.rs    config, prelogin, register, token (password/refresh), logout
    ├── core/          accounts, sync, items, folders, attachments, sends, devices, events
    │                  (JSON responses gzip/br compressed; downloads are ciphertext and are not)
    ├── admin.rs       users, settings, diagnostics, audit log (owner/admin role)
    └── web.rs         WebUI static files (precompressed .br/.gz, SPA fallback except /assets), CSP, /alive
```

Framework note: vaultwarden uses Rocket; this server uses axum (tokio-native, plain streaming request
bodies for uploads). Rocket would work too; swapping the HTTP layer would not touch `db/` or the protocol.

## Run

```bash
cargo run                      # http://127.0.0.1:8000, data in ./data
cp .env.example .env          # optional configuration
```

Serve the WebUI from the same process:

```bash
(cd ../webui && npm ci && npm run build)
WEB_VAULT_FOLDER=../webui/dist cargo run
```

Or build the single-container image from the repository root: `docker build -t monica-cloud .`
and run it with `-v monica-data:/data -p 8000:80`.

## Checks

```bash
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
# full protocol test against a running server (from webui/):
MONICA_URL=http://127.0.0.1:8000 node scripts/e2e.ts
```

Use a throw-away `DATA_FOLDER` for the end-to-end test; it registers accounts. Set
`LOGIN_RATELIMIT_PER_MINUTE=200` for that server, the test logs in many times.

## Backups

Stop the server (or use `sqlite3 db.sqlite3 ".backup ..."`) and copy `DATA_FOLDER`. It holds the
database, `jwt.key` and `blobs/`. Everything sensitive in it is client ciphertext.
