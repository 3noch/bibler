//! The groups API against an in-memory accounts database.

use api::groups::{handle, role_of, AccountsDb, Reply, Role};
use rusqlite::Connection;
use serde_json::json;
use space::{Db, Value};
use std::future::Future;
use std::task::{Context, Poll, Waker};

struct Mem(Connection);

#[allow(clippy::unused_async_trait_impl)] // a synchronous database
impl AccountsDb for Mem {
    async fn exec(&self, sql: &str, params: Vec<Value>) -> Result<(), String> {
        self.0.exec(sql, &params)
    }
    async fn query(&self, sql: &str, params: Vec<Value>) -> Result<Vec<Vec<Value>>, String> {
        self.0.query(sql, &params)
    }
}

/// The futures here never wait (the database is synchronous).
fn run<T>(f: impl Future<Output = T>) -> T {
    let mut f = std::pin::pin!(f);
    match f.as_mut().poll(&mut Context::from_waker(Waker::noop())) {
        Poll::Ready(v) => v,
        Poll::Pending => panic!("pending"),
    }
}

fn db() -> Mem {
    let c = Connection::open_in_memory().unwrap();
    c.execute_batch("PRAGMA foreign_keys = ON").unwrap();
    for s in api::accounts::schema_statements() {
        c.execute_batch(s).unwrap();
    }
    for (o, name) in [("ann", Some("Ann")), ("bob", None), ("cat", None)] {
        c.execute("INSERT INTO owners (id, created_at) VALUES (?1, 0)", [o]).unwrap();
        if let Some(n) = name {
            c.execute(
                "INSERT INTO identities (provider, subject, owner_id, email, name, created_at)
                 VALUES ('google', ?1, ?1, 'ann@example.com', ?2, 0)",
                [o, n],
            )
            .unwrap();
        }
    }
    Mem(c)
}

fn call(db: &Mem, owner: &str, method: &str, path: &str, body: &serde_json::Value) -> Reply {
    run(handle(db, owner, method, path, &body.to_string(), 1)).unwrap()
}

#[test]
fn create_invite_join_roles_and_leave() {
    let db = db();
    let none = json!(null);
    assert_eq!(call(&db, "ann", "POST", "/groups", &json!({"name": "  "})).body["error"], "bad_name");
    let g = call(&db, "ann", "POST", "/groups", &json!({"name": " Study "})).body;
    let id = g["id"].as_str().unwrap().to_string();
    assert_eq!((g["name"].as_str(), g["role"].as_str()), (Some("Study"), Some("admin")));
    assert_eq!(g["members"], json!([{"owner": "ann", "role": "admin", "name": "Ann", "email": "ann@example.com"}]));
    assert_eq!(run(role_of(&db, &id, "ann")).unwrap(), Some(Role::Admin));
    assert_eq!(run(role_of(&db, "bob", "bob")).unwrap(), Some(Role::Admin), "your personal space");
    assert_eq!(run(role_of(&db, &id, "bob")).unwrap(), None);
    assert_eq!(call(&db, "bob", "GET", &format!("/groups/{id}"), &none).status, 404, "not a member");

    // Invite links, one per role; a new one revokes the old.
    let path = format!("/groups/{id}/invites");
    assert_eq!(call(&db, "ann", "POST", &path, &json!({"role": "admin"})).body["error"], "bad_role");
    let old = call(&db, "ann", "POST", &path, &json!({"role": "read"})).body["invites"]["read"].clone();
    let g = call(&db, "ann", "POST", &path, &json!({"role": "read"})).body;
    let read = g["invites"]["read"].clone();
    assert_ne!(old, read);
    assert_eq!(call(&db, "bob", "POST", "/join", &json!({"token": old})).body["error"], "bad_invite");
    let joined = call(&db, "bob", "POST", "/join", &json!({"token": read})).body;
    assert_eq!((joined["role"].as_str(), joined.get("invites")), (Some("read"), None), "only admins see links");
    assert_eq!(run(role_of(&db, &id, "bob")).unwrap(), Some(Role::Read));
    assert!(!Role::Read.can_write() && Role::Edit.can_write());
    let list = call(&db, "bob", "GET", "/groups", &none).body;
    assert_eq!(list, json!({"groups": [{"id": id, "name": "Study", "role": "read"}]}));

    // Only admins manage.
    assert_eq!(call(&db, "bob", "PATCH", &format!("/groups/{id}"), &json!({"name": "Mine"})).status, 403);
    let bob = format!("/groups/{id}/members/bob");
    assert_eq!(call(&db, "bob", "PUT", &bob, &json!({"role": "edit"})).status, 403);
    let r = call(&db, "ann", "PUT", &bob, &json!({"role": "edit"}));
    assert_eq!(r.kicks, [(id.clone(), "bob".to_string())], "bob reconnects with the new role");
    assert_eq!(run(role_of(&db, &id, "bob")).unwrap(), Some(Role::Edit));
    assert_eq!(call(&db, "ann", "PATCH", &format!("/groups/{id}"), &json!({"name": "Romans"})).body["name"], "Romans");

    // The last admin can't step down or leave while others remain.
    let ann = format!("/groups/{id}/members/ann");
    assert_eq!(call(&db, "ann", "PUT", &ann, &json!({"role": "read"})).body["error"], "last_admin");
    assert_eq!(call(&db, "ann", "DELETE", &ann, &none).body["error"], "last_admin");
    // Anyone can leave; admins can remove.
    assert_eq!(call(&db, "bob", "DELETE", &bob, &none).kicks, [(id.clone(), "bob".to_string())]);
    assert_eq!(run(role_of(&db, &id, "bob")).unwrap(), None);
    call(&db, "cat", "POST", "/join", &json!({"token": read}));
    call(&db, "ann", "DELETE", &format!("/groups/{id}/members/cat"), &none);
    assert_eq!(run(role_of(&db, &id, "cat")).unwrap(), None);
    assert_eq!(call(&db, "ann", "DELETE", &ann, &none).status, 200, "alone: may leave");
    assert_eq!(call(&db, "cat", "POST", "/join", &json!({"token": read})).body["error"], "bad_invite");
}

#[test]
fn signing_in_moves_an_anonymous_owners_memberships_to_the_account() {
    let db = db();
    let create = |owner: &str, name: &str| {
        call(&db, owner, "POST", "/groups", &json!({"name": name})).body["id"].as_str().unwrap().to_string()
    };
    let invite = |g: &str, role: &str| {
        call(&db, "ann", "POST", &format!("/groups/{g}/invites"), &json!({"role": role})).body["invites"][role]
            .as_str()
            .unwrap()
            .to_string()
    };
    let (both, only_cat, cats) = (create("ann", "Both"), create("ann", "Cat only"), create("cat", "Cat's own"));
    // bob (the account) can view "Both"; cat (anonymous) can edit it, and edits "Cat only", and created "Cat's own".
    call(&db, "bob", "POST", "/join", &json!({"token": invite(&both, "read")}));
    call(&db, "cat", "POST", "/join", &json!({"token": invite(&both, "edit")}));
    call(&db, "cat", "POST", "/join", &json!({"token": invite(&only_cat, "edit")}));

    let mut moved = run(api::groups::move_memberships(&db, "cat", "bob", 2)).unwrap();
    moved.sort();
    let mut want = vec![both.clone(), only_cat.clone(), cats.clone()];
    want.sort();
    assert_eq!(moved, want);
    let role = |g: &str, o: &str| run(role_of(&db, g, o)).unwrap();
    assert_eq!(role(&both, "bob"), Some(Role::Edit), "the higher role wins");
    assert_eq!(role(&only_cat, "bob"), Some(Role::Edit));
    assert_eq!(role(&cats, "bob"), Some(Role::Admin), "the group cat made is bob's to run");
    assert_eq!([role(&both, "cat"), role(&only_cat, "cat"), role(&cats, "cat")], [None, None, None]);
    assert_eq!(run(api::groups::spaces_of(&db, "cat")).unwrap(), Vec::<String>::new());
}
