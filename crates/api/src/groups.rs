//! Groups: shared spaces several people tag and comment in together.
//!
//! A group is a space whose rows all belong to the group (the space id is the data owner, as a personal space's is
//! its owner's id), so members share tags, the palette and comments; `space::apply_by` records who wrote each
//! comment and lets only its author change it. Memberships live in the accounts database with a role:
//! `admin` (manage the group, edit), `edit` (tag and comment), `read` (view and receive live updates only).
//! People join with an invite link (`/?join=<token>`), one reusable link per role; making a new one revokes the old.
//!
//! The HTTP API ([`handle`]) is shared by both servers over the [`AccountsDb`] trait:
//! ```text
//! GET    /api/groups                         -> {groups: [{id, name, role}]}
//! POST   /api/groups {name}                  -> group          (you are its admin)
//! GET    /api/groups/:id                     -> group          {id, name, role, members: [{owner, role, name,
//!                                                               email}], invites: {edit, read} (admins only)}
//! PATCH  /api/groups/:id {name}              -> group          (admin)
//! POST   /api/groups/:id/invites {role}      -> group          (admin; a new link for that role)
//! PUT    /api/groups/:id/members/:owner {role} -> group        (admin)
//! DELETE /api/groups/:id/members/:owner      -> {}             (admin, or yourself = leave)
//! POST   /api/join {token}                   -> group          (join with an invite link)
//! ```
//! Errors: `{error: code}` with 400 (`bad_name`, `bad_role`, `last_admin`), 403 (`forbidden`), 404 (`not_found`,
//! `bad_invite`). A member whose role changed or who was removed is disconnected ([`Reply::kicks`]) so their next
//! connection gets the new role (or is refused).

use crate::session::random_hex;
use serde_json::{json, Value as Json};
use space::Value;

pub const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS groups (
  id TEXT PRIMARY KEY,
  name TEXT NOT NULL,
  created_by TEXT NOT NULL,
  created_at INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS memberships (
  space_id TEXT NOT NULL,
  owner_id TEXT NOT NULL REFERENCES owners(id),
  role TEXT NOT NULL,
  created_at INTEGER NOT NULL,
  PRIMARY KEY (space_id, owner_id)
);
CREATE INDEX IF NOT EXISTS memberships_by_owner ON memberships (owner_id);
CREATE TABLE IF NOT EXISTS invites (
  token TEXT PRIMARY KEY,
  space_id TEXT NOT NULL,
  role TEXT NOT NULL,
  created_by TEXT NOT NULL,
  created_at INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS invites_by_space ON invites (space_id, role);
";

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Role {
    Read,
    Edit,
    Admin,
}

impl Role {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Role::Read => "read",
            Role::Edit => "edit",
            Role::Admin => "admin",
        }
    }

    #[must_use]
    pub fn parse(s: &str) -> Option<Role> {
        match s {
            "read" => Some(Role::Read),
            "edit" => Some(Role::Edit),
            "admin" => Some(Role::Admin),
            _ => None,
        }
    }

    /// May send ops.
    #[must_use]
    pub fn can_write(self) -> bool {
        self >= Role::Edit
    }
}

/// The accounts database (rusqlite on the self-hosted server, D1 on Cloudflare). Positional `?` parameters.
#[allow(async_fn_in_trait)] // implemented per server; the server checks `Send` on its concrete type
pub trait AccountsDb {
    /// # Errors
    /// On SQL failure.
    async fn exec(&self, sql: &str, params: Vec<Value>) -> Result<(), String>;
    /// # Errors
    /// On SQL failure.
    async fn query(&self, sql: &str, params: Vec<Value>) -> Result<Vec<Vec<Value>>, String>;
}

/// `owner`'s role in `space`: their personal space (space id = owner id) is theirs; a group needs a membership.
///
/// # Errors
/// On storage failure.
pub async fn role_of(db: &impl AccountsDb, space: &str, owner: &str) -> Result<Option<Role>, String> {
    if space == owner {
        return Ok(Some(Role::Admin));
    }
    let rows = db
        .query("SELECT role FROM memberships WHERE space_id = ? AND owner_id = ?", vec![space.into(), owner.into()])
        .await?;
    Ok(rows.first().and_then(|r| Role::parse(text(&r[0]))))
}

/// The groups `owner` belongs to.
///
/// # Errors
/// On storage failure.
pub async fn spaces_of(db: &impl AccountsDb, owner: &str) -> Result<Vec<String>, String> {
    let rows =
        db.query("SELECT space_id FROM memberships WHERE owner_id = ? ORDER BY space_id", vec![owner.into()]).await?;
    Ok(rows.iter().map(|r| text(&r[0]).to_string()).collect())
}

/// Signing in merged anonymous owner `from` into account `to`: give `to` `from`'s memberships (keeping the higher
/// role where both are members) and the groups `from` created. Call it after reassigning `from`'s comments in those
/// groups (`space::reassign_author`), so a failure in between can be retried. Returns the groups.
///
/// # Errors
/// On storage failure.
pub async fn move_memberships(db: &impl AccountsDb, from: &str, to: &str, now: i64) -> Result<Vec<String>, String> {
    let rows = db
        .query("SELECT space_id, role FROM memberships WHERE owner_id = ? ORDER BY space_id", vec![from.into()])
        .await?;
    let mut spaces = vec![];
    for r in &rows {
        let (space, role) = (text(&r[0]).to_string(), Role::parse(text(&r[1])).unwrap_or(Role::Read));
        match role_of(db, &space, to).await? {
            None => {
                let sql = "INSERT INTO memberships (space_id, owner_id, role, created_at) VALUES (?, ?, ?, ?)";
                db.exec(sql, vec![space.as_str().into(), to.into(), role.as_str().into(), now.into()]).await?;
            }
            Some(theirs) if role > theirs => {
                let sql = "UPDATE memberships SET role = ? WHERE space_id = ? AND owner_id = ?";
                db.exec(sql, vec![role.as_str().into(), space.as_str().into(), to.into()]).await?;
            }
            Some(_) => {}
        }
        spaces.push(space);
    }
    db.exec("DELETE FROM memberships WHERE owner_id = ?", vec![from.into()]).await?;
    db.exec("UPDATE groups SET created_by = ? WHERE created_by = ?", vec![to.into(), from.into()]).await?;
    Ok(spaces)
}

/// What to answer, and whom to disconnect.
#[derive(Debug)]
pub struct Reply {
    pub status: u16,
    pub body: Json,
    /// `(space, owner)`: close this owner's sockets in this space (their role changed or they left).
    pub kicks: Vec<(String, String)>,
}

fn ok(body: Json) -> Reply {
    Reply { status: 200, body, kicks: vec![] }
}

fn err(status: u16, code: &str) -> Reply {
    Reply { status, body: json!({ "error": code }), kicks: vec![] }
}

fn text(v: &Value) -> &str {
    match v {
        Value::Text(s) => s,
        _ => "",
    }
}

fn opt_text(v: &Value) -> Json {
    match v {
        Value::Text(s) => json!(s),
        _ => Json::Null,
    }
}

fn group_name(body: &Json) -> Option<String> {
    let name = body["name"].as_str()?.trim();
    (!name.is_empty() && name.chars().count() <= 100).then(|| name.to_string())
}

/// Serve one groups request for the session's `owner`. `path` is below `/api` (`/groups/...`, `/join`).
///
/// # Errors
/// On storage failure (answer 500).
pub async fn handle(
    db: &impl AccountsDb,
    owner: &str,
    method: &str,
    path: &str,
    body: &str,
    now: i64,
) -> Result<Reply, String> {
    let body: Json = serde_json::from_str(body).unwrap_or(Json::Null);
    let parts: Vec<&str> = path.trim_matches('/').split('/').collect();
    match (method, parts.as_slice()) {
        ("GET", ["groups"]) => list(db, owner).await,
        ("POST", ["groups"]) => {
            let Some(name) = group_name(&body) else { return Ok(err(400, "bad_name")) };
            let id = random_hex(16)?;
            db.exec(
                "INSERT INTO groups (id, name, created_by, created_at) VALUES (?, ?, ?, ?)",
                vec![id.as_str().into(), name.into(), owner.into(), now.into()],
            )
            .await?;
            add_member(db, &id, owner, Role::Admin, now).await?;
            detail(db, &id, owner).await
        }
        ("POST", ["join"]) => {
            let token = body["token"].as_str().unwrap_or_default();
            let rows = db.query("SELECT space_id, role FROM invites WHERE token = ?", vec![token.into()]).await?;
            let Some(r) = rows.first() else { return Ok(err(404, "bad_invite")) };
            let (space, role) = (text(&r[0]).to_string(), Role::parse(text(&r[1])).unwrap_or(Role::Read));
            if role_of(db, &space, owner).await?.is_none() {
                add_member(db, &space, owner, role, now).await?;
            }
            detail(db, &space, owner).await
        }
        (_, ["groups", id, rest @ ..]) => {
            let Some(mine) = membership(db, id, owner).await? else { return Ok(err(404, "not_found")) };
            match (method, rest) {
                ("GET", []) => detail(db, id, owner).await,
                ("DELETE", ["members", who]) if *who == owner => remove(db, id, who).await,
                _ if mine != Role::Admin => Ok(err(403, "forbidden")),
                ("PATCH", []) => {
                    let Some(name) = group_name(&body) else { return Ok(err(400, "bad_name")) };
                    db.exec("UPDATE groups SET name = ? WHERE id = ?", vec![name.into(), (*id).into()]).await?;
                    detail(db, id, owner).await
                }
                ("POST", ["invites"]) => {
                    let Some(role) = body["role"].as_str().and_then(Role::parse).filter(|r| *r != Role::Admin) else {
                        return Ok(err(400, "bad_role"));
                    };
                    db.exec(
                        "DELETE FROM invites WHERE space_id = ? AND role = ?",
                        vec![(*id).into(), role.as_str().into()],
                    )
                    .await?;
                    db.exec(
                        "INSERT INTO invites (token, space_id, role, created_by, created_at) VALUES (?, ?, ?, ?, ?)",
                        vec![random_hex(16)?.into(), (*id).into(), role.as_str().into(), owner.into(), now.into()],
                    )
                    .await?;
                    detail(db, id, owner).await
                }
                ("PUT", ["members", who]) => {
                    let Some(role) = body["role"].as_str().and_then(Role::parse) else {
                        return Ok(err(400, "bad_role"));
                    };
                    let Some(was) = membership(db, id, who).await? else { return Ok(err(404, "not_found")) };
                    if was == Role::Admin && role != Role::Admin && admins(db, id).await? <= 1 {
                        return Ok(err(400, "last_admin"));
                    }
                    db.exec(
                        "UPDATE memberships SET role = ? WHERE space_id = ? AND owner_id = ?",
                        vec![role.as_str().into(), (*id).into(), (*who).into()],
                    )
                    .await?;
                    let mut reply = detail(db, id, owner).await?;
                    if was != role {
                        reply.kicks.push(((*id).to_string(), (*who).to_string()));
                    }
                    Ok(reply)
                }
                ("DELETE", ["members", who]) => remove(db, id, who).await,
                _ => Ok(err(404, "not_found")),
            }
        }
        _ => Ok(err(404, "not_found")),
    }
}

async fn membership(db: &impl AccountsDb, space: &str, owner: &str) -> Result<Option<Role>, String> {
    let rows = db
        .query("SELECT role FROM memberships WHERE space_id = ? AND owner_id = ?", vec![space.into(), owner.into()])
        .await?;
    Ok(rows.first().and_then(|r| Role::parse(text(&r[0]))))
}

async fn add_member(db: &impl AccountsDb, space: &str, owner: &str, role: Role, now: i64) -> Result<(), String> {
    db.exec(
        "INSERT OR IGNORE INTO memberships (space_id, owner_id, role, created_at) VALUES (?, ?, ?, ?)",
        vec![space.into(), owner.into(), role.as_str().into(), now.into()],
    )
    .await
}

async fn admins(db: &impl AccountsDb, space: &str) -> Result<usize, String> {
    let rows =
        db.query("SELECT owner_id FROM memberships WHERE space_id = ? AND role = 'admin'", vec![space.into()]).await?;
    Ok(rows.len())
}

/// Remove `who` (the last admin can only leave a group nobody else is in).
async fn remove(db: &impl AccountsDb, space: &str, who: &str) -> Result<Reply, String> {
    let Some(role) = membership(db, space, who).await? else { return Ok(err(404, "not_found")) };
    let members = db.query("SELECT owner_id FROM memberships WHERE space_id = ?", vec![space.into()]).await?;
    if role == Role::Admin && admins(db, space).await? <= 1 && members.len() > 1 {
        return Ok(err(400, "last_admin"));
    }
    db.exec("DELETE FROM memberships WHERE space_id = ? AND owner_id = ?", vec![space.into(), who.into()]).await?;
    if members.len() <= 1 {
        db.exec("DELETE FROM invites WHERE space_id = ?", vec![space.into()]).await?;
    }
    Ok(Reply { status: 200, body: json!({}), kicks: vec![(space.to_string(), who.to_string())] })
}

async fn list(db: &impl AccountsDb, owner: &str) -> Result<Reply, String> {
    let rows = db
        .query(
            "SELECT g.id, g.name, m.role FROM memberships m JOIN groups g ON g.id = m.space_id
             WHERE m.owner_id = ? ORDER BY g.name, g.id",
            vec![owner.into()],
        )
        .await?;
    let groups: Vec<Json> =
        rows.iter().map(|r| json!({ "id": text(&r[0]), "name": text(&r[1]), "role": text(&r[2]) })).collect();
    Ok(ok(json!({ "groups": groups })))
}

async fn detail(db: &impl AccountsDb, space: &str, owner: &str) -> Result<Reply, String> {
    let g = db.query("SELECT name FROM groups WHERE id = ?", vec![space.into()]).await?;
    let (Some(g), Some(role)) = (g.first(), membership(db, space, owner).await?) else {
        return Ok(err(404, "not_found"));
    };
    let members = db
        .query(
            "SELECT m.owner_id, m.role,
               (SELECT name FROM identities i WHERE i.owner_id = m.owner_id ORDER BY created_at LIMIT 1),
               (SELECT email FROM identities i WHERE i.owner_id = m.owner_id ORDER BY created_at LIMIT 1)
             FROM memberships m WHERE m.space_id = ? ORDER BY m.created_at, m.owner_id",
            vec![space.into()],
        )
        .await?;
    let members: Vec<Json> = members
        .iter()
        .map(
            |r| json!({ "owner": text(&r[0]), "role": text(&r[1]), "name": opt_text(&r[2]), "email": opt_text(&r[3]) }),
        )
        .collect();
    let mut body = json!({ "id": space, "name": text(&g[0]), "role": role.as_str(), "members": members });
    if role == Role::Admin {
        let invites = db.query("SELECT role, token FROM invites WHERE space_id = ?", vec![space.into()]).await?;
        let invites: serde_json::Map<String, Json> =
            invites.iter().map(|r| (text(&r[0]).to_string(), json!(text(&r[1])))).collect();
        body["invites"] = Json::Object(invites);
    }
    Ok(ok(body))
}
