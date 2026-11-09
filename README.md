# rust-agents

A self-hosted AI agent platform inspired by [taOS](./taOS), rewritten from scratch with **no Python**: a Rust backend, a React web desktop, and WASM plugins, all shipped with Docker Compose for **x86_64 and arm64**.

> **Status:** milestone 4 done: agents (model + instructions), channels (shared or private) where agents answer live, agent tokens on `/v1`. Before it: LLM gateway, users and sessions. Next: milestone 5 (memory). See [Still open](#still-open).

## Stack

| Layer | Choice |
|---|---|
| Backend | Rust, `axum` + `async-graphql` (GraphQL + WebSocket subscriptions) |
| Machine APIs | REST in `axum`: OpenAI-compatible `/v1` (SSE), MCP (`rmcp`), webhooks |
| LLM gateway | In-house Rust router (providers, keys, budgets, usage, fallbacks) |
| Agent runtime | Native Rust agent loop |
| Memory | Rust rewrite of taOSmd: `fastembed` / `candle` embeddings + rerank, knowledge graph in Postgres |
| Plugins | [Extism](https://extism.org) (WASM): tools, hooks, channel connectors |
| Frontend | Vike (SPA mode) + React + Motion + i18next + DaisyUI |

## Services (Docker Compose)

| Service | Role |
|---|---|
| `app` | Rust backend; also serves the built frontend |
| `postgres` | Source of truth (`sqlx` migrations), except memory, where it's the async backup |
| `typesense` | Keyword + vector hybrid search, fed from Postgres via an outbox |
| `valkey` | Primary memory store, cache, job queues (Streams), pub/sub fan-out. AOF on a persistent volume |
| `garage` + `garage-init` | S3 asset storage; AWS S3 or a compatible service can replace it |
| `browser` | Compose profile: `headless` (chromedp/headless-shell) or `chromium` (linuxserver/chromium, viewable over KasmVNC) |
| `inference` | Optional: Ollama / llama.cpp, or run on the host (Mac / Metal) |

```sh
cd deploy
cp .env.example .env   # fill in the secrets (commands are in the file)
docker compose up -d --build
# → http://localhost:8080
```

## Channels

Channels live natively in Postgres. External connectors (Telegram, Discord, Slack…) will come later as Extism plugins; Rust does the network I/O and plugins only translate messages. Matrix (Tuwunel) may come later as an optional connector.

## Scope

The first version covers **all features of taOS** (see `taOS/`), delivered in milestones.

## Auth, IDs and secrets

- **Master key**: provided through an env var / Docker secret, never stored in the database. Each purpose gets its own subkey derived with HKDF (`relay-id`, `session`, `secrets`…), with a version byte for rotation.
- **GraphQL Relay**: `Node` interface, connections, and opaque global IDs.
- **Encrypted, expiring global IDs**: like Rails' GlobalID / `signed_id`, but encrypted. The payload is `gid://rust-agents/<Type>/<uuid>` + purpose + expiry, encrypted with randomized AEAD (XChaCha20-Poly1305). An expired ID returns an `ID_EXPIRED` error and the client refetches the parent query. An ID hides the type and the database key, but never grants access: resolvers still check permissions.
- **`cacheKey`**: every `Node` also exposes `cacheKey = base64url(HMAC-SHA256(cache-key subkey, "<Type>:<uuid>"))`. It is stable forever and can't be decoded or used to fetch anything. The Relay environment identifies records by `cacheKey` (`getDataID`), so IDs can change on every response without breaking the cache.
- **Primary keys**: UUIDv7 wherever possible.
- **Capability IDs** (share links, invites, downloads): same format as global IDs, with their own purpose and a custom expiry.
- **Sessions**: an encrypted, purpose-bound and expiring token (randomized AEAD) wrapping the user's GlobalID and a session id. A session row in Postgres allows revocation.
- **Agent tokens**: scoped, revocable, stored hashed.
- **API keys** (`/v1`): `ra_` + 244 random bits, stored as SHA-256, revocable, owned by a user.
- **Secrets at rest** (provider API keys…): encrypted with the `secrets` subkey.
- **Docker socket**: used for app installs and per-profile browsers, always behind a socket proxy.

## LLM gateway

Admins add providers in the **Models** app: any OpenAI-compatible API (OpenAI, Anthropic, OpenRouter, Ollama, vLLM, llama.cpp…). Programs call `POST /v1/chat/completions` (streaming or not), `POST /v1/embeddings` and `GET /v1/models` with `Authorization: Bearer ra_…`, and name models `"<provider>/<model>"` (e.g. `openai/gpt-4o`). Token usage is recorded per user and model, streams included (the gateway asks upstreams for `include_usage`).

## Agents and channels

Agents have a handle (`@name`), a gateway model and instructions. In a channel with one agent, it answers every message; with several, only the `@mentioned` ones answer. Posting a message queues one reply job per agent on the `agents:replies` Valkey Stream; every backend instance runs a worker in the consumer group, which sends the last 30 messages to the model and posts the answer (or the failure reason). Messages, typing indicators and replies reach the browser over GraphQL subscriptions. An **agent token** is a `/v1` key that acts as the agent: limited to its model, usage counted for it.

## Still open

- LLM gateway: budgets, fallbacks between providers, per-key model allowlists (agent tokens are limited to one model)
- Chat: Markdown rendering, threads, reactions, attachments, search (Typesense via the outbox), agent tools; channel members other than "everyone" / "only me"; streamed agent replies
- Evaluation harness for the memory system (LongMemEval)
- Real-condition tests of Valkey memory growth on 4–8 GB boards; add a recent-in-Valkey / older-in-Postgres split only if needed

## License

MIT. This is a clean rewrite: no code, assets or manifests are copied from taOS / taOSmd (AGPL-3.0).

## Reference

`taOS/` is a read-only copy of the original Python project, used as a functional reference only.
