mod chat;
mod llm;
mod system;
mod users;

use std::time::Duration;

use async_graphql::{
    Context, Error, ErrorExtensions, ID, Interface, MergedObject, MergedSubscription, Result,
    Schema,
};
use crypto::{Keys, TokenError};
use uuid::Uuid;

use crate::{
    auth::{AuthError, Viewer},
    services::Services,
};

/// Lifetime of Relay IDs handed to clients; past it, `node(id:)` answers `ID_EXPIRED` and the client refetches.
pub const ID_TTL: Duration = Duration::from_secs(24 * 3600);

#[derive(MergedObject, Default)]
pub struct Query(
    system::SystemQuery,
    users::UsersQuery,
    llm::LlmQuery,
    chat::ChatQuery,
);

#[derive(MergedObject, Default)]
pub struct Mutation(
    system::SystemMutation,
    users::UsersMutation,
    llm::LlmMutation,
    chat::ChatMutation,
);

#[derive(MergedSubscription, Default)]
pub struct Subscription(system::SystemSubscription, chat::ChatSubscription);

pub type AppSchema = Schema<Query, Mutation, Subscription>;

pub fn schema(services: Services) -> AppSchema {
    Schema::build(
        Query::default(),
        Mutation::default(),
        Subscription::default(),
    )
    .data(services)
    .finish()
}

/// SDL for frontend codegen (`api schema`).
pub fn sdl() -> String {
    Schema::build(
        Query::default(),
        Mutation::default(),
        Subscription::default(),
    )
    .finish()
    .sdl()
}

fn services<'a>(ctx: &Context<'a>) -> &'a Services {
    ctx.data_unchecked::<Services>()
}

/// An error the UI can act on: `extensions.code` is stable, the message is for humans.
pub fn coded(code: &str, message: impl Into<String>) -> Error {
    Error::new(message).extend_with(|_, e| e.set("code", code))
}

fn auth_error(e: AuthError) -> Error {
    coded(e.code(), e.to_string())
}

/// The request's viewer, set by the HTTP / WebSocket handlers from the session cookie.
fn require_viewer(ctx: &Context<'_>) -> Result<Viewer> {
    ctx.data_opt::<Viewer>()
        .copied()
        .ok_or_else(|| coded("UNAUTHENTICATED", "sign in required"))
}

fn require_admin(ctx: &Context<'_>) -> Result<Viewer> {
    let viewer = require_viewer(ctx)?;
    if viewer.is_admin {
        Ok(viewer)
    } else {
        Err(coded("FORBIDDEN", "admins only"))
    }
}

/// Infrastructure failure: logged here, sent to the client as a bare `INTERNAL`.
fn internal(e: impl std::fmt::Display) -> Error {
    tracing::error!("graphql: {e:#}");
    coded("INTERNAL", "internal error")
}

/// Relay object identification: `id` is an expiring encrypted token, `cacheKey` the stable client identity.
#[derive(Interface)]
#[graphql(
    field(name = "id", ty = "&ID"),
    field(name = "cache_key", ty = "&String")
)]
pub enum Node {
    User(users::User),
    Provider(llm::Provider),
    ApiKey(llm::ApiKey),
    Agent(chat::Agent),
    Channel(chat::Channel),
    Message(chat::Message),
}

/// Decodes a client ID, mapping failures to the codes the UI handles.
fn decode_id(keys: &Keys, id: &str) -> Result<(String, Uuid)> {
    keys.decode_id(id).map_err(|e| match e {
        TokenError::Expired => coded("ID_EXPIRED", "id expired, refetch it"),
        TokenError::Invalid => coded("INVALID_ID", "invalid id"),
    })
}

/// Decodes a client ID that must point to a `type_name` object.
fn decode_typed(keys: &Keys, id: &str, type_name: &str) -> Result<Uuid> {
    match decode_id(keys, id)? {
        (t, uuid) if t == type_name => Ok(uuid),
        _ => Err(coded("INVALID_ID", "invalid id")),
    }
}
