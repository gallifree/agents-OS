//! Events fanned out to every backend instance over Valkey pub/sub (`EVENTS_CHANNEL`), for GraphQL
//! subscriptions. Fire-and-forget: a client that misses one refetches.

use fred::prelude::*;
use futures::Stream;
use serde::{Deserialize, Serialize};
use tokio::sync::broadcast::error::RecvError;
use uuid::Uuid;

use crate::services::{EVENTS_CHANNEL, Services};

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(tag = "type")]
pub enum Event {
    Ping {
        message: String,
    },
    MessagePosted {
        channel_id: Uuid,
        message_id: Uuid,
    },
    AgentTyping {
        channel_id: Uuid,
        agent: String,
        typing: bool,
    },
}

pub async fn publish(s: &Services, event: &Event) -> anyhow::Result<()> {
    let _: i64 = s
        .valkey
        .publish(EVENTS_CHANNEL, serde_json::to_string(event)?)
        .await?;
    Ok(())
}

/// Every event received from now on, by this instance.
pub fn stream(s: &Services) -> impl Stream<Item = Event> + use<> {
    futures::stream::unfold(s.events.message_rx(), |mut rx| async move {
        loop {
            match rx.recv().await {
                Ok(msg) => {
                    if let Some(event) = msg
                        .value
                        .as_string()
                        .and_then(|raw| serde_json::from_str(&raw).ok())
                    {
                        return Some((event, rx));
                    }
                }
                // A slow client missed events: keep going, pub/sub is fire-and-forget anyway.
                Err(RecvError::Lagged(_)) => continue,
                Err(RecvError::Closed) => return None,
            }
        }
    })
}
