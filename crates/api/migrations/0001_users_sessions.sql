CREATE TABLE users (
    id            uuid PRIMARY KEY DEFAULT uuidv7(),
    -- Stored lowercase (see auth::normalize_username).
    username      text NOT NULL UNIQUE CHECK (username = lower(username)),
    password_hash text NOT NULL,
    is_admin      boolean NOT NULL DEFAULT false,
    -- UI preferences (theme, language…), owned by the frontend.
    prefs         jsonb NOT NULL DEFAULT '{}',
    created_at    timestamptz NOT NULL DEFAULT now()
);

CREATE TABLE sessions (
    id          uuid PRIMARY KEY DEFAULT uuidv7(),
    user_id     uuid NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    created_at  timestamptz NOT NULL DEFAULT now(),
    expires_at  timestamptz NOT NULL,
    revoked_at  timestamptz
);

CREATE INDEX sessions_user_id ON sessions (user_id);
