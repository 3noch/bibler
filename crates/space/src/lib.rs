//! One **space** = one SQLite database = one sync stream: the unit a Durable Object (or the self-hosted server's
//! `spaces/<id>.sqlite`) holds. A personal space has one member; the schema is multi-owner so a shared space
//! (a group) can later use the same tables: every row keeps its `owner_id`, ops only see their own owner's rows
//! (`ops::RowSource` is owner-scoped), ids are unique within the space, and `changes` has ONE `seq` for the whole
//! space (every member's client needs the same total order to rebase).
//!
//! The SQL runs over the small [`Db`] trait so the same statements serve rusqlite and Durable Object storage.
//! Callers wrap [`apply`] in a transaction (on Durable Objects, one synchronous call is atomic). `apply` does all its
//! checks before its first write, so a failed op writes nothing even without a rollback.
//!
//! **Retention**: [`prune`] blanks the payload of changes older than the retention period, oldest first, a bounded
//! batch per call (servers call it after each applied op, so pruning keeps up without long-running work). A pruned
//! change keeps its row (seq, author, op id), so a re-sent op is still recognized as a duplicate forever; it just
//! can no longer be replayed or reverted. `meta.pruned_through` is the last pruned seq: a client whose last seq is
//! older gets a [`reset`] (every row) instead of a replay.

use ops::{effect, Change, CommentRow, Intent, Op, OpError, PaletteRow, Row, RowSource, TagRow, TaggingRow};
use std::cell::RefCell;

/// A SQLite value (the subset this schema uses).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Value {
    Null,
    Int(i64),
    Text(String),
}

impl From<&str> for Value {
    fn from(s: &str) -> Self {
        Value::Text(s.to_string())
    }
}
impl From<String> for Value {
    fn from(s: String) -> Self {
        Value::Text(s)
    }
}
impl From<&String> for Value {
    fn from(s: &String) -> Self {
        Value::Text(s.clone())
    }
}
impl From<i64> for Value {
    fn from(n: i64) -> Self {
        Value::Int(n)
    }
}
impl From<u32> for Value {
    fn from(n: u32) -> Self {
        Value::Int(i64::from(n))
    }
}
impl From<u64> for Value {
    fn from(n: u64) -> Self {
        Value::Int(i64::try_from(n).unwrap_or(i64::MAX))
    }
}
impl<T: Into<Value>> From<Option<T>> for Value {
    fn from(v: Option<T>) -> Self {
        v.map_or(Value::Null, Into::into)
    }
}

/// Synchronous SQL with positional `?` parameters.
pub trait Db {
    /// # Errors
    /// On SQL failure.
    fn exec(&self, sql: &str, params: &[Value]) -> Result<(), String>;
    /// # Errors
    /// On SQL failure.
    fn query(&self, sql: &str, params: &[Value]) -> Result<Vec<Vec<Value>>, String>;
}

/// Run once per database open (idempotent). No PRAGMAs: Durable Objects reject most of them; the self-hosted
/// server sets its own.
pub const SCHEMA: &[&str] = &[
    "CREATE TABLE IF NOT EXISTS meta (key TEXT PRIMARY KEY, value TEXT NOT NULL)",
    "INSERT OR IGNORE INTO meta (key, value) VALUES ('schema', '1')",
    "CREATE TABLE IF NOT EXISTS members (owner_id TEXT PRIMARY KEY, created_at INTEGER NOT NULL)",
    "CREATE TABLE IF NOT EXISTS tags (
      id TEXT PRIMARY KEY,
      owner_id TEXT NOT NULL REFERENCES members(owner_id),
      name TEXT NOT NULL,
      color TEXT NOT NULL,
      description TEXT,
      created_at INTEGER NOT NULL,
      updated_at INTEGER NOT NULL,
      deleted_at INTEGER,
      UNIQUE (owner_id, id)
    )",
    // Backstop only (ASCII lower()); the real rule (Unicode case-insensitive) is enforced by ops::effect.
    "CREATE UNIQUE INDEX IF NOT EXISTS tags_live_name ON tags (owner_id, lower(name)) WHERE deleted_at IS NULL",
    "CREATE TABLE IF NOT EXISTS taggings (
      id TEXT PRIMARY KEY,
      owner_id TEXT NOT NULL,
      tag_id TEXT NOT NULL,
      text_id TEXT NOT NULL,
      start_ord INTEGER NOT NULL,
      end_ord INTEGER NOT NULL,
      created_at INTEGER NOT NULL,
      deleted_at INTEGER,
      merged_into TEXT,
      FOREIGN KEY (owner_id, tag_id) REFERENCES tags (owner_id, id),
      CHECK (start_ord < end_ord)
    )",
    "CREATE INDEX IF NOT EXISTS taggings_live ON taggings (owner_id, tag_id, text_id, start_ord)
      WHERE deleted_at IS NULL",
    "CREATE TABLE IF NOT EXISTS palettes (
      owner_id TEXT PRIMARY KEY REFERENCES members(owner_id),
      tags TEXT NOT NULL,
      updated_at INTEGER NOT NULL
    )",
    // Ownership of `tagging_id` is checked by ops::effect (owner-scoped reads); the FK is a backstop.
    "CREATE TABLE IF NOT EXISTS comments (
      id TEXT PRIMARY KEY,
      owner_id TEXT NOT NULL REFERENCES members(owner_id),
      tagging_id TEXT NOT NULL REFERENCES taggings(id),
      body TEXT NOT NULL,
      created_at INTEGER NOT NULL,
      edited_at INTEGER,
      deleted_at INTEGER
    )",
    "CREATE INDEX IF NOT EXISTS comments_by_tagging ON comments (owner_id, tagging_id)",
    // `owner_id` = the op's author. `seq` is per space.
    "CREATE TABLE IF NOT EXISTS changes (
      seq INTEGER PRIMARY KEY,
      owner_id TEXT NOT NULL,
      op_id TEXT NOT NULL,
      kind TEXT NOT NULL,
      payload TEXT NOT NULL,
      created_at INTEGER NOT NULL,
      UNIQUE (owner_id, op_id)
    )",
];

/// Create the tables if needed.
///
/// # Errors
/// On SQL failure.
pub fn init(db: &impl Db) -> Result<(), String> {
    SCHEMA.iter().try_for_each(|s| db.exec(s, &[]))?;
    let version = db.query("SELECT value FROM meta WHERE key = 'schema'", &[])?;
    if version.first().map(|r| &r[0]) == Some(&Value::Text("1".into())) {
        // v2: comment authors (shared spaces show who wrote what).
        db.exec("ALTER TABLE comments ADD COLUMN author_id TEXT", &[])?;
        db.exec("UPDATE meta SET value = '2' WHERE key = 'schema'", &[])?;
    }
    Ok(())
}

#[derive(Debug)]
pub enum ApplyError {
    /// The op is invalid against the owner's rows (a stable code the client understands).
    Op(OpError),
    /// Not allowed for this author (e.g. `not_author`: editing someone else's comment); nothing was written.
    Refused(&'static str),
    /// Storage failure; the op had no effect.
    Internal(String),
}

impl From<String> for ApplyError {
    fn from(e: String) -> Self {
        ApplyError::Internal(e)
    }
}

#[derive(Debug, Clone)]
pub struct Applied {
    pub change: Change,
    /// The op id was already applied by this owner; `change` is the original result.
    pub duplicate: bool,
}

/// Apply `owner`'s op exactly once (by `(owner, op_id)`), assigning the space's next seq.
///
/// # Errors
/// `ApplyError::Op` if the op is invalid (nothing was written); `ApplyError::Internal` on storage failure.
pub fn apply(db: &impl Db, owner: &str, op: &Op, now: i64) -> Result<Applied, ApplyError> {
    apply_by(db, owner, owner, op, now)
}

/// Apply an op to `owner`'s rows on behalf of `author`, a member of a shared space whose rows all belong to one
/// data owner (the space). New comments are stamped with `author`, and a change may only modify comments that
/// `author` wrote.
///
/// # Errors
/// As [`apply`], plus `ApplyError::Refused("not_author")`.
pub fn apply_by(db: &impl Db, owner: &str, author: &str, op: &Op, now: i64) -> Result<Applied, ApplyError> {
    let existing = db.query(
        "SELECT payload, seq, kind FROM changes WHERE owner_id = ? AND op_id = ?",
        &[owner.into(), (&op.op_id).into()],
    )?;
    if let Some(row) = existing.first() {
        let change = if text(&row[0])? == PRUNED {
            // Applied long ago: only its seq is left (enough to confirm it).
            let seq = u64::try_from(int(&row[1])?).map_err(|e| e.to_string())?;
            Change { seq, op_id: op.op_id.clone(), kind: text(&row[2])?.to_string(), rows: vec![] }
        } else {
            parse_change(text(&row[0])?)?
        };
        return Ok(Applied { change, duplicate: true });
    }
    let mut change = {
        let src = Rows { db, owner, err: RefCell::new(None) };
        let result = effect(&src, op);
        if let Some(e) = src.err.into_inner() {
            return Err(e.into()); // a failed read: the computed effect cannot be trusted
        }
        result.map_err(ApplyError::Op)?
    };
    for rc in &mut change.rows {
        if let Row::Comment(after) = &mut rc.after {
            match &rc.before {
                None => after.author = Some(author.to_string()),
                Some(Row::Comment(b)) if b.author.as_deref().is_some_and(|a| a != author) => {
                    return Err(ApplyError::Refused("not_author"));
                }
                _ => {}
            }
        }
    }
    for rc in &change.rows {
        if let Some(other) = owner_of(db, &rc.after)? {
            if other != owner {
                return Err(ApplyError::Op(OpError::IdExists)); // the id belongs to another member
            }
        }
    }
    let seq = db.query("SELECT COALESCE(MAX(seq), 0) + 1 FROM changes", &[])?;
    change.seq = u64::try_from(int(&seq[0][0])?).map_err(|e| e.to_string())?;
    join(db, owner, now)?;
    for rc in &change.rows {
        write_row(db, owner, &rc.after)?;
    }
    insert_change(db, owner, &change, now)?;
    Ok(Applied { change, duplicate: false })
}

/// Changes with `seq > after`, oldest first, at most `limit`. Check [`pruned_through`] first: replaying from before
/// it is an error (send a [`reset`] instead).
///
/// # Errors
/// On storage failure, or if a requested change was pruned.
pub fn changes_since(db: &impl Db, after: u64, limit: usize) -> Result<Vec<Change>, String> {
    let rows = db.query(
        "SELECT payload FROM changes WHERE seq > ? ORDER BY seq LIMIT ?",
        &[after.into(), (limit as u64).into()],
    )?;
    rows.iter()
        .map(|r| match text(&r[0])? {
            PRUNED => Err(format!("changes after {after} were pruned")),
            payload => parse_change(payload),
        })
        .collect()
}

/// A pruned change's payload.
const PRUNED: &str = "";

/// How long changes stay replayable (and revertible): 30 days.
pub const RETENTION_MS: i64 = 30 * 24 * 60 * 60 * 1000;
/// Changes pruned per [`prune`] call at most (each call does a little, so no request runs long).
pub const PRUNE_BATCH: usize = 64;

/// The last pruned seq (0: nothing pruned). Changes up to it can't be replayed.
///
/// # Errors
/// On storage failure.
pub fn pruned_through(db: &impl Db) -> Result<u64, String> {
    let r = db.query("SELECT value FROM meta WHERE key = 'pruned_through'", &[])?;
    r.first().map_or(Ok(0), |r| text(&r[0])?.parse().map_err(|e| format!("pruned_through: {e}")))
}

/// Blank the payload of up to `limit` of the oldest changes created before `before` (ms), continuing where the last
/// call stopped. Pruning stops at the first change that is too new, so everything up to [`pruned_through`] is pruned
/// and everything after it is intact. Returns how many were pruned.
///
/// # Errors
/// On storage failure.
pub fn prune(db: &impl Db, before: i64, limit: usize) -> Result<usize, String> {
    let from = pruned_through(db)?;
    let rows = db.query(
        "SELECT seq, created_at FROM changes WHERE seq > ? ORDER BY seq LIMIT ?",
        &[from.into(), (limit as u64).into()],
    )?;
    let mut last = None;
    for r in &rows {
        if int(&r[1])? >= before {
            break;
        }
        last = Some(int(&r[0])?);
    }
    let Some(last) = last else { return Ok(0) };
    // Record first: if blanking then fails (Durable Objects have no rollback), clients near the boundary just get a
    // reset they didn't need, instead of a replay that can never succeed.
    db.exec(
        "INSERT INTO meta (key, value) VALUES ('pruned_through', ?) ON CONFLICT (key) DO UPDATE SET value = excluded.value",
        &[last.to_string().into()],
    )?;
    db.exec("UPDATE changes SET payload = ? WHERE seq > ? AND seq <= ?", &[PRUNED.into(), from.into(), last.into()])?;
    Ok(rows.iter().take_while(|r| int(&r[0]).is_ok_and(|s| s <= last)).count())
}

/// What a client that is too far behind to replay gets instead: the space's current state.
#[derive(Debug, Clone, PartialEq)]
pub struct Reset {
    /// The space's last seq.
    pub seq: u64,
    /// Every row of the data owner, deleted ones included (comments on merged taggings follow tombstones).
    pub rows: Vec<Row>,
    /// Which of the op ids the client still holds (its `hello.pending`) are already applied, so it can drop them
    /// silently instead of replaying them onto `rows`.
    pub applied: Vec<String>,
}

/// The data owner's whole state, for a client holding unconfirmed ops `pending`. Read it in one transaction.
///
/// # Errors
/// On storage failure.
pub fn reset(db: &impl Db, owner: &str, pending: &[String]) -> Result<Reset, String> {
    let seq = db.query("SELECT COALESCE(MAX(seq), 0) FROM changes", &[])?;
    let seq = u64::try_from(int(&seq[0][0])?).map_err(|e| e.to_string())?;
    let src = Rows { db, owner, err: RefCell::new(None) };
    let mut rows: Vec<Row> = vec![];
    let tags = format!("SELECT {TAG_COLS} FROM tags WHERE owner_id = ? ORDER BY id");
    rows.extend(src.list(&tags, &[owner.into()], tag_row).into_iter().map(Row::Tag));
    let taggings = format!("SELECT {TAGGING_COLS} FROM taggings WHERE owner_id = ? ORDER BY id");
    rows.extend(src.list(&taggings, &[owner.into()], tagging_row).into_iter().map(Row::Tagging));
    let comments = format!("SELECT {COMMENT_COLS} FROM comments WHERE owner_id = ? ORDER BY id");
    rows.extend(src.list(&comments, &[owner.into()], comment_row).into_iter().map(Row::Comment));
    rows.extend(src.palette().map(Row::Palette));
    if let Some(e) = src.err.into_inner() {
        return Err(e);
    }
    let mut applied = vec![];
    for id in pending {
        if !db.query("SELECT 1 FROM changes WHERE owner_id = ? AND op_id = ?", &[owner.into(), id.into()])?.is_empty() {
            applied.push(id.clone());
        }
    }
    Ok(Reset { seq, rows, applied })
}

/// Load `owner`'s rows and recorded changes as-is (no `effect`, seqs kept): fixtures, migrations, imports.
///
/// # Errors
/// On storage failure, or if an id belongs to another member.
pub fn import(db: &impl Db, owner: &str, rows: &[Row], changes: &[Change]) -> Result<(), String> {
    join(db, owner, 0)?;
    let mut rows = rows.to_vec();
    rows.sort_by(|a, b| a.key().cmp(&b.key())); // tags before the taggings that reference them
    for r in &rows {
        if owner_of(db, r)?.is_some_and(|o| o != owner) {
            return Err(format!("{:?} belongs to another member", r.key()));
        }
        write_row(db, owner, r)?;
    }
    changes.iter().try_for_each(|c| insert_change(db, owner, c, 0))
}

/// The owners whose rows are in this space.
///
/// # Errors
/// On storage failure.
pub fn members(db: &impl Db) -> Result<Vec<String>, String> {
    db.query("SELECT owner_id FROM members ORDER BY owner_id", &[])?
        .iter()
        .map(|r| text(&r[0]).map(str::to_string))
        .collect()
}

fn join(db: &impl Db, owner: &str, now: i64) -> Result<(), String> {
    db.exec("INSERT OR IGNORE INTO members (owner_id, created_at) VALUES (?, ?)", &[owner.into(), now.into()])
}

fn insert_change(db: &impl Db, owner: &str, c: &Change, now: i64) -> Result<(), String> {
    let payload = serde_json::to_string(c).map_err(|e| e.to_string())?;
    db.exec(
        "INSERT INTO changes (seq, owner_id, op_id, kind, payload, created_at) VALUES (?, ?, ?, ?, ?, ?)",
        &[c.seq.into(), owner.into(), (&c.op_id).into(), (&c.kind).into(), payload.into(), now.into()],
    )
}

/// The member owning the row's id, if it exists.
fn owner_of(db: &impl Db, row: &Row) -> Result<Option<String>, String> {
    let (table, id) = match row {
        Row::Tag(t) => ("tags", &t.id),
        Row::Tagging(t) => ("taggings", &t.id),
        Row::Comment(c) => ("comments", &c.id),
        Row::Palette(_) => return Ok(None), // keyed by owner
    };
    let r = db.query(&format!("SELECT owner_id FROM {table} WHERE id = ?"), &[id.into()])?;
    r.first().map(|r| text(&r[0]).map(str::to_string)).transpose()
}

/// Upsert a row of `owner` (whose id is known not to belong to another member).
fn write_row(db: &impl Db, owner: &str, row: &Row) -> Result<(), String> {
    match row {
        Row::Tag(t) => db.exec(
            "INSERT INTO tags (id, owner_id, name, color, created_at, updated_at, deleted_at)
             VALUES (?, ?, ?, ?, ?, ?, ?)
             ON CONFLICT (id) DO UPDATE SET name = excluded.name, color = excluded.color,
               updated_at = excluded.updated_at, deleted_at = excluded.deleted_at",
            &[
                (&t.id).into(),
                owner.into(),
                (&t.name).into(),
                (&t.color).into(),
                t.created_at.into(),
                t.updated_at.into(),
                t.deleted_at.into(),
            ],
        ),
        Row::Tagging(t) => db.exec(
            "INSERT INTO taggings (id, owner_id, tag_id, text_id, start_ord, end_ord, created_at, deleted_at, merged_into)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)
             ON CONFLICT (id) DO UPDATE SET start_ord = excluded.start_ord, end_ord = excluded.end_ord,
               deleted_at = excluded.deleted_at, merged_into = excluded.merged_into",
            &[
                (&t.id).into(),
                owner.into(),
                (&t.tag).into(),
                (&t.text_id).into(),
                t.start.into(),
                t.end.into(),
                t.created_at.into(),
                t.deleted_at.into(),
                t.merged_into.as_ref().into(),
            ],
        ),
        Row::Comment(c) => db.exec(
            "INSERT INTO comments (id, owner_id, tagging_id, body, created_at, edited_at, deleted_at, author_id)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?)
             ON CONFLICT (id) DO UPDATE SET body = excluded.body, edited_at = excluded.edited_at,
               deleted_at = excluded.deleted_at",
            &[
                (&c.id).into(),
                owner.into(),
                (&c.tagging).into(),
                (&c.body).into(),
                c.created_at.into(),
                c.edited_at.into(),
                c.deleted_at.into(),
                c.author.as_ref().into(),
            ],
        ),
        Row::Palette(p) => db.exec(
            "INSERT INTO palettes (owner_id, tags, updated_at) VALUES (?, ?, ?)
             ON CONFLICT (owner_id) DO UPDATE SET tags = excluded.tags, updated_at = excluded.updated_at",
            &[owner.into(), serde_json::to_string(&p.tags).map_err(|e| e.to_string())?.into(), p.updated_at.into()],
        ),
    }
}

fn parse_change(payload: &str) -> Result<Change, String> {
    serde_json::from_str(payload).map_err(|e| format!("stored change: {e}"))
}

fn text(v: &Value) -> Result<&str, String> {
    match v {
        Value::Text(s) => Ok(s),
        v => Err(format!("expected text, got {v:?}")),
    }
}

fn opt_text(v: &Value) -> Result<Option<String>, String> {
    match v {
        Value::Null => Ok(None),
        v => text(v).map(|s| Some(s.to_string())),
    }
}

fn int(v: &Value) -> Result<i64, String> {
    match v {
        Value::Int(n) => Ok(*n),
        v => Err(format!("expected an integer, got {v:?}")),
    }
}

fn opt_int(v: &Value) -> Result<Option<i64>, String> {
    match v {
        Value::Null => Ok(None),
        v => int(v).map(Some),
    }
}

fn ord(v: &Value) -> Result<u32, String> {
    u32::try_from(int(v)?).map_err(|e| e.to_string())
}

/// `RowSource` over a [`Db`], scoped to one owner. The trait is infallible, so the first error is recorded and
/// the caller discards the effect.
struct Rows<'a, D: Db> {
    db: &'a D,
    owner: &'a str,
    err: RefCell<Option<String>>,
}

const TAG_COLS: &str = "id, name, color, created_at, updated_at, deleted_at";
const TAGGING_COLS: &str = "id, tag_id, text_id, start_ord, end_ord, created_at, deleted_at, merged_into";
const COMMENT_COLS: &str = "id, tagging_id, body, created_at, edited_at, deleted_at, author_id";

fn tag_row(r: &[Value]) -> Result<TagRow, String> {
    Ok(TagRow {
        id: text(&r[0])?.to_string(),
        name: text(&r[1])?.to_string(),
        color: text(&r[2])?.to_string(),
        created_at: int(&r[3])?,
        updated_at: int(&r[4])?,
        deleted_at: opt_int(&r[5])?,
    })
}

fn tagging_row(r: &[Value]) -> Result<TaggingRow, String> {
    Ok(TaggingRow {
        id: text(&r[0])?.to_string(),
        tag: text(&r[1])?.to_string(),
        text_id: text(&r[2])?.to_string(),
        start: ord(&r[3])?,
        end: ord(&r[4])?,
        created_at: int(&r[5])?,
        deleted_at: opt_int(&r[6])?,
        merged_into: opt_text(&r[7])?,
    })
}

fn comment_row(r: &[Value]) -> Result<CommentRow, String> {
    Ok(CommentRow {
        id: text(&r[0])?.to_string(),
        tagging: text(&r[1])?.to_string(),
        body: text(&r[2])?.to_string(),
        created_at: int(&r[3])?,
        edited_at: opt_int(&r[4])?,
        deleted_at: opt_int(&r[5])?,
        author: opt_text(&r[6])?,
    })
}

impl<D: Db> Rows<'_, D> {
    fn ok<T: Default>(&self, r: Result<T, String>) -> T {
        r.unwrap_or_else(|e| {
            self.err.borrow_mut().get_or_insert(e);
            T::default()
        })
    }

    fn list<T>(&self, sql: &str, params: &[Value], f: fn(&[Value]) -> Result<T, String>) -> Vec<T> {
        let r = self.db.query(sql, params).and_then(|rows| rows.iter().map(|r| f(r)).collect());
        self.ok(r)
    }

    fn one<T>(&self, sql: &str, params: &[Value], f: fn(&[Value]) -> Result<T, String>) -> Option<T> {
        self.list(sql, params, f).into_iter().next()
    }
}

impl<D: Db> RowSource for Rows<'_, D> {
    fn tag(&self, id: &str) -> Option<TagRow> {
        let sql = format!("SELECT {TAG_COLS} FROM tags WHERE owner_id = ? AND id = ?");
        self.one(&sql, &[self.owner.into(), id.into()], tag_row)
    }

    fn tagging(&self, id: &str) -> Option<TaggingRow> {
        let sql = format!("SELECT {TAGGING_COLS} FROM taggings WHERE owner_id = ? AND id = ?");
        self.one(&sql, &[self.owner.into(), id.into()], tagging_row)
    }

    fn palette(&self) -> Option<PaletteRow> {
        let sql = "SELECT tags, updated_at FROM palettes WHERE owner_id = ?";
        self.one(sql, &[self.owner.into()], |r| {
            let tags = serde_json::from_str(text(&r[0])?).map_err(|e| e.to_string())?;
            Ok(PaletteRow { tags, updated_at: int(&r[1])? })
        })
    }

    fn comment(&self, id: &str) -> Option<CommentRow> {
        let sql = format!("SELECT {COMMENT_COLS} FROM comments WHERE owner_id = ? AND id = ?");
        self.one(&sql, &[self.owner.into(), id.into()], comment_row)
    }

    fn live_tag_named(&self, name: &str) -> Option<TagRow> {
        // Unicode case-insensitive, matching the TS implementation; an owner has few tags.
        let sql = format!("SELECT {TAG_COLS} FROM tags WHERE owner_id = ? AND deleted_at IS NULL");
        let wanted = name.to_lowercase();
        self.list(&sql, &[self.owner.into()], tag_row).into_iter().find(|t| t.name.to_lowercase() == wanted)
    }

    fn taggings_near(&self, tag: &str, text_id: &str, start: u32, end: u32) -> Vec<TaggingRow> {
        let sql = format!(
            "SELECT {TAGGING_COLS} FROM taggings WHERE owner_id = ? AND tag_id = ? AND text_id = ?
             AND deleted_at IS NULL AND start_ord <= ? AND end_ord >= ?"
        );
        self.list(&sql, &[self.owner.into(), tag.into(), text_id.into(), end.into(), start.into()], tagging_row)
    }

    fn live_taggings_of_tag(&self, tag: &str) -> Vec<TaggingRow> {
        let sql =
            format!("SELECT {TAGGING_COLS} FROM taggings WHERE owner_id = ? AND tag_id = ? AND deleted_at IS NULL");
        self.list(&sql, &[self.owner.into(), tag.into()], tagging_row)
    }

    fn change(&self, op_id: &str) -> Option<Change> {
        // A pruned change can't be reverted (the op fails with `unknown_op`).
        let sql = "SELECT payload FROM changes WHERE owner_id = ? AND op_id = ? AND payload <> ''";
        self.one(sql, &[self.owner.into(), op_id.into()], |r| parse_change(text(&r[0])?))
    }
}

/// `owner`'s live data, ready to copy into another space with [`merge`]: live tags, their live taggings, live
/// comments (re-pointed to the live end of their tagging's merge chain), and the palette.
///
/// # Errors
/// On storage failure.
pub fn export(db: &impl Db, owner: &str) -> Result<Vec<Row>, String> {
    let src = Rows { db, owner, err: RefCell::new(None) };
    let tags = src.list(
        &format!("SELECT {TAG_COLS} FROM tags WHERE owner_id = ? AND deleted_at IS NULL ORDER BY created_at, id"),
        &[owner.into()],
        tag_row,
    );
    let mut rows: Vec<Row> = vec![];
    for t in &tags {
        let mut taggings = src.live_taggings_of_tag(&t.id);
        taggings.sort_by(|a, b| (&a.text_id, a.start).cmp(&(&b.text_id, b.start)));
        rows.push(Row::Tag(t.clone()));
        rows.extend(taggings.into_iter().map(Row::Tagging));
    }
    let comments = src.list(
        &format!(
            "SELECT {COMMENT_COLS} FROM comments WHERE owner_id = ? AND deleted_at IS NULL ORDER BY created_at, id"
        ),
        &[owner.into()],
        comment_row,
    );
    let live_tags: std::collections::HashSet<&str> = tags.iter().map(|t| t.id.as_str()).collect();
    for mut c in comments {
        // Hidden comments (on erased taggings or deleted tags) stay behind, as they would in the UI.
        if let Ok(t) = ops::resolve_tagging(&src, &c.tagging) {
            if live_tags.contains(t.tag.as_str()) {
                c.tagging = t.id;
                rows.push(Row::Comment(c));
            }
        }
    }
    if let Some(p) = src.palette() {
        rows.push(Row::Palette(p));
    }
    match src.err.into_inner() {
        Some(e) => Err(e),
        None => Ok(rows),
    }
}

/// Copy rows [`export`]ed from owner `from` into `owner`'s data, as ordinary ops (so open clients receive them as
/// changes): a tag with the same name (case-insensitive) as one of `owner`'s live tags is combined with it
/// (taggings painted onto it, so overlaps merge), others are created with their ids and colors; comments follow
/// their taggings; palette tags are appended while there is room. Original timestamps are kept. Idempotent: op ids
/// derive from `from` and the row ids, so a repeated or interrupted merge never duplicates anything.
/// Returns the new changes.
///
/// # Errors
/// On storage failure (ops that are invalid here are skipped).
pub fn merge(db: &impl Db, owner: &str, from: &str, rows: &[Row], now: i64) -> Result<Vec<Change>, String> {
    let mut out = vec![];
    let mut run = |intent: Intent, key: &str| -> Result<Option<Change>, String> {
        let op = Op { op_id: format!("merge-{from}-{key}"), intent };
        match apply(db, owner, &op, now) {
            Ok(a) if !a.duplicate => {
                out.push(a.change.clone());
                Ok(Some(a.change))
            }
            Ok(a) => Ok(Some(a.change)),
            Err(ApplyError::Op(_) | ApplyError::Refused(_)) => Ok(None),
            Err(ApplyError::Internal(e)) => Err(e),
        }
    };
    let mut tag_ids: std::collections::HashMap<&str, String> = std::collections::HashMap::new();
    for r in rows {
        match r {
            Row::Tag(t) => {
                let src = Rows { db, owner, err: RefCell::new(None) };
                let mine = src.tag(&t.id).filter(|m| m.deleted_at.is_none());
                let target = if let Some(existing) = mine.or_else(|| src.live_tag_named(&t.name)) {
                    existing.id
                } else {
                    let intent = Intent::TagCreate {
                        id: t.id.clone(),
                        name: t.name.clone(),
                        color: t.color.clone(),
                        now: t.created_at,
                    };
                    run(intent, &t.id)?;
                    t.id.clone()
                };
                if let Some(e) = src.err.into_inner() {
                    return Err(e);
                }
                tag_ids.insert(&t.id, target);
            }
            Row::Tagging(t) => {
                let Some(tag) = tag_ids.get(t.tag.as_str()) else { continue };
                let intent = Intent::Paint {
                    tag: tag.clone(),
                    text_id: t.text_id.clone(),
                    start: t.start,
                    end: t.end,
                    new_id: t.id.clone(),
                    now: t.created_at,
                };
                run(intent, &t.id)?;
            }
            Row::Comment(c) => {
                // The tagging was painted with its own id; if it merged into an existing tagging, the new id is
                // never created, so attach to whichever live tagging of that tag now covers it.
                let Some(Row::Tagging(t)) = rows.iter().find(|r| matches!(r, Row::Tagging(t) if t.id == c.tagging))
                else {
                    continue;
                };
                let Some(tag) = tag_ids.get(t.tag.as_str()) else { continue };
                let src = Rows { db, owner, err: RefCell::new(None) };
                let target = src
                    .taggings_near(tag, &t.text_id, t.start, t.end)
                    .into_iter()
                    .find(|x| x.start <= t.start && x.end >= t.end)
                    .map(|x| x.id);
                if let Some(e) = src.err.into_inner() {
                    return Err(e);
                }
                let Some(tagging) = target else { continue };
                let intent =
                    Intent::CommentCreate { id: c.id.clone(), tagging, body: c.body.clone(), now: c.created_at };
                run(intent, &c.id)?;
            }
            Row::Palette(p) => {
                let src = Rows { db, owner, err: RefCell::new(None) };
                let mut tags = src.palette().map(|p| p.tags).unwrap_or_default();
                if let Some(e) = src.err.into_inner() {
                    return Err(e);
                }
                for t in p.tags.iter().filter_map(|t| tag_ids.get(t.as_str())) {
                    if tags.len() < ops::MAX_PALETTE && !tags.contains(t) {
                        tags.push(t.clone());
                    }
                }
                run(Intent::PaletteSet { tags, now }, "palette")?;
            }
        }
    }
    Ok(out)
}

#[cfg(feature = "rusqlite")]
mod sqlite {
    use super::{Db, Value};
    use rusqlite::types::{Value as V, ValueRef};

    fn bind(params: &[Value]) -> Vec<V> {
        params
            .iter()
            .map(|p| match p {
                Value::Null => V::Null,
                Value::Int(n) => V::Integer(*n),
                Value::Text(s) => V::Text(s.clone()),
            })
            .collect()
    }

    impl Db for rusqlite::Connection {
        fn exec(&self, sql: &str, params: &[Value]) -> Result<(), String> {
            let mut stmt = self.prepare_cached(sql).map_err(|e| e.to_string())?;
            stmt.execute(rusqlite::params_from_iter(bind(params))).map(|_| ()).map_err(|e| e.to_string())
        }

        fn query(&self, sql: &str, params: &[Value]) -> Result<Vec<Vec<Value>>, String> {
            let mut stmt = self.prepare_cached(sql).map_err(|e| e.to_string())?;
            let n = stmt.column_count();
            let rows = stmt
                .query_map(rusqlite::params_from_iter(bind(params)), |r| {
                    (0..n)
                        .map(|i| {
                            Ok(match r.get_ref(i)? {
                                ValueRef::Null => Value::Null,
                                ValueRef::Integer(n) => Value::Int(n),
                                ValueRef::Text(t) => Value::Text(String::from_utf8_lossy(t).into_owned()),
                                ValueRef::Real(_) | ValueRef::Blob(_) => {
                                    return Err(rusqlite::Error::InvalidColumnType(
                                        i,
                                        "unsupported".into(),
                                        rusqlite::types::Type::Blob,
                                    ))
                                }
                            })
                        })
                        .collect()
                })
                .map_err(|e| e.to_string())?;
            rows.collect::<Result<_, _>>().map_err(|e| e.to_string())
        }
    }
}
