mod system;
mod users;

use std::time::Duration;

use async_graphql::{Context, Error, ErrorExtensions, MergedObject, Result, Schema};

use crate::{
    auth::{AuthError, Viewer},
    services::Services,
};

/// Lifetime of Relay IDs handed to clients; past it, `node(id:)` answers `ID_EXPIRED` and the client refetches.
pub const ID_TTL: Duration = Duration::from_secs(24 * 3600);

#[derive(MergedObject, Default)]
pub struct Query(system::SystemQuery, users::UsersQuery);

#[derive(MergedObject, Default)]
pub struct Mutation(system::SystemMutation, users::UsersMutation);

pub type AppSchema = Schema<Query, Mutation, system::Subscription>;

pub fn schema(services: Services) -> AppSchema {
    Schema::build(Query::default(), Mutation::default(), system::Subscription)
        .data(services)
        .finish()
}

/// SDL for frontend codegen (`api schema`).
pub fn sdl() -> String {
    Schema::build(Query::default(), Mutation::default(), system::Subscription)
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
