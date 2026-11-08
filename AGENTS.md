# AGENTS.md

Guidance for coding agents working in this repository. Read [README.md](./README.md) first for the stack.

## Hard rules

- **No Python.** Not in the app, the build, scripts or containers we maintain. Third-party images (e.g. inference) are fine as long as we don't ship Python code.
- **x86_64 and arm64.** Every crate, image and build step must work on both. Prefer pure-Rust crates and `rustls` (no OpenSSL). Flag anything that is x86-only.
- **Postgres is the source of truth, except for memory.** Typesense must be rebuildable from Postgres. Outside memory, never store anything only in Valkey or Typesense.
- **Typesense writes go through the outbox**: insert the row and the outbox event in the same Postgres transaction; a worker pushes it to Typesense.
- **Memory (taOSmd rewrite) is Valkey-first, with an async Postgres backup** (accepted risk: memories not yet persisted are lost if Valkey is wiped):
  - Write: store the record in Valkey (no TTL) and `XADD` a `memory:persist` event in the same `MULTI`. Workers upsert it into Postgres, then index it in Typesense / compute embeddings, then `XACK`.
  - Upserts are keyed by UUID and only apply when `version` is newer, so retries and replays are harmless.
  - Read: Valkey first; on a miss, load from Postgres and write it back to Valkey.
  - Valkey must run with AOF on a persistent volume and `maxmemory-policy volatile-lru`: memory keys have no TTL and are never evicted; only TTL'd cache keys are.
- **Valkey jobs must be idempotent**, because Streams deliver at least once.
- **Object storage goes through `object_store`**, never a Garage- or AWS-specific client.
- **GraphQL is for the UI; REST is for machines**: the OpenAI-compatible `/v1` (SSE), MCP, webhooks, uploads.
- **Extism plugins don't do I/O loops.** Rust owns sockets, webhooks and polling; plugins parse and format, and reach the network only through allowlisted host functions.
- **Never expose CDP (`browser:9222`) or the KasmVNC viewer publicly.** Keep them on the internal network or localhost.

## `taOS/` is reference only

`taOS/` is the original Python project (AGPL-3.0). This project is MIT, so it must stay a clean rewrite. Read `taOS/` to understand features and behavior, then write new code from that understanding. **Do not modify it, and never copy or line-by-line translate its code, assets, themes or catalog manifests.** Its own `CLAUDE.md` / `AGENTS.md` apply to that project, not this one.

- **Dependency licenses**: Rust crates and npm packages we link must be MIT-compatible (MIT, Apache-2.0, BSD, ISC, MPL-2.0…); no GPL/AGPL libraries. Enforce with `cargo deny`. GPL/AGPL software running as a separate container (Typesense, Garage) is fine.

## IDs and secrets

- Every GraphQL object exposed to clients implements Relay `Node`. Its `id` is an encrypted, expiring GlobalID (`gid://rust-agents/<Type>/<uuid>` + purpose + expiry, XChaCha20-Poly1305). Never expose raw database IDs. Expired IDs return the `ID_EXPIRED` error code.
- Every `Node` also exposes `cacheKey` (HMAC-SHA256 of `<Type>:<uuid>` with the `cache-key` subkey). Relay identifies records by `cacheKey` via `getDataID`, so **every Node selection must include `cacheKey`**; never use `id` for client-side identity.
- Use UUIDv7 primary keys wherever possible.
- Decoding an ID only identifies an object; resolvers must still check that the caller may access it.
- Derive keys from the master key with HKDF, one subkey per purpose, with a version byte. Never reuse a key across purposes and never store the master key in the database.

## Planned layout

Not created yet; follow it when scaffolding:

```
crates/
  api/        axum + async-graphql, REST machine APIs
  core/       domain types, auth, channels
  gateway/    LLM router (OpenAI-compatible)
  memory/     taOSmd rewrite (embeddings, rerank, knowledge graph)
  plugins/    Extism host + host functions
  worker/     Valkey Stream consumers, outbox → Typesense
web/          Vike + React + DaisyUI frontend
deploy/       docker-compose.yml, garage-init, .env.example
```

## Conventions

- Rust: stable toolchain, `cargo fmt` + `cargo clippy -- -D warnings`, `sqlx` migrations in `crates/api/migrations` (or wherever the scaffold puts them; keep a single location).
- GraphQL is code-first (`async-graphql` derives); export the SDL and generate typed frontend hooks with `graphql-codegen`.
- Frontend: every user-facing string goes through i18next; backend errors return codes, not text. Respect `prefers-reduced-motion` with Motion. Heavy apps load lazily via dynamic `import()`.
- Tests: `testcontainers` for Postgres/Valkey/Typesense integration tests, Playwright for end-to-end tests.

## Commands

The frontend uses pnpm through corepack (`corepack pnpm …`, or `corepack enable` once).

```sh
# Backend (from the repo root; serves web/dist/client)
cargo build
cargo run -p api                      # BIND_ADDR=0.0.0.0:8080, WEB_DIR=web/dist/client
cargo fmt && cargo clippy -- -D warnings
cargo deny check                      # needs: cargo install cargo-deny

# Frontend (from web/)
corepack pnpm install
corepack pnpm dev                     # Vite dev server
corepack pnpm build                   # SPA → web/dist/client (prerendered index.html)
corepack pnpm test                    # vitest

# Full stack, for testing (from the repo root)
bin/dev                               # creates deploy/.env with fresh secrets if missing, builds, starts, waits
bin/dev logs [service] | ps | down | reset   # reset wipes data volumes, keeps .env
```

## Known gotchas

- Chrome's CDP HTTP endpoints reject a `Host` header that isn't an IP or `localhost`: send `Host: localhost`, then rewrite the returned `ws://localhost/...` URL to `browser:9222`.
- The Typesense image has no curl/wget; its health check uses bash `/dev/tcp`.
- Garage's image has no shell; `garage-init` runs `deploy/garage/init.sh` from a curl image against the v2 admin API. It is idempotent.
- Postgres keeps the password from its first start: changing `POSTGRES_PASSWORD` later needs `docker compose down -v` (wipes data) or an `ALTER USER`.
