//! The Cloudflare deployment of bible-tags: the same HTTP API and sync protocol as the self-hosted server
//! (`crates/server`), with accounts in D1 and each space in its own Durable Object ([`object::Space`]).
//! Static assets (client + text) are served by Workers Static Assets; only `/api/*` runs this Worker.
//!
//! ```text
//! GET  /api/health
//! POST /api/session                  -> {owner, account, providers}   (creates an anonymous owner if needed)
//! GET  /api/sync?space=              -> WebSocket, forwarded to the space (default: the owner's own)
//! *    /api/groups..., POST /api/join   groups (api::groups)
//! POST /api/logout
//! GET  /api/auth/google[/callback]   Sign in with Google (api::google)
//! ```

mod object;

use ::space::Value;
use api::accounts::{self as sql, Identity, SignIn};
use api::google::{self, Google, TokenResponse};
use api::groups;
use api::session::{self, random_hex, token_hash};
use object::now_ms;
use wasm_bindgen::JsValue;
use worker::{event, Context, D1Database, Env, Fetch, Headers, Method, Request, RequestInit, Response, Result, Stub};

#[event(fetch)]
async fn fetch(req: Request, env: Env, _ctx: Context) -> Result<Response> {
    let res = route(req, &env).await;
    res.or_else(|e| {
        worker::console_error!("{e}");
        Response::error("internal error", 500)
    })
}

async fn route(req: Request, env: &Env) -> Result<Response> {
    let path = req.path();
    match (req.method(), path.as_str()) {
        (Method::Get, "/api/health") => Response::from_json(&api::health("cloudflare")),
        (Method::Post, "/api/session") => session_route(&req, env).await,
        (Method::Get, "/api/sync") => sync(req, env).await,
        (Method::Post, "/api/logout") => logout(&req, env).await,
        (Method::Get, "/api/auth/google") => google_start(&req, env),
        (Method::Get, "/api/auth/google/callback") => google_callback(&req, env).await,
        (_, "/api/join") => groups(req, env).await,
        (_, p) if p == "/api/groups" || p.starts_with("/api/groups/") => groups(req, env).await,
        _ => Response::error("no such API route", 404),
    }
}

/// This request's origin (`https://host[:port]`).
fn origin(req: &Request) -> Result<String> {
    Ok(req.url()?.origin().ascii_serialization())
}

/// Cookies are `Secure` except on plain-HTTP local development.
fn secure(req: &Request) -> bool {
    req.url().is_ok_and(|u| u.scheme() == "https")
}

fn same_origin(req: &Request) -> Result<bool> {
    let url = req.url()?;
    let host = match url.port() {
        Some(p) => format!("{}:{p}", url.host_str().unwrap_or_default()),
        None => url.host_str().unwrap_or_default().to_string(),
    };
    Ok(session::same_origin(req.headers().get("Origin")?.as_deref(), &host))
}

fn cookie(req: &Request, name: &str) -> Option<String> {
    let header = req.headers().get("Cookie").ok().flatten()?;
    session::cookie([header.as_str()], name)
}

fn forbidden() -> Result<Response> {
    Response::error("cross-origin", 403)
}

// ---- accounts (D1) ----

/// The accounts database, its schema created once per isolate.
async fn accounts(env: &Env) -> Result<D1Database> {
    thread_local!(static READY: std::cell::Cell<bool> = const { std::cell::Cell::new(false) });
    let db = env.d1("ACCOUNTS")?;
    if !READY.get() {
        let statements = sql::schema_statements().into_iter().map(|s| db.prepare(s)).collect();
        db.batch(statements).await?;
        READY.set(true);
    }
    Ok(db)
}

fn js(s: &str) -> JsValue {
    JsValue::from_str(s)
}

#[allow(clippy::cast_precision_loss)] // milliseconds since 1970 are far below 2^53
fn js_int(n: i64) -> JsValue {
    JsValue::from_f64(n as f64)
}

async fn one_string(db: &D1Database, query: &str, params: &[JsValue]) -> Result<Option<String>> {
    db.prepare(query).bind(params)?.first::<String>(Some("owner_id")).await
}

async fn session_owner(db: &D1Database, req: &Request) -> Result<Option<String>> {
    let Some(token) = cookie(req, session::COOKIE) else { return Ok(None) };
    one_string(db, sql::SESSION_OWNER, &[js(&token_hash(&token))]).await
}

async fn owner_identity(db: &D1Database, owner: &str) -> Result<Option<Identity>> {
    db.prepare(sql::OWNER_IDENTITY).bind(&[js(owner)])?.first::<Identity>(None).await
}

async fn create_owner(db: &D1Database, owner: &str, now: i64) -> Result<()> {
    db.prepare(sql::CREATE_OWNER).bind(&[js(owner), js_int(now)])?.run().await.map(drop)
}

async fn add_identity(db: &D1Database, id: &Identity, owner: &str, now: i64) -> Result<()> {
    let opt = |s: &Option<String>| s.as_deref().map_or(JsValue::NULL, js);
    let params = [js(&id.provider), js(&id.subject), js(owner), opt(&id.email), opt(&id.name), js_int(now)];
    db.prepare(sql::ADD_IDENTITY).bind(&params)?.run().await.map(drop)
}

/// A new session for `owner`; returns its token.
async fn create_session(db: &D1Database, owner: &str, now: i64) -> Result<String> {
    let token = random_hex(32).map_err(worker::Error::RustError)?;
    db.prepare(sql::CREATE_SESSION).bind(&[js(&token_hash(&token)), js(owner), js_int(now)])?.run().await?;
    Ok(token)
}

fn providers(env: &Env) -> &'static [&'static str] {
    if google_config(env).ok().flatten().is_some() {
        &["google"]
    } else {
        &[]
    }
}

/// `POST /api/session`.
async fn session_route(req: &Request, env: &Env) -> Result<Response> {
    if !same_origin(req)? {
        return forbidden();
    }
    let db = accounts(env).await?;
    if let Some(owner) = session_owner(&db, req).await? {
        let account = owner_identity(&db, &owner).await?;
        return Response::from_json(&session::session_json(&owner, account.as_ref(), providers(env)));
    }
    let owner = random_hex(16).map_err(worker::Error::RustError)?;
    let now = now_ms();
    create_owner(&db, &owner, now).await?;
    let token = create_session(&db, &owner, now).await?;
    let mut res = Response::from_json(&session::session_json(&owner, None, providers(env)))?;
    res.headers_mut().append("Set-Cookie", &session::session_cookie(&token, secure(req)))?;
    Ok(res)
}

/// `POST /api/logout`.
async fn logout(req: &Request, env: &Env) -> Result<Response> {
    if !same_origin(req)? {
        return forbidden();
    }
    if let Some(token) = cookie(req, session::COOKIE) {
        accounts(env).await?.prepare(sql::DELETE_SESSION).bind(&[js(&token_hash(&token))])?.run().await?;
    }
    let mut res = Response::empty()?.with_status(204);
    res.headers_mut().append("Set-Cookie", &session::clear_session_cookie(secure(req)))?;
    Ok(res)
}

// ---- spaces ----

fn space_stub(env: &Env, space: &str) -> Result<Stub> {
    env.durable_object("SPACES")?.id_from_name(space)?.get_stub()
}

/// `GET /api/sync?space=`: authenticate, check the role, then hand the WebSocket upgrade to the space.
async fn sync(req: Request, env: &Env) -> Result<Response> {
    if !same_origin(&req)? {
        return forbidden();
    }
    let db = accounts(env).await?;
    let Some(owner) = session_owner(&db, &req).await? else {
        return Response::error("no session", 401);
    };
    let space = req.url()?.query_pairs().find(|(k, _)| k == "space").map(|(_, v)| v.into_owned());
    let space = space.unwrap_or_else(|| owner.clone());
    if !api::check::valid_id(&space) {
        return Response::error("bad space", 400);
    }
    // An early refusal; the Durable Object checks again when it accepts the socket.
    if groups::role_of(&D1Accounts(db), &space, &owner).await.map_err(worker::Error::RustError)?.is_none() {
        return Response::error("not a member", 403);
    }
    let headers = Headers::new();
    for name in ["Upgrade", "Connection", "Sec-WebSocket-Key", "Sec-WebSocket-Version"] {
        if let Some(v) = req.headers().get(name)? {
            headers.set(name, &v)?;
        }
    }
    headers.set("x-owner", &owner)?;
    headers.set("x-space", &space)?;
    let mut init = RequestInit::new();
    init.with_headers(headers);
    space_stub(env, &space)?.fetch_with_request(Request::new_with_init("https://space/sync", &init)?).await
}

/// `space::Value` rows over D1.
struct D1Accounts(D1Database);

fn js_value(v: &Value) -> JsValue {
    match v {
        Value::Null => JsValue::NULL,
        Value::Int(n) => js_int(*n),
        Value::Text(s) => js(s),
    }
}

impl groups::AccountsDb for D1Accounts {
    async fn exec(&self, sql: &str, params: Vec<Value>) -> std::result::Result<(), String> {
        let params: Vec<JsValue> = params.iter().map(js_value).collect();
        let stmt = self.0.prepare(sql).bind(&params).map_err(|e| e.to_string())?;
        stmt.run().await.map(drop).map_err(|e| e.to_string())
    }

    async fn query(&self, sql: &str, params: Vec<Value>) -> std::result::Result<Vec<Vec<Value>>, String> {
        let params: Vec<JsValue> = params.iter().map(js_value).collect();
        let stmt = self.0.prepare(sql).bind(&params).map_err(|e| e.to_string())?;
        let rows = stmt.raw::<serde_json::Value>().await.map_err(|e| e.to_string())?;
        Ok(rows
            .into_iter()
            .map(|r| {
                r.into_iter()
                    .map(|v| match v {
                        serde_json::Value::String(s) => Value::Text(s),
                        serde_json::Value::Number(n) => Value::Int(n.as_i64().unwrap_or_default()),
                        _ => Value::Null,
                    })
                    .collect()
            })
            .collect())
    }
}

/// `/api/groups...` and `/api/join` (`api::groups`).
async fn groups(mut req: Request, env: &Env) -> Result<Response> {
    if req.method() != Method::Get && !same_origin(&req)? {
        return forbidden();
    }
    let db = accounts(env).await?;
    let Some(owner) = session_owner(&db, &req).await? else { return Response::error("no session", 401) };
    let body = req.text().await.unwrap_or_default();
    let (method, path) = (req.method().to_string(), req.path());
    let path = path.trim_start_matches("/api");
    let reply = groups::handle(&D1Accounts(db), &owner, &method, path, &body, now_ms())
        .await
        .map_err(worker::Error::RustError)?;
    for (space, who) in &reply.kicks {
        let url = format!("https://space/kick?owner={who}");
        let mut init = RequestInit::new();
        init.with_method(Method::Post);
        space_stub(env, space)?.fetch_with_request(Request::new_with_init(&url, &init)?).await?;
    }
    Ok(Response::from_json(&reply.body)?.with_status(reply.status))
}

/// Copy anonymous owner `from`'s live data (its personal space) into `into`'s personal space.
async fn merge(env: &Env, from: &str, into: &str) -> Result<usize> {
    let mut res = space_stub(env, from)?.fetch_with_str(&format!("https://space/export?owner={from}")).await?;
    let rows: Vec<ops::Row> = res.json().await?;
    if rows.is_empty() {
        return Ok(0);
    }
    let body = serde_json::json!({ "owner": into, "from": from, "rows": rows });
    let mut init = RequestInit::new();
    init.with_method(Method::Post).with_body(Some(js(&body.to_string())));
    let mut res =
        space_stub(env, into)?.fetch_with_request(Request::new_with_init("https://space/merge", &init)?).await?;
    if res.status_code() != 200 {
        return Err(worker::Error::RustError(format!("merge: HTTP {}: {}", res.status_code(), res.text().await?)));
    }
    res.json().await
}

/// The anonymous owner `from`, merged into account `into`, may have joined (or created) groups: its comments there
/// become `into`'s (so they stay editable), its sockets there are closed, then its memberships move to `into`.
async fn carry_groups(env: &Env, from: &str, into: &str, now: i64) -> Result<()> {
    let accounts = D1Accounts(accounts(env).await?);
    for space in groups::spaces_of(&accounts, from).await.map_err(worker::Error::RustError)? {
        let body = serde_json::json!({ "space": space, "from": from, "to": into });
        let mut init = RequestInit::new();
        init.with_method(Method::Post).with_body(Some(js(&body.to_string())));
        let stub = space_stub(env, &space)?;
        let res = stub.fetch_with_request(Request::new_with_init("https://space/reassign", &init)?).await?;
        if res.status_code() != 200 {
            return Err(worker::Error::RustError(format!("reassign in {space}: HTTP {}", res.status_code())));
        }
        let mut kick = RequestInit::new();
        kick.with_method(Method::Post);
        stub.fetch_with_request(Request::new_with_init(&format!("https://space/kick?owner={from}"), &kick)?).await?;
    }
    groups::move_memberships(&accounts, from, into, now).await.map_err(worker::Error::RustError)?;
    Ok(())
}

// ---- Sign in with Google ----

/// From the `GOOGLE_CLIENT_ID` / `GOOGLE_CLIENT_SECRET` secrets (both or neither). `GOOGLE_AUTH_URL` /
/// `GOOGLE_TOKEN_URL` override the endpoints (for local tests).
fn google_config(env: &Env) -> Result<Option<Google>> {
    let get = |name: &str| env.secret(name).or_else(|_| env.var(name)).ok().map(|v| v.to_string());
    let mut g = match (get("GOOGLE_CLIENT_ID"), get("GOOGLE_CLIENT_SECRET")) {
        (Some(id), Some(secret)) => Google::new(id, secret),
        (None, None) => return Ok(None),
        _ => return Err(worker::Error::RustError("set both GOOGLE_CLIENT_ID and GOOGLE_CLIENT_SECRET".into())),
    };
    if let Some(u) = get("GOOGLE_AUTH_URL") {
        g.auth_url = u;
    }
    if let Some(u) = get("GOOGLE_TOKEN_URL") {
        g.token_url = u;
    }
    Ok(Some(g))
}

fn callback_url(req: &Request, env: &Env) -> Result<String> {
    let public = env.var("PUBLIC_URL").ok().map(|v| v.to_string()).filter(|v| !v.is_empty());
    Ok(google::callback_url(&public.map_or_else(|| origin(req), Ok)?))
}

fn redirect(to: &str, cookies: &[String]) -> Result<Response> {
    let headers = Headers::new();
    headers.set("Location", to)?;
    for c in cookies {
        headers.append("Set-Cookie", c)?;
    }
    Ok(Response::empty()?.with_status(302).with_headers(headers))
}

/// `GET /api/auth/google?return=#hash`: off to Google's consent screen.
fn google_start(req: &Request, env: &Env) -> Result<Response> {
    let Some(g) = google_config(env)? else { return Response::error("sign-in is not configured", 404) };
    let ret = req.url()?.query_pairs().find(|(k, _)| k == "return").map(|(_, v)| v.into_owned()).unwrap_or_default();
    let (url, flow) = google::start(&g, &callback_url(req, env)?, &ret).map_err(worker::Error::RustError)?;
    redirect(&url, &[google::flow_cookie(&flow, 600, secure(req))])
}

/// `GET /api/auth/google/callback`: finish signing in, then back to the app.
async fn google_callback(req: &Request, env: &Env) -> Result<Response> {
    let Some(g) = google_config(env)? else { return Response::error("sign-in is not configured", 404) };
    let flow = cookie(req, google::FLOW_COOKIE).unwrap_or_default();
    let (state, verifier, ret) = google::parse_flow(&flow);
    let clear_flow = google::flow_cookie("", 0, secure(req));
    let fail = |code: &str| redirect(&format!("/?auth_error={code}{ret}"), std::slice::from_ref(&clear_flow));
    let query = |name: &str| req.url().ok()?.query_pairs().find(|(k, _)| k == name).map(|(_, v)| v.into_owned());
    if query("error").is_some() {
        return fail("cancelled");
    }
    let (Some(code), Some(state), Some(verifier)) = (query("code"), state, verifier) else { return fail("expired") };
    if query("state").as_deref() != Some(state) {
        return fail("expired");
    }
    let form = google::token_form(&g, &code, &callback_url(req, env)?, verifier);
    let token = match exchange(&g.token_url, &google::form_body(&form)).await {
        Ok(t) => t,
        Err(e) => {
            worker::console_warn!("google token exchange: {e}");
            return fail("provider");
        }
    };
    let identity = match google::identity_from(&token.id_token, &g.client_id, now_ms() / 1000) {
        Ok(i) => i,
        Err(code) => return fail(code),
    };
    match sign_in(req, env, &identity).await {
        Ok(token) => redirect(&format!("/{ret}"), &[clear_flow.clone(), session::session_cookie(&token, secure(req))]),
        Err(e) => {
            worker::console_error!("sign-in: {e}");
            fail("internal")
        }
    }
}

async fn exchange(token_url: &str, body: &str) -> Result<TokenResponse> {
    let headers = Headers::new();
    headers.set("Content-Type", "application/x-www-form-urlencoded")?;
    let mut init = RequestInit::new();
    init.with_method(Method::Post).with_headers(headers).with_body(Some(js(body)));
    let mut res = Fetch::Request(Request::new_with_init(token_url, &init)?).send().await?;
    if res.status_code() != 200 {
        return Err(worker::Error::RustError(format!("HTTP {}: {}", res.status_code(), res.text().await?)));
    }
    res.json().await
}

/// Decide the account for `identity` (`api::accounts::sign_in`) and open a new session for it; returns its token.
async fn sign_in(req: &Request, env: &Env, identity: &Identity) -> Result<String> {
    let db = accounts(env).await?;
    let now = now_ms();
    let current = session_owner(&db, req).await?;
    let existing = one_string(&db, sql::IDENTITY_OWNER, &[js(&identity.provider), js(&identity.subject)]).await?;
    let anonymous = match &current {
        Some(c) => owner_identity(&db, c).await?.is_none(),
        None => false,
    };
    let owner = match sql::sign_in(current.as_deref(), anonymous, existing.as_deref()) {
        SignIn::Existing { owner, merge_from } => {
            if let Some(a) = merge_from {
                let n = merge(env, &a, &owner).await?;
                worker::console_log!("merged {n} changes from anonymous owner {a} into {owner}");
                carry_groups(env, &a, &owner, now).await?;
            }
            owner
        }
        SignIn::Link(a) => {
            add_identity(&db, identity, &a, now).await?;
            a
        }
        SignIn::New => {
            let o = random_hex(16).map_err(worker::Error::RustError)?;
            create_owner(&db, &o, now).await?;
            add_identity(&db, identity, &o, now).await?;
            o
        }
    };
    if let Some(t) = cookie(req, session::COOKIE) {
        db.prepare(sql::DELETE_SESSION).bind(&[js(&token_hash(&t))])?.run().await?;
    }
    create_session(&db, &owner, now).await
}
