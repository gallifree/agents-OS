-- AI agents: a handle (@name in channels), a gateway model ("<provider>/<model>") and a system prompt.
CREATE TABLE agents (
    id uuid PRIMARY KEY DEFAULT uuidv7(),
    name text NOT NULL UNIQUE CHECK (name ~ '^[a-z0-9][a-z0-9_-]{0,31}$'),
    model text NOT NULL,
    system_prompt text NOT NULL DEFAULT '',
    created_by uuid REFERENCES users (id) ON DELETE SET NULL,
    created_at timestamptz NOT NULL DEFAULT now()
);

-- Agent tokens are API keys bound to an agent: limited to its model, usage counted for it.
ALTER TABLE api_keys ADD COLUMN agent_id uuid REFERENCES agents (id) ON DELETE CASCADE;
ALTER TABLE llm_usage ADD COLUMN agent_id uuid REFERENCES agents (id) ON DELETE SET NULL;

-- Shared with every user, or private to its creator.
CREATE TABLE channels (
    id uuid PRIMARY KEY DEFAULT uuidv7(),
    name text NOT NULL CHECK (length(name) BETWEEN 1 AND 64),
    private boolean NOT NULL DEFAULT false,
    created_by uuid NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    created_at timestamptz NOT NULL DEFAULT now()
);

CREATE TABLE channel_agents (
    channel_id uuid NOT NULL REFERENCES channels (id) ON DELETE CASCADE,
    agent_id uuid NOT NULL REFERENCES agents (id) ON DELETE CASCADE,
    PRIMARY KEY (channel_id, agent_id)
);

CREATE TABLE messages (
    id uuid PRIMARY KEY DEFAULT uuidv7(),
    channel_id uuid NOT NULL REFERENCES channels (id) ON DELETE CASCADE,
    author_user_id uuid REFERENCES users (id) ON DELETE SET NULL,
    author_agent_id uuid REFERENCES agents (id) ON DELETE SET NULL,
    -- Kept so messages still show who wrote them after the author is deleted.
    author_name text NOT NULL,
    author_is_agent boolean NOT NULL DEFAULT false,
    content text NOT NULL,
    -- An agent reply that failed: content holds the reason.
    is_error boolean NOT NULL DEFAULT false,
    -- The user message an agent reply answers; with the agent, makes reply jobs idempotent.
    reply_to uuid REFERENCES messages (id) ON DELETE SET NULL,
    created_at timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX messages_channel_id ON messages (channel_id, id);
CREATE UNIQUE INDEX messages_one_reply_per_agent ON messages (reply_to, author_agent_id);
