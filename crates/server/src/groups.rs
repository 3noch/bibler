//! Groups (`api::groups`) on the self-hosted server: the shared handler over the accounts database.

use crate::store::Store;
use crate::sync::{blocking, internal, owner_of, same_origin, AppState};
use api::groups::AccountsDb;
use axum::extract::State;
use axum::http::{HeaderMap, Method, StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use axum::Json;
use space::Value;
use std::sync::Arc;

/// The accounts database, for `api::groups` (each call runs on the blocking pool).
pub struct Accounts(pub Arc<dyn Store>);

impl AccountsDb for Accounts {
    async fn exec(&self, sql: &str, params: Vec<Value>) -> Result<(), String> {
        let (store, sql) = (self.0.clone(), sql.to_string());
        blocking(move || store.accounts_exec(&sql, params)).await?
    }

    async fn query(&self, sql: &str, params: Vec<Value>) -> Result<Vec<Vec<Value>>, String> {
        let (store, sql) = (self.0.clone(), sql.to_string());
        blocking(move || store.accounts_query(&sql, params)).await?
    }
}

/// `/api/groups...` and `/api/join`.
pub async fn groups(
    State(st): State<Arc<AppState>>,
    method: Method,
    uri: Uri,
    headers: HeaderMap,
    body: String,
) -> Response {
    if method != Method::GET && !same_origin(&headers) {
        return (StatusCode::FORBIDDEN, "cross-origin").into_response();
    }
    let Some(owner) = owner_of(&st, &headers).await else {
        return (StatusCode::UNAUTHORIZED, "no session").into_response();
    };
    let db = Accounts(st.store.clone());
    match api::groups::handle(&db, &owner, method.as_str(), uri.path(), &body, crate::sync::now_ms()).await {
        Ok(reply) => {
            for (space, who) in &reply.kicks {
                st.hub.kick(space, who);
                // Left the group: their reading history there goes too.
                if let Ok(None) = api::groups::role_of(&db, space, who).await {
                    let (store, s, w) = (st.store.clone(), space.clone(), who.clone());
                    if let Err(e) | Ok(Err(e)) = crate::sync::blocking(move || store.forget_history(&s, &w)).await {
                        tracing::error!("forget history: {e}");
                    }
                }
            }
            let status = StatusCode::from_u16(reply.status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
            (status, Json(reply.body)).into_response()
        }
        Err(e) => internal(&e),
    }
}
