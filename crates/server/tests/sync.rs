//! End-to-end sync over real `WebSocket`s against a live server.
mod common;

use axum::body::Body;
use axum::http::{header, Request, StatusCode};
use futures_util::{SinkExt, StreamExt};
use serde_json::{json, Value};
use server::{router, Config};
use std::time::Duration;
use tokio::net::TcpStream;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream};
use tower::ServiceExt;

const T1: u32 = 2 << 24 | 1 << 16 | 1 << 8; // 1:1, the first word of the fixture text (100 words)

struct Server {
    app: axum::Router,
    addr: std::net::SocketAddr,
}

async fn start(cfg: &Config) -> Server {
    let app = router(cfg).unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let served = app.clone();
    tokio::spawn(async move { axum::serve(listener, served).await.unwrap() });
    Server { app, addr }
}

fn config(name: &str) -> Config {
    let (_, text_dir, data) = common::fixture(name, 100);
    Config {
        static_dir: None,
        text_dir,
        data,
        migrate_from: None,
        secure_cookies: false,
        google: None,
        public_url: None,
        retention_ms: space::RETENTION_MS,
        min_client_protocol: api::protocol::MIN_CLIENT_PROTOCOL,
    }
}

/// A new anonymous owner: returns its session cookie (`name=value`).
async fn session(srv: &Server) -> String {
    let req = Request::post("/api/session").body(Body::empty()).unwrap();
    let res = srv.app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let cookie = res.headers()[header::SET_COOKIE].to_str().unwrap().to_string();
    assert!(cookie.contains("HttpOnly") && cookie.contains("SameSite=Lax") && !cookie.contains("Secure"));
    cookie.split(';').next().unwrap().to_string()
}

struct Client(WebSocketStream<MaybeTlsStream<TcpStream>>);

async fn connect(srv: &Server, cookie: Option<&str>, origin: Option<&str>) -> Result<Client, StatusCode> {
    connect_to(srv, cookie, origin, "").await
}

/// `query`: e.g. `?space=<group>`.
async fn connect_to(
    srv: &Server,
    cookie: Option<&str>,
    origin: Option<&str>,
    query: &str,
) -> Result<Client, StatusCode> {
    let mut req = format!("ws://{}/api/sync{query}", srv.addr).into_client_request().unwrap();
    if let Some(c) = cookie {
        req.headers_mut().insert(header::COOKIE, c.parse().unwrap());
    }
    if let Some(o) = origin {
        req.headers_mut().insert(header::ORIGIN, o.parse().unwrap());
    }
    match tokio_tungstenite::connect_async(req).await {
        Ok((ws, _)) => Ok(Client(ws)),
        Err(tokio_tungstenite::tungstenite::Error::Http(res)) => Err(res.status()),
        Err(e) => panic!("{e}"),
    }
}

impl Client {
    async fn send(&mut self, v: Value) {
        self.0.send(Message::text(v.to_string())).await.unwrap();
    }
    async fn recv(&mut self) -> Value {
        let msg =
            tokio::time::timeout(Duration::from_secs(3), self.0.next()).await.expect("timed out").unwrap().unwrap();
        serde_json::from_str(msg.to_text().unwrap()).unwrap()
    }
    /// Nothing else arrives within a short wait.
    async fn quiet(&mut self) {
        let r = tokio::time::timeout(Duration::from_millis(200), self.0.next()).await;
        assert!(r.is_err(), "unexpected message: {r:?}");
    }
    /// Say hello as the client does (taking `history` messages); returns the replayed changes.
    async fn hello(&mut self, after: u64) -> Vec<Value> {
        self.send(json!({"type": "hello", "after": after, "history": true})).await;
        let mut changes = vec![];
        loop {
            let msg = self.recv().await;
            match msg["type"].as_str() {
                Some("change") => changes.push(msg["change"].clone()),
                Some("ready") => return changes,
                _ => panic!("unexpected {msg}"),
            }
        }
    }
    async fn op(&mut self, op: Value) {
        self.send(json!({"type": "op", "op": op})).await;
    }
}

fn tag_create(op_id: &str, id: &str, name: &str) -> Value {
    json!({"op_id": op_id, "kind": "tag_create", "id": id, "name": name, "color": "#336699", "now": 1})
}

fn paint(op_id: &str, tag: &str, start: u32, end: u32, new_id: &str) -> Value {
    json!({"op_id": op_id, "kind": "paint", "tag": tag, "text_id": "t1", "start": start, "end": end, "new_id": new_id, "now": 2})
}

#[tokio::test]
async fn ops_fan_out_to_every_socket_of_the_owner_in_seq_order() {
    let srv = start(&config("fanout")).await;
    let cookie = session(&srv).await;
    let mut a = connect(&srv, Some(&cookie), None).await.unwrap();
    let mut b = connect(&srv, Some(&cookie), None).await.unwrap();
    assert_eq!(a.hello(0).await, Vec::<Value>::new());
    assert_eq!(b.hello(0).await, Vec::<Value>::new());

    a.op(tag_create("op1", "T", "Grace")).await;
    a.op(paint("op2", "T", T1 + 10, T1 + 20, "g1")).await;
    // `a` gets acks and changes (interleaving between the two kinds is unspecified); `b` gets the changes in order.
    let mut acks = vec![];
    let mut seqs = vec![];
    while acks.len() < 2 || seqs.len() < 2 {
        let msg = a.recv().await;
        match msg["type"].as_str().unwrap() {
            "ack" => acks.push((msg["op_id"].as_str().unwrap().to_string(), msg["seq"].as_u64().unwrap())),
            "change" => seqs.push(msg["change"]["seq"].as_u64().unwrap()),
            t => panic!("unexpected {t}"),
        }
    }
    assert_eq!(acks, [("op1".to_string(), 1), ("op2".to_string(), 2)]);
    assert_eq!(seqs, [1, 2]);
    for seq in [1, 2] {
        let msg = b.recv().await;
        assert_eq!((msg["type"].as_str(), msg["change"]["seq"].as_u64()), (Some("change"), Some(seq)));
    }
    b.quiet().await;
}

#[tokio::test]
async fn reconnect_replays_exactly_what_was_missed() {
    let srv = start(&config("replay")).await;
    let cookie = session(&srv).await;
    let mut a = connect(&srv, Some(&cookie), None).await.unwrap();
    a.hello(0).await;
    a.op(tag_create("op1", "T", "Grace")).await;
    a.op(paint("op2", "T", T1, T1 + 5, "g1")).await;
    a.op(paint("op3", "T", T1 + 50, T1 + 55, "g2")).await;
    for _ in 0..6 {
        a.recv().await; // 3 acks + 3 changes
    }
    let mut late = connect(&srv, Some(&cookie), None).await.unwrap();
    let replay = late.hello(1).await;
    assert_eq!(replay.iter().map(|c| c["seq"].as_u64().unwrap()).collect::<Vec<_>>(), [2, 3]);
    assert_eq!(replay[0]["rows"][0]["after"]["id"], "g1");
    late.quiet().await;
}

#[tokio::test]
async fn duplicate_ops_are_acked_once_and_never_reapplied() {
    let srv = start(&config("dup")).await;
    let cookie = session(&srv).await;
    let mut a = connect(&srv, Some(&cookie), None).await.unwrap();
    a.hello(0).await;
    a.op(tag_create("op1", "T", "Grace")).await;
    a.recv().await;
    a.recv().await;
    a.op(tag_create("op1", "T", "Grace")).await; // resent after a reconnect, say
    let msg = a.recv().await;
    assert_eq!((msg["type"].as_str(), msg["seq"].as_u64()), (Some("ack"), Some(1)));
    a.quiet().await;
}

#[tokio::test]
async fn invalid_ops_are_rejected_with_stable_codes() {
    let srv = start(&config("reject")).await;
    let cookie = session(&srv).await;
    let mut a = connect(&srv, Some(&cookie), None).await.unwrap();
    a.hello(0).await;
    a.op(tag_create("op1", "T", "Grace")).await;
    a.recv().await;
    a.recv().await;
    let cases = [
        (paint("p1", "nope", T1, T1 + 1, "g1"), "unknown_tag"),
        (paint("p2", "T", T1 + 99, T1 + 101, "g2"), "bad_text"), // runs past the book's end (a gap)
        (paint("p3", "T", 0, 5, "g3"), "bad_text"),
        (
            json!({"op_id": "p4", "kind": "paint", "tag": "T", "text_id": "other", "start": T1, "end": T1 + 1, "new_id": "g4", "now": 1}),
            "bad_text",
        ),
        (paint("p5", "T", T1, T1 + 1, "has space"), "bad_id"),
        (tag_create("p6", "U", "grace"), "name_taken"),
        (json!({"op_id": "p7", "kind": "revert", "target": "nope", "now": 1}), "unknown_op"),
    ];
    for (op, code) in cases {
        let id = op["op_id"].as_str().unwrap().to_string();
        a.op(op).await;
        let msg = a.recv().await;
        assert_eq!(
            (msg["type"].as_str(), msg["op_id"].as_str(), msg["code"].as_str()),
            (Some("reject"), Some(id.as_str()), Some(code))
        );
    }
    a.quiet().await;
}

#[tokio::test]
async fn owners_are_isolated() {
    let srv = start(&config("owners")).await;
    let (alice, bob) = (session(&srv).await, session(&srv).await);
    assert_ne!(alice, bob);
    let mut a = connect(&srv, Some(&alice), None).await.unwrap();
    let mut b = connect(&srv, Some(&bob), None).await.unwrap();
    a.hello(0).await;
    b.hello(0).await;
    a.op(tag_create("op1", "T", "Grace")).await;
    a.recv().await;
    a.recv().await;
    b.quiet().await; // nothing of alice's reaches bob
    b.op(paint("op1", "T", T1, T1 + 1, "g1")).await; // alice's tag id, alice's op id
    let msg = b.recv().await;
    assert_eq!(msg["code"], "unknown_tag");
    // Same op id and tag id as alice's: fine, each owner has their own space (ids are unique per space).
    b.op(tag_create("op1", "T", "Mine")).await;
    let msg = b.recv().await;
    assert_eq!((msg["type"].as_str(), msg["seq"].as_u64()), (Some("ack"), Some(1)));
    assert_eq!(b.recv().await["change"]["rows"][0]["after"]["name"], "Mine");
    a.quiet().await;
}

#[tokio::test]
async fn revert_round_trips_through_the_server() {
    let srv = start(&config("revert")).await;
    let cookie = session(&srv).await;
    let mut a = connect(&srv, Some(&cookie), None).await.unwrap();
    a.hello(0).await;
    a.op(tag_create("op1", "T", "Grace")).await;
    a.op(paint("op2", "T", T1, T1 + 5, "g1")).await;
    a.op(json!({"op_id": "op3", "kind": "revert", "target": "op2", "now": 9})).await;
    let mut changes = vec![];
    while changes.len() < 3 {
        let msg = a.recv().await;
        if msg["type"] == "change" {
            changes.push(msg["change"].clone());
        }
    }
    assert_eq!(changes[2]["kind"], "revert");
    assert_eq!(changes[2]["rows"][0]["after"]["deleted_at"], 9);
}

#[tokio::test]
async fn sync_requires_a_session_and_same_origin() {
    let srv = start(&config("auth")).await;
    assert_eq!(connect(&srv, None, None).await.err(), Some(StatusCode::UNAUTHORIZED));
    assert_eq!(connect(&srv, Some("bt_session=forged"), None).await.err(), Some(StatusCode::UNAUTHORIZED));
    let cookie = session(&srv).await;
    assert_eq!(connect(&srv, Some(&cookie), Some("https://evil.example")).await.err(), Some(StatusCode::FORBIDDEN));
    let same = format!("http://{}", srv.addr);
    assert!(connect(&srv, Some(&cookie), Some(&same)).await.is_ok());
    // An existing session is reused, not replaced.
    let req = Request::post("/api/session").header(header::COOKIE, &cookie).body(Body::empty()).unwrap();
    let res = srv.app.clone().oneshot(req).await.unwrap();
    assert!(res.headers().get(header::SET_COOKIE).is_none());
}

#[tokio::test]
async fn changes_survive_a_restart() {
    let cfg = config("restart");
    let cookie = {
        let srv = start(&cfg).await;
        let cookie = session(&srv).await;
        let mut a = connect(&srv, Some(&cookie), None).await.unwrap();
        a.hello(0).await;
        a.op(tag_create("op1", "T", "Grace")).await;
        a.recv().await;
        a.recv().await;
        cookie
    };
    let srv = start(&cfg).await; // a new store on the same database file
    let mut a = connect(&srv, Some(&cookie), None).await.unwrap();
    let replay = a.hello(0).await;
    assert_eq!(replay.len(), 1);
    assert_eq!(replay[0]["rows"][0]["after"]["name"], "Grace");
}

#[tokio::test]
async fn a_pre_spaces_database_is_imported_with_its_seqs() {
    use sha2::Digest;
    let mut cfg = config("legacy");
    let old = cfg.data.with_file_name("old.db");
    {
        // The old layout: every owner in one database, seq per owner.
        let c = rusqlite::Connection::open(&old).unwrap();
        c.execute_batch(
            "CREATE TABLE owners (id TEXT PRIMARY KEY, created_at INTEGER NOT NULL);
             CREATE TABLE sessions (token_hash TEXT PRIMARY KEY, owner_id TEXT NOT NULL, created_at INTEGER NOT NULL);
             CREATE TABLE tags (id TEXT PRIMARY KEY, owner_id TEXT NOT NULL, name TEXT NOT NULL, color TEXT NOT NULL,
               description TEXT, created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL, deleted_at INTEGER);
             CREATE TABLE taggings (id TEXT PRIMARY KEY, owner_id TEXT NOT NULL, tag_id TEXT NOT NULL,
               text_id TEXT NOT NULL, start_ord INTEGER NOT NULL, end_ord INTEGER NOT NULL,
               created_at INTEGER NOT NULL, deleted_at INTEGER, merged_into TEXT);
             CREATE TABLE palettes (owner_id TEXT PRIMARY KEY, tags TEXT NOT NULL, updated_at INTEGER NOT NULL);
             CREATE TABLE comments (id TEXT PRIMARY KEY, owner_id TEXT NOT NULL, tagging_id TEXT NOT NULL,
               body TEXT NOT NULL, created_at INTEGER NOT NULL, edited_at INTEGER, deleted_at INTEGER);
             CREATE TABLE changes (owner_id TEXT NOT NULL, seq INTEGER NOT NULL, op_id TEXT NOT NULL,
               kind TEXT NOT NULL, payload TEXT NOT NULL, created_at INTEGER NOT NULL, PRIMARY KEY (owner_id, seq));",
        )
        .unwrap();
        let hash = format!("{:x}", sha2::Sha256::digest(b"tok"));
        let tag = json!({"table": "tag", "id": "T", "name": "Grace", "color": "#336699", "created_at": 1, "updated_at": 1, "deleted_at": null});
        let tagging = json!({"table": "tagging", "id": "g1", "tag": "T", "text_id": "t1", "start": T1, "end": T1 + 5, "created_at": 2, "deleted_at": null, "merged_into": null});
        let changes = [
            json!({"seq": 1, "op_id": "op1", "kind": "tag_create", "rows": [{"before": null, "after": tag}]}),
            json!({"seq": 2, "op_id": "op2", "kind": "paint", "rows": [{"before": null, "after": tagging}]}),
        ];
        c.execute_batch(&format!(
            "INSERT INTO owners VALUES ('alice', 0), ('bob', 0);
             INSERT INTO sessions VALUES ('{hash}', 'alice', 0);
             INSERT INTO tags VALUES ('T', 'alice', 'Grace', '#336699', NULL, 1, 1, NULL);
             INSERT INTO taggings VALUES ('g1', 'alice', 'T', 't1', {}, {}, 2, NULL, NULL);
             INSERT INTO palettes VALUES ('alice', '[\"T\"]', 3);
             INSERT INTO comments VALUES ('c1', 'alice', 'g1', 'hi', 4, NULL, NULL);
             INSERT INTO changes VALUES ('alice', 1, 'op1', 'tag_create', '{}', 0), ('alice', 2, 'op2', 'paint', '{}', 0);
             INSERT INTO changes VALUES ('bob', 1, 'b1', 'tag_create', '{{\"seq\":1,\"op_id\":\"b1\",\"kind\":\"tag_create\",\"rows\":[]}}', 0);",
            T1,
            T1 + 5,
            changes[0],
            changes[1]
        ))
        .unwrap();
    }
    cfg.migrate_from = Some(old.clone());
    let srv = start(&cfg).await;
    let mut a = connect(&srv, Some("bt_session=tok"), None).await.unwrap();
    let replay = a.hello(0).await;
    assert_eq!(replay.iter().map(|c| c["op_id"].as_str().unwrap()).collect::<Vec<_>>(), ["op1", "op2"]);
    // The imported rows are live: painting next to g1 merges into it, and seqs continue.
    a.op(paint("op3", "T", T1 + 5, T1 + 8, "g2")).await;
    let mut got = vec![];
    while got.len() < 2 {
        let m = a.recv().await;
        got.push(m);
    }
    let change = got.iter().find(|m| m["type"] == "change").unwrap();
    assert_eq!(change["change"]["seq"], 3);
    assert_eq!(change["change"]["rows"][0]["after"]["end"], T1 + 8);
    assert!(cfg.data.join("spaces/bob.sqlite").exists());
    drop(srv);
    // Never imported twice: the data directory exists now.
    std::fs::remove_file(&old).unwrap();
    start(&cfg).await;
}

/// A groups API call (`/api/groups...`, `/api/join`).
async fn api(srv: &Server, cookie: &str, method: &str, path: &str, body: Value) -> (StatusCode, Value) {
    let req = Request::builder()
        .method(method)
        .uri(format!("/api{path}"))
        .header(header::COOKIE, cookie)
        .body(Body::from(body.to_string()))
        .unwrap();
    let res = srv.app.clone().oneshot(req).await.unwrap();
    let status = res.status();
    let bytes = axum::body::to_bytes(res.into_body(), 1 << 20).await.unwrap();
    (status, serde_json::from_slice(&bytes).unwrap_or(Value::Null))
}

/// Read `n` messages, skipping acks; returns the rest.
async fn non_acks(c: &mut Client, n: usize) -> Vec<Value> {
    let mut out = vec![];
    while out.len() < n {
        let m = c.recv().await;
        if m["type"] != "ack" {
            out.push(m);
        }
    }
    out
}

#[tokio::test]
#[allow(clippy::many_single_char_names)]
async fn groups_share_rows_with_roles() {
    let srv = start(&config("groups")).await;
    let (ann, bob, cat, dan) = (session(&srv).await, session(&srv).await, session(&srv).await, session(&srv).await);
    let (_, g) = api(&srv, &ann, "POST", "/groups", json!({"name": "Study"})).await;
    let id = g["id"].as_str().unwrap().to_string();
    let invites = format!("/groups/{id}/invites");
    let (_, g) = api(&srv, &ann, "POST", &invites, json!({"role": "read"})).await;
    let read = g["invites"]["read"].clone();
    let (_, g) = api(&srv, &ann, "POST", &invites, json!({"role": "edit"})).await;
    let edit = g["invites"]["edit"].clone();
    assert_eq!(api(&srv, &bob, "POST", "/join", json!({"token": read})).await.1["role"], "read");
    assert_eq!(api(&srv, &cat, "POST", "/join", json!({"token": edit})).await.1["role"], "edit");
    let (_, list) = api(&srv, &cat, "GET", "/groups", Value::Null).await;
    assert_eq!(list["groups"][0]["name"], "Study");

    let q = format!("?space={id}");
    assert_eq!(connect_to(&srv, Some(&dan), None, &q).await.err(), Some(StatusCode::FORBIDDEN), "not a member");
    let mut a = connect_to(&srv, Some(&ann), None, &q).await.unwrap();
    let mut b = connect_to(&srv, Some(&bob), None, &q).await.unwrap();
    let mut c = connect_to(&srv, Some(&cat), None, &q).await.unwrap();
    for x in [&mut a, &mut b, &mut c] {
        assert_eq!(x.hello(0).await.len(), 0);
    }
    // Ann's tag is shared: Cat paints with it, Bob (read only) sees everything live but can't write.
    a.op(tag_create("t", "T", "Grace")).await;
    c.op(paint("p", "T", T1 + 1, T1 + 5, "g1")).await;
    c.op(json!({"op_id": "c", "kind": "comment_create", "id": "c1", "tagging": "g1", "body": "Hi", "now": 3})).await;
    let seen = non_acks(&mut b, 3).await;
    assert_eq!(seen.iter().map(|m| m["change"]["seq"].as_u64().unwrap()).collect::<Vec<_>>(), [1, 2, 3]);
    let (_, me) = api(&srv, &cat, "POST", "/session", Value::Null).await;
    assert_eq!(seen[2]["change"]["rows"][0]["after"]["author"], me["owner"], "comments carry their author");
    b.op(paint("pb", "T", T1 + 20, T1 + 22, "g2")).await;
    assert_eq!(b.recv().await["code"], "read_only");
    // Only its author can change a comment.
    a.op(json!({"op_id": "e", "kind": "comment_update", "id": "c1", "body": "Mine now", "now": 4})).await;
    assert_eq!(non_acks(&mut a, 4).await[3]["code"], "not_author");
    // The group's rows are not Ann's personal rows.
    let mut personal = connect(&srv, Some(&ann), None).await.unwrap();
    assert_eq!(personal.hello(0).await.len(), 0);

    // Demoting Cat disconnects her; she reconnects as a reader.
    let (status, _) = api(
        &srv,
        &ann,
        "PUT",
        &format!("/groups/{id}/members/{}", me["owner"].as_str().unwrap()),
        json!({"role": "read"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let kicked = non_acks(&mut c, 4).await; // the three changes, then the error
    assert_eq!(kicked[3]["type"], "error");
    let mut c = connect_to(&srv, Some(&cat), None, &q).await.unwrap();
    assert_eq!(c.hello(0).await.len(), 3);
    c.op(paint("pc", "T", T1 + 30, T1 + 32, "g3")).await;
    assert_eq!(c.recv().await["code"], "read_only");
    // Leaving revokes access.
    let (_, bob_me) = api(&srv, &bob, "POST", "/session", Value::Null).await;
    api(&srv, &bob, "DELETE", &format!("/groups/{id}/members/{}", bob_me["owner"].as_str().unwrap()), Value::Null)
        .await;
    assert_eq!(b.recv().await["type"], "error");
    assert_eq!(connect_to(&srv, Some(&bob), None, &q).await.err(), Some(StatusCode::FORBIDDEN));
}

#[tokio::test]
async fn a_client_behind_the_retention_window_gets_a_reset() {
    let mut cfg = config("retention");
    cfg.retention_ms = 0; // every change before the newest is pruned by the next op
    let srv = start(&cfg).await;
    let cookie = session(&srv).await;
    let mut a = connect(&srv, Some(&cookie), None).await.unwrap();
    a.hello(0).await;
    for (i, op) in [tag_create("op1", "T", "Grace"), paint("op2", "T", T1, T1 + 5, "g1")].into_iter().enumerate() {
        if i > 0 {
            tokio::time::sleep(Duration::from_millis(5)).await; // so op1 is strictly older
        }
        a.op(op).await;
        a.recv().await;
        a.recv().await; // ack + change
    }
    // From seq 0, op1's change can't be replayed: the whole state instead, then live as usual.
    let mut late = connect(&srv, Some(&cookie), None).await.unwrap();
    late.send(json!({"type": "hello", "after": 0, "pending": ["op1", "never-sent"]})).await;
    let reset = late.recv().await;
    assert_eq!((reset["type"].as_str(), reset["seq"].as_u64()), (Some("reset"), Some(2)));
    let ids: Vec<_> = reset["rows"].as_array().unwrap().iter().map(|r| r["id"].as_str().unwrap()).collect();
    assert_eq!(ids, ["T", "g1"]);
    assert_eq!(reset["applied"], json!(["op1"]), "which of the client's ops are already in it");
    assert_eq!(reset["changes"], json!([]));
    assert_eq!(late.recv().await, ready(2));
    // A re-sent pruned op is still a duplicate: acked with its old seq, never reapplied.
    late.op(tag_create("op1", "T", "Grace")).await;
    assert_eq!(late.recv().await, json!({"type": "ack", "op_id": "op1", "seq": 1}));
    late.quiet().await;
    // From seq 1 (the pruning point) the rest still replays normally.
    let mut recent = connect(&srv, Some(&cookie), None).await.unwrap();
    assert_eq!(recent.hello(1).await.iter().map(|c| c["seq"].as_u64().unwrap()).collect::<Vec<_>>(), [2]);
}

/// A `ready`: it carries the server's build (shown in the client's menu).
fn ready(seq: u64) -> Value {
    json!({"type": "ready", "seq": seq, "build": api::build()})
}

#[tokio::test]
async fn a_client_ahead_of_a_restored_space_gets_a_reset() {
    let cfg = config("restored");
    let srv = start(&cfg).await;
    let cookie = session(&srv).await;
    let mut a = connect(&srv, Some(&cookie), None).await.unwrap();
    a.hello(0).await;
    for op in [tag_create("op1", "T", "Grace"), paint("op2", "T", T1, T1 + 5, "g1")] {
        a.op(op).await;
        a.recv().await;
        a.recv().await; // ack + change
    }
    // Say the space was restored from a backup taken at seq 2, after the client saw seqs 3 and 4 (and then the
    // space went on to a different seq 2: the client's is "lost-op"). Replaying would skip changes it thinks it has.
    for (after, last_op) in [(4, json!("op4")), (4, Value::Null), (2, json!("lost-op"))] {
        let mut c = connect(&srv, Some(&cookie), None).await.unwrap();
        c.send(json!({"type": "hello", "after": after, "last_op": last_op, "pending": []})).await;
        let reset = c.recv().await;
        assert_eq!((reset["type"].as_str(), reset["seq"].as_u64()), (Some("reset"), Some(2)), "after {after}");
        assert_eq!(reset["last_op"], "op2");
        assert_eq!(reset["rows"].as_array().unwrap().len(), 2);
        assert_eq!(c.recv().await, ready(2));
    }
    // The same change at its seq replays as usual.
    let mut c = connect(&srv, Some(&cookie), None).await.unwrap();
    c.send(json!({"type": "hello", "after": 1, "last_op": "op1", "pending": []})).await;
    assert_eq!(c.recv().await["change"]["seq"], 2);
    assert_eq!(c.recv().await, ready(2));
}

#[tokio::test]
async fn a_space_file_is_created_by_its_first_write_not_by_reading_it() {
    let cfg = config("lazy-spaces");
    let srv = start(&cfg).await;
    let files = || {
        std::fs::read_dir(cfg.data.join("spaces"))
            .map_or(0, |d| d.filter(|e| e.as_ref().unwrap().path().extension().is_some_and(|x| x == "sqlite")).count())
    };
    let cookie = session(&srv).await;
    let mut a = connect(&srv, Some(&cookie), None).await.unwrap();
    assert_eq!(a.hello(0).await.len(), 0);
    assert_eq!(files(), 0, "a visitor who only reads leaves nothing on disk");
    a.op(tag_create("op1", "T", "Grace")).await;
    assert_eq!(a.recv().await["type"], "ack");
    assert_eq!(files(), 1);
    let mut b = connect(&srv, Some(&cookie), None).await.unwrap();
    assert_eq!(b.hello(0).await.len(), 1, "and it reads like any other");
}

#[tokio::test]
async fn a_storage_failure_closes_the_socket_instead_of_rejecting_the_op() {
    let cfg = config("storage-failure");
    let srv = start(&cfg).await;
    let cookie = session(&srv).await;
    let mut a = connect(&srv, Some(&cookie), None).await.unwrap();
    a.hello(0).await;
    a.op(tag_create("op1", "T", "Grace")).await;
    a.recv().await;
    a.recv().await;
    // Break the space's database behind the server's back.
    let file = std::fs::read_dir(cfg.data.join("spaces"))
        .unwrap()
        .map(|e| e.unwrap().path())
        .find(|p| p.extension().is_some_and(|x| x == "sqlite"))
        .unwrap();
    rusqlite::Connection::open(&file).unwrap().execute_batch("DROP TABLE changes").unwrap();
    a.op(paint("op2", "T", T1, T1 + 5, "g1")).await;
    // Not a reject (the client would drop the op for good): an error, then the server closes; the client reconnects
    // and re-sends.
    assert_eq!(a.recv().await, json!({"type": "error", "message": "storage error; reconnect"}));
    let end = tokio::time::timeout(Duration::from_secs(3), a.0.next()).await.expect("timed out");
    assert!(matches!(end, None | Some(Ok(Message::Close(_)) | Err(_))), "closed: {end:?}");
}

#[tokio::test]
async fn ready_says_which_client_is_served_now() {
    let mut cfg = config("client-version");
    let (dist, _, _) = common::fixture("client-version-dist", 100);
    let version = dist.join(api::CLIENT_VERSION_FILE);
    std::fs::write(&version, "aaaa\n").unwrap();
    cfg.static_dir = Some(dist.clone());
    let srv = start(&cfg).await;
    let cookie = session(&srv).await;
    let ready = || async {
        let mut c = connect(&srv, Some(&cookie), None).await.unwrap();
        c.send(json!({"type": "hello", "after": 0})).await;
        c.recv().await
    };
    assert_eq!(ready().await["client"], "aaaa");
    let mut open = connect(&srv, Some(&cookie), None).await.unwrap();
    open.send(json!({"type": "hello", "after": 0})).await;
    assert_eq!(open.recv().await["client"], "aaaa");
    // A new client is put in place without restarting the server: the next connection says so, and so does a
    // socket that stayed open, when it pings.
    std::fs::write(&version, "bbbb\n").unwrap();
    assert_eq!(ready().await["client"], "bbbb");
    open.send(json!({"type": "ping"})).await;
    assert_eq!(open.recv().await, json!({"type": "pong", "client": "bbbb"}));
}

#[tokio::test]
async fn a_client_below_the_minimum_protocol_is_told_it_is_outdated() {
    let mut cfg = config("outdated");
    cfg.min_client_protocol = 2;
    let srv = start(&cfg).await;
    let cookie = session(&srv).await;
    for hello in [json!({"type": "hello", "after": 0}), json!({"type": "hello", "after": 0, "protocol": 1})] {
        let mut c = connect(&srv, Some(&cookie), None).await.unwrap();
        c.send(hello).await;
        assert_eq!(c.recv().await, json!({"type": "outdated"}));
        let end = tokio::time::timeout(Duration::from_secs(3), c.0.next()).await.expect("timed out");
        assert!(matches!(end, None | Some(Ok(Message::Close(_)) | Err(_))), "then closed: {end:?}");
    }
    let mut c = connect(&srv, Some(&cookie), None).await.unwrap();
    c.send(json!({"type": "hello", "after": 0, "protocol": 2})).await;
    assert_eq!(c.recv().await["type"], "ready");
}

#[tokio::test]
async fn a_ping_gets_a_pong_before_and_after_hello() {
    let cfg = config("ping");
    let srv = start(&cfg).await;
    let cookie = session(&srv).await;
    let mut c = connect(&srv, Some(&cookie), None).await.unwrap();
    c.send(json!({"type": "ping"})).await;
    assert_eq!(c.recv().await, json!({"type": "pong"}));
    c.send(json!({"type": "hello", "after": 0})).await;
    assert_eq!(c.recv().await["type"], "ready");
    c.send(json!({"type": "ping"})).await;
    assert_eq!(c.recv().await, json!({"type": "pong"}));
    c.quiet().await;
}

/// Make `cookie`'s owner an account (as signing in would); returns the owner.
async fn make_account(srv: &Server, cfg: &Config, cookie: &str, subject: &str) -> String {
    let (_, me) = api(srv, cookie, "POST", "/session", Value::Null).await;
    let owner = me["owner"].as_str().unwrap().to_string();
    let db = rusqlite::Connection::open(cfg.data.join("accounts.sqlite")).unwrap();
    db.execute(
        "INSERT INTO identities (provider, subject, owner_id, created_at) VALUES ('google', ?1, ?2, 0)",
        (subject, &owner),
    )
    .unwrap();
    owner
}

fn entry(id: &str, ord: u32, at: i64) -> Value {
    json!({"id": id, "anchor": {"ord": ord, "at": at}, "last": {"ord": ord, "at": at}, "gone": false})
}

#[tokio::test]
async fn reading_history_syncs_between_an_accounts_devices_only() {
    let cfg = config("history");
    let srv = start(&cfg).await;
    let cookie = session(&srv).await;
    // Anonymous: no history from the server, and what it sends is ignored (it stays on the device).
    let mut anon = connect(&srv, Some(&cookie), None).await.unwrap();
    assert_eq!(anon.hello(0).await.len(), 0);
    anon.send(json!({"type": "history", "entries": [entry("x", T1, 1)]})).await;
    anon.quiet().await;
    drop(anon);

    make_account(&srv, &cfg, &cookie, "alice").await;
    let mut a = connect(&srv, Some(&cookie), None).await.unwrap();
    let mut b = connect(&srv, Some(&cookie), None).await.unwrap();
    for c in [&mut a, &mut b] {
        assert_eq!(c.hello(0).await.len(), 0);
        assert_eq!(c.recv().await, json!({"type": "history", "entries": []}), "the anonymous entry wasn't kept");
    }
    // A client from before reading history doesn't ask for it, and gets none (one that did dropped the socket and
    // reconnected forever).
    let mut old = connect(&srv, Some(&cookie), None).await.unwrap();
    old.send(json!({"type": "hello", "after": 0})).await;
    assert_eq!(old.recv().await["type"], "ready");
    a.send(json!({"type": "history", "entries": [entry("h1", T1, 10)]})).await;
    for c in [&mut a, &mut b] {
        assert_eq!(
            c.recv().await,
            json!({"type": "history", "entries": [entry("h1", T1, 10)]}),
            "every device hears it"
        );
    }
    // An older copy changes nothing; a time far in the future is refused.
    b.send(json!({"type": "history", "entries": [entry("h1", T1 + 5, 5), entry("later", T1, 4_000_000_000_000_000)]}))
        .await;
    a.quiet().await;
    old.quiet().await;
    let mut again = connect(&srv, Some(&cookie), None).await.unwrap();
    assert_eq!(again.hello(0).await.len(), 0, "history is not a change");
    assert_eq!(again.recv().await, json!({"type": "history", "entries": [entry("h1", T1, 10)]}));
}

#[tokio::test]
async fn a_member_who_leaves_a_group_leaves_no_history_there() {
    let cfg = config("history-groups");
    let srv = start(&cfg).await;
    let (ann, bob) = (session(&srv).await, session(&srv).await);
    let bob_owner = make_account(&srv, &cfg, &bob, "bob").await;
    let (_, g) = api(&srv, &ann, "POST", "/groups", json!({"name": "Study"})).await;
    let id = g["id"].as_str().unwrap().to_string();
    let (_, g) = api(&srv, &ann, "POST", &format!("/groups/{id}/invites"), json!({"role": "read"})).await;
    let read = g["invites"]["read"].clone();
    api(&srv, &bob, "POST", "/join", json!({"token": read})).await;
    let q = format!("?space={id}");
    let mut b = connect_to(&srv, Some(&bob), None, &q).await.unwrap();
    assert_eq!(b.hello(0).await.len(), 0);
    assert_eq!(b.recv().await["type"], "history");
    b.send(json!({"type": "history", "entries": [entry("h1", T1, 10)]})).await;
    assert_eq!(b.recv().await["entries"][0]["id"], "h1", "a view-only member keeps history too");
    api(&srv, &ann, "DELETE", &format!("/groups/{id}/members/{bob_owner}"), Value::Null).await;
    assert_eq!(b.recv().await["type"], "error");
    api(&srv, &bob, "POST", "/join", json!({"token": read})).await;
    let mut b = connect_to(&srv, Some(&bob), None, &q).await.unwrap();
    assert_eq!(b.hello(0).await.len(), 0);
    assert_eq!(b.recv().await, json!({"type": "history", "entries": []}), "removed with the membership");
}
