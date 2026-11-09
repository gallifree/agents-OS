use async_graphql::{Context, Object, Result, SimpleObject, Subscription};
use futures::{Stream, StreamExt};

use super::{internal, require_admin, require_viewer, services};
use crate::{
    events::{self, Event},
    services::ServiceStatus,
};

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
        events::publish(services(ctx), &Event::Ping { message })
            .await
            .map_err(internal)?;
        Ok(true)
    }
}

#[derive(SimpleObject, Clone)]
pub struct Ping {
    pub message: String,
}

#[derive(Default)]
pub struct SystemSubscription;

#[Subscription]
impl SystemSubscription {
    // The viewer is checked when the subscription starts; a session revoked later ends with the socket.
    async fn pings(&self, ctx: &Context<'_>) -> Result<impl Stream<Item = Ping> + use<>> {
        require_viewer(ctx)?;
        Ok(events::stream(services(ctx)).filter_map(|e| async move {
            match e {
                Event::Ping { message } => Some(Ping { message }),
                _ => None,
            }
        }))
    }
}
