use async_graphql::{Context, Object, Result, SimpleObject, Subscription};
use fred::prelude::*;
use futures::Stream;
use serde::{Deserialize, Serialize};
use tokio::sync::broadcast::error::RecvError;

use super::{require_admin, require_viewer, services};
use crate::services::{EVENTS_CHANNEL, ServiceStatus};

#[derive(Default)]
pub struct SystemQuery;

#[Object]
impl SystemQuery {
    /// State of every backing service. Admins only: errors expose internal hostnames.
    async fn system_status(&self, ctx: &Context<'_>) -> Result<Vec<ServiceStatus>> {
        require_admin(ctx)?;
        Ok(services(ctx).status().await)
    }
}

#[derive(Default)]
pub struct SystemMutation;

const MAX_PING_LEN: usize = 200;

#[Object]
impl SystemMutation {
    /// Publishes a ping on Valkey pub/sub; every `pings` subscriber, on any backend instance, receives it.
    async fn ping(&self, ctx: &Context<'_>, message: String) -> Result<bool> {
        require_viewer(ctx)?;
        if message.chars().count() > MAX_PING_LEN {
            return Err(format!("message longer than {MAX_PING_LEN} characters").into());
        }
        let event = serde_json::to_string(&Event::Ping(Ping { message }))?;
        let _: i64 = services(ctx).valkey.publish(EVENTS_CHANNEL, event).await?;
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

pub struct Subscription;

#[Subscription]
impl Subscription {
    // The viewer is checked when the subscription starts; a session revoked later ends with the socket.
    async fn pings(&self, ctx: &Context<'_>) -> Result<impl Stream<Item = Ping> + use<>> {
        require_viewer(ctx)?;
        let rx = services(ctx).events.message_rx();
        Ok(futures::stream::unfold(rx, |mut rx| async move {
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
        }))
    }
}
