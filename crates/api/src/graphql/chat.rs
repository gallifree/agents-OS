//! Agents, channels and messages. Agents answer in channels through the reply jobs of `runtime`.

use async_graphql::{
    ComplexObject, Context, Enum, ID, Object, Result, SimpleObject, Subscription,
    connection::{Connection, Edge, query},
};
use chrono::{DateTime, Utc};
use crypto::Keys;
use futures::{Stream, StreamExt};
use sqlx::{FromRow, PgPool};
use uuid::Uuid;

use super::{
    ID_TTL, coded, decode_typed, internal,
    llm::{ApiKey, ApiKeyRow, CreatedApiKey, validate_handle},
    require_viewer, services,
};
use crate::{
    auth::Viewer,
    events::{self, Event},
    gateway, runtime,
    services::Services,
};

const MAX_MESSAGE_CHARS: usize = 16_000;
const MAX_PROMPT_CHARS: usize = 20_000;

// ---- Agents ---------------------------------------------------------------------------------

/// An AI agent. Every user can see agents and add them to channels; their creator and admins edit them.
#[derive(SimpleObject, Clone)]
#[graphql(complex)]
pub struct Agent {
    pub id: ID,
    pub cache_key: String,
    /// Handle used for @mentions.
    pub name: String,
    /// Gateway model, `"<provider>/<model>"`.
    pub model: String,
    pub system_prompt: String,
    pub can_edit: bool,
    pub created_at: DateTime<Utc>,
    #[graphql(skip)]
    pub uuid: Uuid,
}

#[derive(FromRow)]
struct AgentRow {
    id: Uuid,
    name: String,
    model: String,
    system_prompt: String,
    created_by: Option<Uuid>,
    created_at: DateTime<Utc>,
}

impl AgentRow {
    fn can_edit(&self, viewer: &Viewer) -> bool {
        viewer.is_admin || self.created_by == Some(viewer.user_id)
    }

    fn into_agent(self, keys: &Keys, viewer: &Viewer) -> Agent {
        Agent {
            id: ID(keys.encode_id("Agent", self.id, ID_TTL)),
            cache_key: keys.cache_key("Agent", self.id),
            can_edit: self.can_edit(viewer),
            name: self.name,
            model: self.model,
            system_prompt: self.system_prompt,
            created_at: self.created_at,
            uuid: self.id,
        }
    }
}

macro_rules! message_columns {
    () => {
        "m.id, m.author_user_id, m.author_name, m.author_is_agent, m.content, m.is_error, m.created_at"
    };
}

macro_rules! agent_columns {
    () => {
        "a.id, a.name, a.model, a.system_prompt, a.created_by, a.created_at"
    };
}

async fn agent_row(pg: &PgPool, id: Uuid) -> Result<Option<AgentRow>> {
    sqlx::query_as(concat!(
        "SELECT ",
        agent_columns!(),
        " FROM agents a WHERE a.id = $1"
    ))
    .bind(id)
    .fetch_optional(pg)
    .await
    .map_err(internal)
}

pub async fn load_agent(s: &Services, viewer: &Viewer, id: Uuid) -> Result<Option<Agent>> {
    Ok(agent_row(&s.pg, id)
        .await?
        .map(|r| r.into_agent(&s.keys, viewer)))
}

/// The agent behind `id`, if the viewer may edit it.
async fn editable_agent(s: &Services, viewer: &Viewer, id: &ID) -> Result<AgentRow> {
    let uuid = decode_typed(&s.keys, id, "Agent")?;
    match agent_row(&s.pg, uuid).await? {
        Some(row) if row.can_edit(viewer) => Ok(row),
        Some(_) => Err(coded(
            "FORBIDDEN",
            "only its creator or an admin can change this agent",
        )),
        None => Err(coded("NOT_FOUND", "no such agent")),
    }
}

#[ComplexObject]
impl Agent {
    /// The agent's `/v1` tokens. Empty for viewers who can't edit the agent.
    async fn tokens(&self, ctx: &Context<'_>) -> Result<Vec<ApiKey>> {
        if !self.can_edit {
            return Ok(Vec::new());
        }
        let s = services(ctx);
        let rows: Vec<ApiKeyRow> = sqlx::query_as(
            "SELECT id, name, created_at, last_used_at FROM api_keys
             WHERE agent_id = $1 AND revoked_at IS NULL ORDER BY id",
        )
        .bind(self.uuid)
        .fetch_all(&s.pg)
        .await
        .map_err(internal)?;
        Ok(rows
            .into_iter()
            .map(|r| ApiKey::from_row(r, &s.keys))
            .collect())
    }
}

fn validate_agent(model: &str, system_prompt: &str) -> Result<()> {
    if !model
        .split_once('/')
        .is_some_and(|(p, m)| !p.is_empty() && !m.is_empty())
    {
        return Err(coded(
            "INVALID_MODEL",
            "model must be \"<provider>/<model>\"",
        ));
    }
    if system_prompt.chars().count() > MAX_PROMPT_CHARS {
        return Err(coded("TOO_LONG", "system prompt too long"));
    }
    Ok(())
}

// ---- Channels -------------------------------------------------------------------------------

/// A conversation between users and agents. Shared with every user, or private to its creator.
#[derive(SimpleObject, Clone)]
#[graphql(complex)]
pub struct Channel {
    pub id: ID,
    pub cache_key: String,
    pub name: String,
    pub private: bool,
    pub can_edit: bool,
    pub created_at: DateTime<Utc>,
    #[graphql(skip)]
    pub uuid: Uuid,
}

#[derive(FromRow)]
struct ChannelRow {
    id: Uuid,
    name: String,
    private: bool,
    created_by: Uuid,
    created_at: DateTime<Utc>,
}

impl ChannelRow {
    fn into_channel(self, keys: &Keys, viewer: &Viewer) -> Channel {
        Channel {
            id: ID(keys.encode_id("Channel", self.id, ID_TTL)),
            cache_key: keys.cache_key("Channel", self.id),
            // Admins can manage shared channels, never read others' private ones (not visible to them).
            can_edit: viewer.is_admin || self.created_by == viewer.user_id,
            name: self.name,
            private: self.private,
            created_at: self.created_at,
            uuid: self.id,
        }
    }
}

/// The channel if the viewer may see it.
pub async fn load_channel(s: &Services, viewer: &Viewer, id: Uuid) -> Result<Option<Channel>> {
    let row: Option<ChannelRow> = sqlx::query_as(
        "SELECT c.id, c.name, c.private, c.created_by, c.created_at FROM channels c
         WHERE c.id = $1 AND (NOT c.private OR c.created_by = $2)",
    )
    .bind(id)
    .bind(viewer.user_id)
    .fetch_optional(&s.pg)
    .await
    .map_err(internal)?;
    Ok(row.map(|r| r.into_channel(&s.keys, viewer)))
}

async fn visible_channel(s: &Services, viewer: &Viewer, id: &ID) -> Result<Channel> {
    let uuid = decode_typed(&s.keys, id, "Channel")?;
    load_channel(s, viewer, uuid)
        .await?
        .ok_or_else(|| coded("NOT_FOUND", "no such channel"))
}

async fn editable_channel(s: &Services, viewer: &Viewer, id: &ID) -> Result<Channel> {
    let channel = visible_channel(s, viewer, id).await?;
    if channel.can_edit {
        Ok(channel)
    } else {
        Err(coded(
            "FORBIDDEN",
            "only its creator or an admin can change this channel",
        ))
    }
}

fn validate_channel_name(name: &str) -> Result<String> {
    let name = name.trim();
    if (1..=64).contains(&name.chars().count()) {
        Ok(name.to_owned())
    } else {
        Err(coded("INVALID_NAME", "1–64 characters"))
    }
}

/// Replaces the channel's agents.
async fn set_channel_agents(s: &Services, channel: Uuid, agent_ids: &[ID]) -> Result<()> {
    let ids = agent_ids
        .iter()
        .map(|id| decode_typed(&s.keys, id, "Agent"))
        .collect::<Result<Vec<_>>>()?;
    let mut tx = s.pg.begin().await.map_err(internal)?;
    sqlx::query("DELETE FROM channel_agents WHERE channel_id = $1")
        .bind(channel)
        .execute(&mut *tx)
        .await
        .map_err(internal)?;
    // Unknown agent ids are skipped by the join.
    sqlx::query(
        "INSERT INTO channel_agents (channel_id, agent_id)
         SELECT $1, a.id FROM agents a WHERE a.id = ANY($2) ON CONFLICT DO NOTHING",
    )
    .bind(channel)
    .bind(ids)
    .execute(&mut *tx)
    .await
    .map_err(internal)?;
    tx.commit().await.map_err(internal)
}

#[ComplexObject]
impl Channel {
    async fn agents(&self, ctx: &Context<'_>) -> Result<Vec<Agent>> {
        let viewer = require_viewer(ctx)?;
        let s = services(ctx);
        let rows: Vec<AgentRow> = sqlx::query_as(concat!(
            "SELECT ",
            agent_columns!(),
            " FROM agents a JOIN channel_agents ca ON ca.agent_id = a.id WHERE ca.channel_id = $1 ORDER BY a.name"
        ))
        .bind(self.uuid)
        .fetch_all(&s.pg)
        .await
        .map_err(internal)?;
        Ok(rows
            .into_iter()
            .map(|r| r.into_agent(&s.keys, &viewer))
            .collect())
    }

    /// Oldest first. Page backwards from the newest with `last` / `before`.
    async fn messages(
        &self,
        ctx: &Context<'_>,
        before: Option<String>,
        last: Option<i32>,
    ) -> Result<Connection<String, Message>> {
        let viewer = require_viewer(ctx)?;
        let s = services(ctx);
        let channel = self.uuid;
        query(
            None,
            before,
            None,
            last,
            |_after: Option<String>,
             before: Option<String>,
             _first: Option<usize>,
             last: Option<usize>| async move {
                let before = before
                    .map(|c| decode_typed(&s.keys, &c, "Message"))
                    .transpose()?;
                let limit = last.unwrap_or(50).min(100);
                let mut rows: Vec<MessageRow> = sqlx::query_as(concat!(
                    "SELECT ",
                    message_columns!(),
                    " FROM messages m WHERE m.channel_id = $1 AND ($2::uuid IS NULL OR m.id < $2)
                      ORDER BY m.id DESC LIMIT $3"
                ))
                .bind(channel)
                .bind(before)
                .bind(limit as i64 + 1)
                .fetch_all(&s.pg)
                .await
                .map_err(internal)?;
                let has_previous = rows.len() > limit;
                rows.truncate(limit);
                rows.reverse();
                let mut connection = Connection::new(has_previous, false);
                connection.edges.extend(rows.into_iter().map(|row| {
                    let cursor = s.keys.encode_id("Message", row.id, ID_TTL);
                    Edge::new(cursor, row.into_message(&s.keys, &viewer))
                }));
                Ok::<_, async_graphql::Error>(connection)
            },
        )
        .await
    }
}

// ---- Messages -------------------------------------------------------------------------------

#[derive(Enum, Copy, Clone, PartialEq, Eq)]
pub enum AuthorKind {
    User,
    Agent,
}

#[derive(SimpleObject, Clone)]
pub struct Message {
    pub id: ID,
    pub cache_key: String,
    pub author_name: String,
    pub author_kind: AuthorKind,
    pub from_viewer: bool,
    pub content: String,
    /// A failed agent reply; `content` holds the reason.
    pub is_error: bool,
    pub created_at: DateTime<Utc>,
}

#[derive(FromRow)]
struct MessageRow {
    id: Uuid,
    author_user_id: Option<Uuid>,
    author_name: String,
    author_is_agent: bool,
    content: String,
    is_error: bool,
    created_at: DateTime<Utc>,
}

impl MessageRow {
    fn into_message(self, keys: &Keys, viewer: &Viewer) -> Message {
        Message {
            id: ID(keys.encode_id("Message", self.id, ID_TTL)),
            cache_key: keys.cache_key("Message", self.id),
            author_name: self.author_name,
            author_kind: if self.author_is_agent {
                AuthorKind::Agent
            } else {
                AuthorKind::User
            },
            from_viewer: self.author_user_id == Some(viewer.user_id),
            content: self.content,
            is_error: self.is_error,
            created_at: self.created_at,
        }
    }
}

/// The message if the viewer may see its channel.
pub async fn load_message(s: &Services, viewer: &Viewer, id: Uuid) -> Result<Option<Message>> {
    let row: Option<MessageRow> = sqlx::query_as(concat!(
        "SELECT ",
        message_columns!(),
        " FROM messages m JOIN channels c ON c.id = m.channel_id
          WHERE m.id = $1 AND (NOT c.private OR c.created_by = $2)"
    ))
    .bind(id)
    .bind(viewer.user_id)
    .fetch_optional(&s.pg)
    .await
    .map_err(internal)?;
    Ok(row.map(|r| r.into_message(&s.keys, viewer)))
}

/// Which of the channel's agents a user message wakes: the only agent, or the @mentioned ones.
fn agents_to_wake(content: &str, agents: &[(Uuid, String)]) -> Vec<Uuid> {
    if let [(id, _)] = agents {
        return vec![*id];
    }
    let mentioned: Vec<&str> = content
        .split_whitespace()
        .filter_map(|w| w.strip_prefix('@'))
        .map(|w| w.trim_end_matches(|c: char| !(c.is_ascii_alphanumeric() || c == '-' || c == '_')))
        .collect();
    agents
        .iter()
        .filter(|(_, name)| mentioned.contains(&name.as_str()))
        .map(|(id, _)| *id)
        .collect()
}

// ---- Resolvers ------------------------------------------------------------------------------

#[derive(Default)]
pub struct ChatQuery;

#[Object]
impl ChatQuery {
    async fn agents(&self, ctx: &Context<'_>) -> Result<Vec<Agent>> {
        let viewer = require_viewer(ctx)?;
        let s = services(ctx);
        let rows: Vec<AgentRow> = sqlx::query_as(concat!(
            "SELECT ",
            agent_columns!(),
            " FROM agents a ORDER BY a.name"
        ))
        .fetch_all(&s.pg)
        .await
        .map_err(internal)?;
        Ok(rows
            .into_iter()
            .map(|r| r.into_agent(&s.keys, &viewer))
            .collect())
    }

    /// Models agents can use, `"<provider>/<model>"`, from every enabled provider.
    async fn available_models(&self, ctx: &Context<'_>) -> Result<Vec<String>> {
        require_viewer(ctx)?;
        let models = gateway::list_models(services(ctx))
            .await
            .map_err(internal)?;
        Ok(models
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|m| m["id"].as_str().map(String::from))
            .collect())
    }

    /// The channels the viewer can see, by name.
    async fn channels(&self, ctx: &Context<'_>) -> Result<Vec<Channel>> {
        let viewer = require_viewer(ctx)?;
        let s = services(ctx);
        // Private channels stay private, admins included.
        let rows: Vec<ChannelRow> = sqlx::query_as(
            "SELECT c.id, c.name, c.private, c.created_by, c.created_at FROM channels c
             WHERE NOT c.private OR c.created_by = $1 ORDER BY lower(c.name), c.id",
        )
        .bind(viewer.user_id)
        .fetch_all(&s.pg)
        .await
        .map_err(internal)?;
        Ok(rows
            .into_iter()
            .map(|r| r.into_channel(&s.keys, &viewer))
            .collect())
    }
}

#[derive(Default)]
pub struct ChatMutation;

#[Object]
impl ChatMutation {
    async fn create_agent(
        &self,
        ctx: &Context<'_>,
        name: String,
        model: String,
        #[graphql(default)] system_prompt: String,
    ) -> Result<Agent> {
        let viewer = require_viewer(ctx)?;
        if !validate_handle(&name) {
            return Err(coded(
                "INVALID_AGENT_NAME",
                "1–32 characters: a-z, 0-9, '-' or '_'",
            ));
        }
        validate_agent(&model, &system_prompt)?;
        let s = services(ctx);
        let id: Uuid = sqlx::query_scalar(
            "INSERT INTO agents (name, model, system_prompt, created_by) VALUES ($1, $2, $3, $4) RETURNING id",
        )
        .bind(&name)
        .bind(&model)
        .bind(&system_prompt)
        .bind(viewer.user_id)
        .fetch_one(&s.pg)
        .await
        .map_err(|e| match e.as_database_error() {
            Some(d) if d.is_unique_violation() => coded("AGENT_EXISTS", "name already used"),
            _ => internal(e),
        })?;
        load_agent(s, &viewer, id)
            .await?
            .ok_or_else(|| internal("agent vanished"))
    }

    /// Omitted fields keep their value.
    async fn update_agent(
        &self,
        ctx: &Context<'_>,
        id: ID,
        model: Option<String>,
        system_prompt: Option<String>,
    ) -> Result<Agent> {
        let viewer = require_viewer(ctx)?;
        let s = services(ctx);
        let row = editable_agent(s, &viewer, &id).await?;
        let model = model.unwrap_or(row.model);
        let system_prompt = system_prompt.unwrap_or(row.system_prompt);
        validate_agent(&model, &system_prompt)?;
        sqlx::query("UPDATE agents SET model = $2, system_prompt = $3 WHERE id = $1")
            .bind(row.id)
            .bind(model)
            .bind(system_prompt)
            .execute(&s.pg)
            .await
            .map_err(internal)?;
        load_agent(s, &viewer, row.id)
            .await?
            .ok_or_else(|| coded("NOT_FOUND", "no such agent"))
    }

    /// Also revokes its tokens and removes it from channels; its messages stay.
    async fn delete_agent(&self, ctx: &Context<'_>, id: ID) -> Result<bool> {
        let viewer = require_viewer(ctx)?;
        let s = services(ctx);
        let row = editable_agent(s, &viewer, &id).await?;
        sqlx::query("DELETE FROM agents WHERE id = $1")
            .bind(row.id)
            .execute(&s.pg)
            .await
            .map_err(internal)?;
        Ok(true)
    }

    /// A `/v1` token that acts as the agent: limited to its model, usage counted for it.
    /// Returned this once only; revoke it with `revokeApiKey`.
    async fn create_agent_token(
        &self,
        ctx: &Context<'_>,
        agent_id: ID,
        name: String,
    ) -> Result<CreatedApiKey> {
        let viewer = require_viewer(ctx)?;
        let s = services(ctx);
        let agent = editable_agent(s, &viewer, &agent_id).await?;
        let name = name.trim();
        if !(1..=64).contains(&name.chars().count()) {
            return Err(coded("INVALID_NAME", "1–64 characters"));
        }
        let token = gateway::new_token();
        let row: ApiKeyRow = sqlx::query_as(
            "INSERT INTO api_keys (user_id, agent_id, name, token_hash) VALUES ($1, $2, $3, $4)
             RETURNING id, name, created_at, last_used_at",
        )
        .bind(viewer.user_id)
        .bind(agent.id)
        .bind(name)
        .bind(gateway::hash_token(&token))
        .fetch_one(&s.pg)
        .await
        .map_err(internal)?;
        Ok(CreatedApiKey {
            api_key: ApiKey::from_row(row, &s.keys),
            token,
        })
    }

    async fn create_channel(
        &self,
        ctx: &Context<'_>,
        name: String,
        #[graphql(default)] private: bool,
        #[graphql(default)] agent_ids: Vec<ID>,
    ) -> Result<Channel> {
        let viewer = require_viewer(ctx)?;
        let name = validate_channel_name(&name)?;
        let s = services(ctx);
        let id: Uuid = sqlx::query_scalar(
            "INSERT INTO channels (name, private, created_by) VALUES ($1, $2, $3) RETURNING id",
        )
        .bind(name)
        .bind(private)
        .bind(viewer.user_id)
        .fetch_one(&s.pg)
        .await
        .map_err(internal)?;
        set_channel_agents(s, id, &agent_ids).await?;
        load_channel(s, &viewer, id)
            .await?
            .ok_or_else(|| internal("channel vanished"))
    }

    /// Omitted fields keep their value; `agentIds` replaces the channel's agents.
    async fn update_channel(
        &self,
        ctx: &Context<'_>,
        id: ID,
        name: Option<String>,
        agent_ids: Option<Vec<ID>>,
    ) -> Result<Channel> {
        let viewer = require_viewer(ctx)?;
        let s = services(ctx);
        let channel = editable_channel(s, &viewer, &id).await?;
        if let Some(name) = name {
            sqlx::query("UPDATE channels SET name = $2 WHERE id = $1")
                .bind(channel.uuid)
                .bind(validate_channel_name(&name)?)
                .execute(&s.pg)
                .await
                .map_err(internal)?;
        }
        if let Some(agent_ids) = agent_ids {
            set_channel_agents(s, channel.uuid, &agent_ids).await?;
        }
        load_channel(s, &viewer, channel.uuid)
            .await?
            .ok_or_else(|| coded("NOT_FOUND", "no such channel"))
    }

    /// Deletes the channel and its messages.
    async fn delete_channel(&self, ctx: &Context<'_>, id: ID) -> Result<bool> {
        let viewer = require_viewer(ctx)?;
        let s = services(ctx);
        let channel = editable_channel(s, &viewer, &id).await?;
        sqlx::query("DELETE FROM channels WHERE id = $1")
            .bind(channel.uuid)
            .execute(&s.pg)
            .await
            .map_err(internal)?;
        Ok(true)
    }

    /// Posts as the viewer; the channel's only agent, or the @mentioned ones, will answer.
    async fn post_message(
        &self,
        ctx: &Context<'_>,
        channel_id: ID,
        content: String,
    ) -> Result<Message> {
        let viewer = require_viewer(ctx)?;
        let content = content.trim();
        if content.is_empty() {
            return Err(coded("EMPTY_MESSAGE", "empty message"));
        }
        if content.chars().count() > MAX_MESSAGE_CHARS {
            return Err(coded("TOO_LONG", "message too long"));
        }
        let s = services(ctx);
        let channel = visible_channel(s, &viewer, &channel_id).await?;
        let id: Uuid = sqlx::query_scalar(
            "INSERT INTO messages (channel_id, author_user_id, author_name, content)
             SELECT $1, u.id, u.username, $3 FROM users u WHERE u.id = $2 RETURNING id",
        )
        .bind(channel.uuid)
        .bind(viewer.user_id)
        .bind(content)
        .fetch_one(&s.pg)
        .await
        .map_err(internal)?;

        let channel_event = Event::MessagePosted {
            channel_id: channel.uuid,
            message_id: id,
        };
        if let Err(e) = events::publish(s, &channel_event).await {
            tracing::warn!("publishing message {id}: {e:#}");
        }

        let agents: Vec<(Uuid, String)> = sqlx::query_as(
            "SELECT a.id, a.name FROM agents a JOIN channel_agents ca ON ca.agent_id = a.id
             WHERE ca.channel_id = $1",
        )
        .bind(channel.uuid)
        .fetch_all(&s.pg)
        .await
        .map_err(internal)?;
        for agent in agents_to_wake(content, &agents) {
            // shortcut: the message is committed before the job is queued; if Valkey is down here, that reply is skipped (the user can ask again).
            if let Err(e) = runtime::enqueue(s, id, agent).await {
                tracing::error!("queueing the reply of agent {agent} to {id}: {e:#}");
            }
        }

        load_message(s, &viewer, id)
            .await?
            .ok_or_else(|| internal("message vanished"))
    }
}

#[derive(SimpleObject, Clone)]
pub struct AgentTyping {
    pub agent: String,
    pub typing: bool,
}

#[derive(Default)]
pub struct ChatSubscription;

#[Subscription]
impl ChatSubscription {
    /// New messages in a channel the viewer can see (checked when the subscription starts).
    async fn message_posted(
        &self,
        ctx: &Context<'_>,
        channel_id: ID,
    ) -> Result<impl Stream<Item = Message> + use<>> {
        let viewer = require_viewer(ctx)?;
        let s = services(ctx).clone();
        let channel = visible_channel(&s, &viewer, &channel_id).await?.uuid;
        Ok(events::stream(&s).filter_map(move |e| {
            let s = s.clone();
            async move {
                match e {
                    Event::MessagePosted {
                        channel_id,
                        message_id,
                    } if channel_id == channel => {
                        load_message(&s, &viewer, message_id).await.ok().flatten()
                    }
                    _ => None,
                }
            }
        }))
    }

    /// Agents starting and finishing a reply in a channel the viewer can see.
    async fn agent_typing(
        &self,
        ctx: &Context<'_>,
        channel_id: ID,
    ) -> Result<impl Stream<Item = AgentTyping> + use<>> {
        let viewer = require_viewer(ctx)?;
        let s = services(ctx);
        let channel = visible_channel(s, &viewer, &channel_id).await?.uuid;
        Ok(events::stream(s).filter_map(move |e| async move {
            match e {
                Event::AgentTyping {
                    channel_id,
                    agent,
                    typing,
                } if channel_id == channel => Some(AgentTyping { agent, typing }),
                _ => None,
            }
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_only_agent_always_answers_else_mentions_decide() {
        let (a, b) = (Uuid::now_v7(), Uuid::now_v7());
        let one = [(a, "ada".to_owned())];
        let two = [(a, "ada".to_owned()), (b, "bob-2".to_owned())];
        assert_eq!(agents_to_wake("hello", &one), vec![a]);
        assert_eq!(agents_to_wake("hello", &two), Vec::<Uuid>::new());
        assert_eq!(agents_to_wake("hey @bob-2, and @ada!", &two), vec![a, b]);
        assert_eq!(
            agents_to_wake("mail ada@x.org or @adam", &two),
            Vec::<Uuid>::new()
        );
        assert_eq!(agents_to_wake("hi", &[]), Vec::<Uuid>::new());
    }
}
