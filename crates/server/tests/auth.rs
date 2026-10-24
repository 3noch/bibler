//! Sign in with Google against a fake token endpoint: PKCE, account linking, merging an anonymous device's data
//! into an existing account, session rotation and logout.
mod common;

use axum::body::Body;
use axum::extract::State;
use axum::http::{header, Request, StatusCode};
use axum::routing::post;
use axum::{Form, Json, Router};
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine as _;
use futures_util::{SinkExt, StreamExt};
use http_body_util::BodyExt;
use serde_json::{json, Value};
use server::auth::Google;
use server::{router, Config};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::Message;
use tower::ServiceExt;

const T1: u32 = 2 << 24 | 1 << 16 | 1 << 8; // 1:1, the first word of the fixture text (100 words)

/// code -> (PKCE challenge, `redirect_uri`, `sub`)
type Codes = Arc<Mutex<HashMap<String, (String, String, String)>>>;

async fn fake_google(codes: Codes) -> String {
    async fn token(
        State(codes): State<Codes>,
        Form(f): Form<HashMap<String, String>>,
    ) -> Result<Json<Value>, StatusCode> {
        let (challenge, redirect, sub) = codes.lock().unwrap().remove(&f["code"]).ok_or(StatusCode::BAD_REQUEST)?;
        let verified = URL_SAFE_NO_PAD.encode(Sha256::digest(f["code_verifier"].as_bytes())) == challenge;
        if !verified || f["redirect_uri"] != redirect || f["client_id"] != "cid" || f["client_secret"] != "secret" {
            return Err(StatusCode::BAD_REQUEST);
        }
        let claims = json!({"iss": "https://accounts.google.com", "aud": "cid", "sub": sub, "exp": 4_000_000_000_i64,
            "email": format!("{sub}@example.com"), "email_verified": true, "name": "Test"});
        Ok(Json(json!({"id_token": format!("h.{}.s", URL_SAFE_NO_PAD.encode(claims.to_string()))})))
    }
    let app = Router::new().route("/token", post(token)).with_state(codes);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    format!("http://{addr}/token")
}

struct Srv {
    app: Router,
    addr: std::net::SocketAddr,
    codes: Codes,
}

async fn start(name: &str) -> Srv {
    let codes: Codes = Arc::default();
    let token_url = fake_google(codes.clone()).await;
    let (_, text_dir, data) = common::fixture(name, 100);
    let google =
        Google { auth_url: "https://google.test/auth".into(), token_url, ..Google::new("cid".into(), "secret".into()) };
    let cfg = Config {
        static_dir: None,
        text_dir,
        data,
        migrate_from: None,
        secure_cookies: false,
        google: Some(google),
        public_url: None,
        retention_ms: space::RETENTION_MS,
        min_client_protocol: api::protocol::MIN_CLIENT_PROTOCOL,
    };
    let app = router(&cfg).unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let served = app.clone();
    tokio::spawn(async move { axum::serve(listener, served).await.unwrap() });
    Srv { app, addr, codes }
}

async fn call(srv: &Srv, req: Request<Body>) -> (StatusCode, axum::http::HeaderMap, String) {
    let res = srv.app.clone().oneshot(req).await.unwrap();
    let (parts, body) = res.into_parts();
    (parts.status, parts.headers, String::from_utf8_lossy(&body.collect().await.unwrap().to_bytes()).into_owned())
}

fn set_cookies(h: &axum::http::HeaderMap) -> HashMap<String, String> {
    h.get_all(header::SET_COOKIE)
        .iter()
        .map(|v| {
            let (kv, _) = v.to_str().unwrap().split_once(';').unwrap();
            let (k, v) = kv.split_once('=').unwrap();
            (k.to_string(), v.to_string())
        })
        .collect()
}

async fn session(srv: &Srv, cookie: Option<&str>) -> (Value, Option<String>) {
    let mut req = Request::post("/api/session");
    if let Some(c) = cookie {
        req = req.header(header::COOKIE, c);
    }
    let (status, headers, body) = call(srv, req.body(Body::empty()).unwrap()).await;
    assert_eq!(status, StatusCode::OK);
    (serde_json::from_str(&body).unwrap(), set_cookies(&headers).get("bt_session").map(|t| format!("bt_session={t}")))
}

/// Run the whole browser flow as Google user `sub`, starting from session `cookie`. Returns (Location, cookie).
async fn sign_in(srv: &Srv, cookie: Option<&str>, sub: &str) -> (String, Option<String>) {
    let mut req = Request::get("/api/auth/google?return=%23john%2F3").header(header::HOST, srv.addr.to_string());
    if let Some(c) = cookie {
        req = req.header(header::COOKIE, c);
    }
    let (status, headers, _) = call(srv, req.body(Body::empty()).unwrap()).await;
    assert_eq!(status, StatusCode::FOUND);
    let to = headers[header::LOCATION].to_str().unwrap().to_string();
    assert!(to.starts_with("https://google.test/auth?"), "{to}");
    let q: HashMap<String, String> = url_query(&to);
    let redirect = format!("http://{}/api/auth/google/callback", srv.addr);
    assert_eq!(q["redirect_uri"], redirect);
    assert_eq!(q["code_challenge_method"], "S256");
    let flow = set_cookies(&headers)["bt_oauth"].clone();
    let code = format!("code-{sub}");
    srv.codes.lock().unwrap().insert(code.clone(), (q["code_challenge"].clone(), redirect, sub.to_string()));
    let mut cookies = format!("bt_oauth={flow}");
    if let Some(c) = cookie {
        cookies = format!("{c}; {cookies}");
    }
    let req = Request::get(format!("/api/auth/google/callback?code={code}&state={}", q["state"]))
        .header(header::HOST, srv.addr.to_string())
        .header(header::COOKIE, cookies)
        .body(Body::empty())
        .unwrap();
    let (status, headers, _) = call(srv, req).await;
    assert_eq!(status, StatusCode::FOUND);
    let set = set_cookies(&headers);
    assert_eq!(set["bt_oauth"], "", "the flow cookie is cleared");
    (headers[header::LOCATION].to_str().unwrap().to_string(), set.get("bt_session").map(|t| format!("bt_session={t}")))
}

fn url_query(url: &str) -> HashMap<String, String> {
    let query = url.split_once('?').unwrap().1;
    query
        .split('&')
        .map(|pair| {
            let (key, value) = pair.split_once('=').unwrap();
            let (mut out, bytes, mut i) = (vec![], value.as_bytes(), 0);
            while i < bytes.len() {
                if bytes[i] == b'%' {
                    out.push(u8::from_str_radix(&value[i + 1..i + 3], 16).unwrap());
                    i += 3;
                } else {
                    out.push(bytes[i]);
                    i += 1;
                }
            }
            (key.to_string(), String::from_utf8(out).unwrap())
        })
        .collect()
}

/// Apply ops over the sync socket and wait for their acks; returns the replayed change rows' `after`s.
async fn sync(srv: &Srv, cookie: &str, ops: &[Value]) -> Vec<Value> {
    sync_in(srv, cookie, "", ops).await
}

/// `sync` in a space (`query` = `?space=<id>`, or "" for the personal space).
async fn sync_in(srv: &Srv, cookie: &str, query: &str, ops: &[Value]) -> Vec<Value> {
    let mut req = format!("ws://{}/api/sync{query}", srv.addr).into_client_request().unwrap();
    req.headers_mut().insert(header::COOKIE, cookie.parse().unwrap());
    let (mut ws, _) = tokio_tungstenite::connect_async(req).await.unwrap();
    ws.send(Message::text(json!({"type": "hello", "after": 0}).to_string())).await.unwrap();
    let mut rows = vec![];
    loop {
        let m = next(&mut ws).await;
        if m["type"] == "ready" {
            break;
        }
        rows.extend(m["change"]["rows"].as_array().unwrap().iter().map(|r| r["after"].clone()));
    }
    for op in ops {
        ws.send(Message::text(json!({"type": "op", "op": op}).to_string())).await.unwrap();
    }
    let mut acks = 0;
    while acks < ops.len() {
        let m = next(&mut ws).await;
        assert_ne!(m["type"], "reject", "{m}");
        if m["type"] == "ack" {
            acks += 1;
        }
    }
    rows
}

async fn next<S: StreamExt<Item = Result<Message, tokio_tungstenite::tungstenite::Error>> + Unpin>(
    ws: &mut S,
) -> Value {
    let m = tokio::time::timeout(std::time::Duration::from_secs(3), ws.next()).await.unwrap().unwrap().unwrap();
    serde_json::from_str(m.to_text().unwrap()).unwrap()
}

fn tag(op_id: &str, id: &str, name: &str) -> Value {
    json!({"op_id": op_id, "kind": "tag_create", "id": id, "name": name, "color": "#336699", "now": 1})
}

fn paint(op_id: &str, tag: &str, start: u32, end: u32, new_id: &str) -> Value {
    json!({"op_id": op_id, "kind": "paint", "tag": tag, "text_id": "t1", "start": start, "end": end, "new_id": new_id, "now": 2})
}

#[tokio::test]
async fn sign_in_links_merges_and_rotates_sessions() {
    let srv = start("auth").await;
    let (s, phone) = session(&srv, None).await;
    let phone = phone.unwrap();
    assert_eq!((s["account"].clone(), s["providers"].clone()), (Value::Null, json!(["google"])));
    let phone_owner = s["owner"].as_str().unwrap().to_string();
    sync(&srv, &phone, &[tag("p1", "PG", "Grace"), paint("p2", "PG", T1, T1 + 5, "pg1")]).await;

    // First sign-in: the anonymous owner becomes the account; its data stays; the session token is replaced.
    let (to, signed) = sign_in(&srv, Some(&phone), "u1").await;
    assert_eq!(to, "/#john/3");
    let signed = signed.unwrap();
    assert_ne!(signed, phone);
    let (s, _) = session(&srv, Some(&signed)).await;
    assert_eq!(
        (s["owner"].as_str(), s["account"]["email"].as_str()),
        (Some(phone_owner.as_str()), Some("u1@example.com"))
    );
    let (s, fresh) = session(&srv, Some(&phone)).await;
    assert!(fresh.is_some() && s["owner"] != phone_owner.as_str(), "the old token no longer works");

    // Another device, anonymous with its own data, signs in to the same account: its data is merged in.
    let (_, laptop) = session(&srv, None).await;
    let laptop = laptop.unwrap();
    sync(&srv, &laptop, &[tag("l1", "LG", "grace"), paint("l2", "LG", T1 + 5, T1 + 9, "lg1"), tag("l3", "LL", "Love")])
        .await;
    let (_, laptop_signed) = sign_in(&srv, Some(&laptop), "u1").await;
    let laptop_signed = laptop_signed.unwrap();
    let (s, _) = session(&srv, Some(&laptop_signed)).await;
    assert_eq!(s["owner"].as_str(), Some(phone_owner.as_str()));
    let rows = sync(&srv, &laptop_signed, &[]).await;
    let mut live: HashMap<String, Value> = HashMap::new();
    for r in rows {
        live.insert(format!("{}:{}", r["table"], r["id"]), r);
    }
    let live_taggings: Vec<_> = live
        .values()
        .filter(|r| r["table"] == "tagging" && r["deleted_at"].is_null())
        .map(|r| (r["start"].clone(), r["end"].clone()))
        .collect();
    assert_eq!(live_taggings, [(json!(T1), json!(T1 + 9))], "same-named tags combined, abutting taggings merged");
    assert!(live.contains_key("\"tag\":\"LL\""), "other tags copied");

    // Signing in again on a signed-in device with another Google account switches to a new, empty account.
    let (_, other) = sign_in(&srv, Some(&laptop_signed), "u2").await;
    let (s, _) = session(&srv, other.as_deref()).await;
    assert_ne!(s["owner"].as_str(), Some(phone_owner.as_str()));
    assert_eq!(s["account"]["email"], "u2@example.com");

    // Logout ends the session.
    let req = Request::post("/api/logout").header(header::COOKIE, &signed).body(Body::empty()).unwrap();
    let (status, headers, _) = call(&srv, req).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    assert_eq!(set_cookies(&headers)["bt_session"], "");
    let (s, fresh) = session(&srv, Some(&signed)).await;
    assert!(fresh.is_some() && s["account"].is_null());
}

#[tokio::test]
async fn a_forged_or_stale_callback_is_refused() {
    let srv = start("auth-bad").await;
    // No flow cookie (e.g. a link from elsewhere).
    let req = Request::get("/api/auth/google/callback?code=x&state=y").body(Body::empty()).unwrap();
    let (status, headers, _) = call(&srv, req).await;
    assert_eq!(status, StatusCode::FOUND);
    assert_eq!(headers[header::LOCATION], "/?auth_error=expired");
    assert!(!set_cookies(&headers).contains_key("bt_session"));
    // State mismatch.
    let req = Request::get("/api/auth/google/callback?code=x&state=y")
        .header(header::COOKIE, format!("bt_oauth=z.v.{}", URL_SAFE_NO_PAD.encode("#gen/1")))
        .body(Body::empty())
        .unwrap();
    let (_, headers, _) = call(&srv, req).await;
    assert_eq!(headers[header::LOCATION], "/?auth_error=expired#gen/1");
    // The user cancelled at Google.
    let req = Request::get("/api/auth/google/callback?error=access_denied").body(Body::empty()).unwrap();
    let (_, headers, _) = call(&srv, req).await;
    assert_eq!(headers[header::LOCATION], "/?auth_error=cancelled");
}

async fn api(srv: &Srv, cookie: &str, method: &str, path: &str, body: Value) -> Value {
    let req = Request::builder()
        .method(method)
        .uri(format!("/api{path}"))
        .header(header::COOKIE, cookie)
        .body(Body::from(body.to_string()))
        .unwrap();
    let (status, _, body) = call(srv, req).await;
    assert_eq!(status, StatusCode::OK, "{method} {path}: {body}");
    serde_json::from_str(&body).unwrap_or(Value::Null)
}

#[tokio::test]
async fn signing_in_keeps_the_groups_joined_while_anonymous() {
    let srv = start("auth-groups").await;
    // An account that runs a group, and a new device that joins it anonymously and comments.
    let (_, ann) = session(&srv, None).await;
    let (ann_signed, phone) =
        (sign_in(&srv, ann.as_deref(), "ann").await.1.unwrap(), session(&srv, None).await.1.unwrap());
    let g = api(&srv, &ann_signed, "POST", "/groups", json!({"name": "Study"})).await;
    let id = g["id"].as_str().unwrap().to_string();
    let edit = api(&srv, &ann_signed, "POST", &format!("/groups/{id}/invites"), json!({"role": "edit"})).await
        ["invites"]["edit"]
        .clone();
    api(&srv, &phone, "POST", "/join", json!({"token": edit})).await;
    let q = format!("?space={id}");
    let comment =
        json!({"op_id": "c", "kind": "comment_create", "id": "c1", "tagging": "g1", "body": "Note", "now": 3});
    sync_in(&srv, &phone, &q, &[tag("t", "T", "Grace"), paint("p", "T", T1, T1 + 3, "g1"), comment]).await;
    // Bob already has an account (on another device); the phone signs in to it.
    let (_, bob) = session(&srv, None).await;
    let bob_signed = sign_in(&srv, bob.as_deref(), "bob").await.1.unwrap();
    let phone_signed = sign_in(&srv, Some(&phone), "bob").await.1.unwrap();

    let groups = api(&srv, &bob_signed, "GET", "/groups", Value::Null).await;
    assert_eq!(groups["groups"], json!([{"id": id, "name": "Study", "role": "edit"}]), "the membership came along");
    let rows = sync_in(&srv, &phone_signed, &q, &[]).await;
    let bob_id = session(&srv, Some(&bob_signed)).await.0["owner"].as_str().unwrap().to_string();
    let authors: Vec<_> = rows.iter().filter(|r| r["table"] == "comment").map(|r| r["author"].clone()).collect();
    assert_eq!(authors.last(), Some(&json!(bob_id)), "the comment is bob's now (the replay ends with the reassign)");
    let update = json!({"op_id": "u", "kind": "comment_update", "id": "c1", "body": "Edited", "now": 4});
    sync_in(&srv, &bob_signed, &q, &[update]).await; // panics on a reject (not_author)
}
