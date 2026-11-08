-- OpenAI-compatible upstreams. Clients address models as "<provider name>/<model>".
CREATE TABLE llm_providers (
    id uuid PRIMARY KEY,
    name text NOT NULL UNIQUE CHECK (name ~ '^[a-z0-9][a-z0-9_-]{0,31}$'),
    base_url text NOT NULL,
    -- Sealed with the master key, bound to the row id (Keys::encrypt_secret, context "llm-provider:<id>").
    api_key bytea,
    enabled boolean NOT NULL DEFAULT true,
    created_at timestamptz NOT NULL DEFAULT now()
);

-- Bearer tokens for the /v1 machine API. Only a SHA-256 of the token is stored.
CREATE TABLE api_keys (
    id uuid PRIMARY KEY DEFAULT uuidv7(),
    user_id uuid NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    name text NOT NULL,
    token_hash bytea NOT NULL UNIQUE,
    created_at timestamptz NOT NULL DEFAULT now(),
    last_used_at timestamptz,
    revoked_at timestamptz
);
CREATE INDEX api_keys_user_id ON api_keys (user_id);

CREATE TABLE llm_usage (
    id uuid PRIMARY KEY DEFAULT uuidv7(),
    user_id uuid REFERENCES users (id) ON DELETE SET NULL,
    api_key_id uuid REFERENCES api_keys (id) ON DELETE SET NULL,
    provider_id uuid REFERENCES llm_providers (id) ON DELETE SET NULL,
    provider text NOT NULL,
    model text NOT NULL,
    endpoint text NOT NULL,
    status integer NOT NULL,
    prompt_tokens integer,
    completion_tokens integer,
    duration_ms integer NOT NULL,
    created_at timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX llm_usage_created_at ON llm_usage (created_at);
