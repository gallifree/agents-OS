use async_graphql::{Context, EmptyMutation, EmptySubscription, Object, Schema};

use crate::services::{ServiceStatus, Services};

pub type AppSchema = Schema<Query, EmptyMutation, EmptySubscription>;

pub fn schema(services: Services) -> AppSchema {
    Schema::build(Query, EmptyMutation, EmptySubscription)
        .data(services)
        .finish()
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
