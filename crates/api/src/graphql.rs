use async_graphql::{Context, Object, Result, Schema, SimpleObject, Subscription};
use fred::prelude::*;
use futures::Stream;
use serde::{Deserialize, Serialize};
use tokio::sync::broadcast::error::RecvError;

use crate::services::{EVENTS_CHANNEL, ServiceStatus, Services};

pub type AppSchema = Schema<Query, Mutation, SubscriptionRoot>;

pub fn schema(services: Services) -> AppSchema {
    Schema::build(Query, Mutation, SubscriptionRoot)
        .data(services)
        .finish()
}

/// SDL for frontend codegen (`api schema`).
pub fn sdl() -> String {
    Schema::build(Query, Mutation, SubscriptionRoot)
        .finish()
        .sdl()
}

pub struct Query;

#[Object]
impl Query {
    /// State of every backing service.
    // shortcut: unauthenticated until milestone 2 (login); restrict to admins then, errors expose internal hostnames.
    async fn system_status(&self, ctx: &Context<'_>) -> Vec<ServiceStatus> {
        ctx.data_unchecked::<Services>().status().await
    }
}

pub struct Mutation;

const MAX_PING_LEN: usize = 200;

#[Object]
impl Mutation {
    /// Publishes a ping on Valkey pub/sub; every `pings` subscriber, on any backend instance, receives it.
    async fn ping(&self, ctx: &Context<'_>, message: String) -> Result<bool> {
        if message.chars().count() > MAX_PING_LEN {
            return Err(format!("message longer than {MAX_PING_LEN} characters").into());
        }
        let event = serde_json::to_string(&Event::Ping(Ping { message }))?;
        let _: i64 = ctx
            .data_unchecked::<Services>()
            .valkey
            .publish(EVENTS_CHANNEL, event)
            .await?;
        Ok(true)
    }
}

/// Envelope of everything published on `EVENTS_CHANNEL`.
#[derive(Serialize, Deserialize)]
#[serde(tag = "type")]
enum Event {
    Ping(Ping),
}

#[derive(SimpleObject, Serialize, Deserialize, Clone)]
pub struct Ping {
    pub message: String,
}

pub struct SubscriptionRoot;

#[Subscription(name = "Subscription")]
impl SubscriptionRoot {
    async fn pings(&self, ctx: &Context<'_>) -> impl Stream<Item = Ping> {
        let rx = ctx.data_unchecked::<Services>().events.message_rx();
        futures::stream::unfold(rx, |mut rx| async move {
            loop {
                match rx.recv().await {
                    Ok(msg) => {
                        let Some(raw) = msg.value.as_string() else {
                            continue;
                        };
                        if let Ok(Event::Ping(ping)) = serde_json::from_str(&raw) {
                            return Some((ping, rx));
                        }
                    }
                    // A slow client missed events: keep going, pub/sub is fire-and-forget anyway.
                    Err(RecvError::Lagged(_)) => continue,
                    Err(RecvError::Closed) => return None,
                }
            }
        })
    }
}
