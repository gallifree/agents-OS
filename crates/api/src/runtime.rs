//! Agent runtime: reply jobs on a Valkey Stream. A user message queues one job per agent that should
//! answer; workers (one per backend instance, consumer group) call the model and post the reply.
//!
//! Streams deliver at least once, so jobs are idempotent: a reply is unique per (message, agent).

use std::{collections::HashMap, time::Duration};

use anyhow::{Context as _, Result};
use fred::{
    prelude::*,
    types::streams::{XReadResponse, XReadValue},
};
use futures::StreamExt;
use serde_json::{Value, json};
use uuid::Uuid;

use crate::{
    events::{self, Event},
    gateway,
    services::Services,
};

const STREAM: &str = "agents:replies";
const GROUP: &str = "agents";
/// Acked entries stay in the stream: keep roughly this many.
const STREAM_MAX_LEN: i64 = 10_000;
/// Jobs a crashed worker left pending are taken over after this long (longer than any LLM call).
const RECLAIM_IDLE_MS: u64 = 10 * 60 * 1000;
const CONCURRENCY: usize = 8;
/// Recent channel messages sent to the model as context.
const HISTORY: i64 = 30;

pub async fn enqueue(s: &Services, message_id: Uuid, agent_id: Uuid) -> Result<()> {
    let _: String = s
        .valkey
        .xadd(
            STREAM,
            false,
            ("MAXLEN", "~", STREAM_MAX_LEN),
            "*",
            vec![
                ("message", message_id.to_string()),
                ("agent", agent_id.to_string()),
            ],
        )
        .await?;
    Ok(())
}

/// Runs the worker for the life of the process, restarting it after errors (Valkey down…).
pub fn spawn(s: Services) {
    tokio::spawn(async move {
        loop {
            if let Err(e) = work(&s).await {
                tracing::warn!("agent runtime: {e:#}; restarting in 5s");
            }
            tokio::time::sleep(Duration::from_secs(5)).await;
        }
    });
}

async fn work(s: &Services) -> Result<()> {
    // Blocking reads would stall every other command on a shared connection: the worker has its own.
    let jobs = &s.jobs;
    if let Err(e) = jobs
        .xgroup_create::<(), _, _, _>(STREAM, GROUP, "$", true)
        .await
        && !e.details().contains("BUSYGROUP")
    {
        return Err(e.into());
    }
    let consumer = std::env::var("HOSTNAME").unwrap_or_else(|_| Uuid::new_v4().to_string());
    tracing::info!("agent runtime: consuming {STREAM} as {consumer}");
    loop {
        let (_, stale): (String, Vec<XReadValue<String, String, String>>) = jobs
            .xautoclaim_values(
                STREAM,
                GROUP,
                &consumer,
                RECLAIM_IDLE_MS,
                "0-0",
                Some(10),
                false,
            )
            .await?;
        let fresh: XReadResponse<String, String, String, String> = jobs
            .xreadgroup_map(GROUP, &consumer, Some(10), Some(5_000), false, STREAM, ">")
            .await?;
        let entries = stale.into_iter().chain(fresh.into_values().flatten());
        futures::stream::iter(entries)
            .for_each_concurrent(CONCURRENCY, |(id, fields)| async move {
                if let Err(e) = handle(s, &fields).await {
                    // Not retried: the failure is either permanent (bad job) or already posted as an error reply.
                    tracing::error!("agent reply job {id}: {e:#}");
                }
                if let Err(e) = jobs.xack::<i64, _, _, _>(STREAM, GROUP, &id).await {
                    tracing::warn!("acking agent reply job {id}: {e}");
                }
            })
            .await;
    }
}

async fn handle(s: &Services, fields: &HashMap<String, String>) -> Result<()> {
    let field = |k: &str| -> Result<Uuid> { Ok(fields.get(k).context("missing field")?.parse()?) };
    reply(s, field("message")?, field("agent")?).await
}

async fn reply(s: &Services, message_id: Uuid, agent_id: Uuid) -> Result<()> {
    let done: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM messages WHERE reply_to = $1 AND author_agent_id = $2)",
    )
    .bind(message_id)
    .bind(agent_id)
    .fetch_one(&s.pg)
    .await?;
    if done {
        return Ok(());
    }
    let agent: Option<(String, String, String)> =
        sqlx::query_as("SELECT name, model, system_prompt FROM agents WHERE id = $1")
            .bind(agent_id)
            .fetch_optional(&s.pg)
            .await?;
    let trigger: Option<(Uuid, Option<Uuid>)> =
        sqlx::query_as("SELECT channel_id, author_user_id FROM messages WHERE id = $1")
            .bind(message_id)
            .fetch_optional(&s.pg)
            .await?;
    // Agent, message or channel deleted since: nothing to answer.
    let (Some((name, model, system_prompt)), Some((channel_id, user_id))) = (agent, trigger) else {
        return Ok(());
    };

    let mut history: Vec<(Option<Uuid>, String, String)> = sqlx::query_as(
        "SELECT author_agent_id, author_name, content FROM messages
         WHERE channel_id = $1 AND id <= $2 AND NOT is_error ORDER BY id DESC LIMIT $3",
    )
    .bind(channel_id)
    .bind(message_id)
    .bind(HISTORY)
    .fetch_all(&s.pg)
    .await?;
    history.reverse();
    let messages = prompt(&name, &system_prompt, agent_id, &history);

    let typing = |typing| Event::AgentTyping {
        channel_id,
        agent: name.clone(),
        typing,
    };
    let _ = events::publish(s, &typing(true)).await;
    let answer = gateway::complete(s, &model, messages, user_id, agent_id).await;
    let _ = events::publish(s, &typing(false)).await;

    let (content, is_error) = match answer {
        Ok(content) => (content, false),
        Err(e) => (format!("{e:#}"), true),
    };
    let posted: Option<Uuid> = sqlx::query_scalar(
        "INSERT INTO messages (channel_id, author_agent_id, author_name, author_is_agent, content, is_error, reply_to)
         VALUES ($1, $2, $3, true, $4, $5, $6)
         ON CONFLICT (reply_to, author_agent_id) DO NOTHING RETURNING id",
    )
    .bind(channel_id)
    .bind(agent_id)
    .bind(&name)
    .bind(content)
    .bind(is_error)
    .bind(message_id)
    .fetch_optional(&s.pg)
    .await?;
    if let Some(id) = posted {
        events::publish(
            s,
            &Event::MessagePosted {
                channel_id,
                message_id: id,
            },
        )
        .await?;
    }
    Ok(())
}

/// The chat sent to the model: the agent's own messages are `assistant`, everyone else's are `user`
/// turns prefixed with their author, so the model can tell people (and other agents) apart.
fn prompt(
    name: &str,
    system_prompt: &str,
    agent_id: Uuid,
    history: &[(Option<Uuid>, String, String)],
) -> Value {
    let mut system = format!(
        "You are {name}, an AI agent in a group chat. Each message from others starts with \"<author>: \". \
         Answer the last message; don't prefix your answer with your name."
    );
    if !system_prompt.trim().is_empty() {
        system = format!("{system}\n\n{system_prompt}");
    }
    let mut messages = vec![json!({ "role": "system", "content": system })];
    messages.extend(history.iter().map(|(author_agent, author, content)| {
        if *author_agent == Some(agent_id) {
            json!({ "role": "assistant", "content": content })
        } else {
            json!({ "role": "user", "content": format!("{author}: {content}") })
        }
    }));
    Value::Array(messages)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prompt_tells_own_turns_from_others() {
        let (me, other) = (Uuid::now_v7(), Uuid::now_v7());
        let history = [
            (None, "alice".to_owned(), "hi".to_owned()),
            (Some(me), "ada".to_owned(), "hello alice".to_owned()),
            (Some(other), "bob".to_owned(), "hey".to_owned()),
        ];
        let p = prompt("ada", "Be brief.", me, &history);
        assert_eq!(p[0]["role"], "system");
        assert!(p[0]["content"].as_str().unwrap().ends_with("Be brief."));
        assert_eq!(p[1], json!({ "role": "user", "content": "alice: hi" }));
        assert_eq!(
            p[2],
            json!({ "role": "assistant", "content": "hello alice" })
        );
        assert_eq!(p[3], json!({ "role": "user", "content": "bob: hey" }));
    }
}
