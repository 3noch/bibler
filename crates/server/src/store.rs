//! Persistence behind the `Store` trait (SQLite now; Postgres or a sync engine must stay a contained swap).
//!
//! SQLite: one dedicated writer thread owns the write connection (ops are serialized per database, which also
//! makes per-owner `seq` assignment trivial); reads use a small pool of extra connections (WAL mode). Every op is
//! one transaction: idempotency check, `ops::effect` against the owner's rows, row upserts, change append, commit,
//! then the commit listener runs on the writer thread (so listeners see changes in seq order).

use ops::{effect, Change, CommentRow, Op, OpError, PaletteRow, Row, RowSource, TagRow, TaggingRow};
use rusqlite::{params, Connection, OptionalExtension, Transaction};
use std::cell::RefCell;
use std::path::Path;
use std::sync::{mpsc, Arc, Mutex};

pub type OwnerId = String;

#[derive(Debug)]
pub enum ApplyError {
    /// The op is invalid against the owner's rows (a stable code the client understands).
    Op(OpError),
    /// Storage failure; the op had no effect.
    Internal(String),
}

impl From<rusqlite::Error> for ApplyError {
    fn from(e: rusqlite::Error) -> Self {
        ApplyError::Internal(e.to_string())
    }
}

#[derive(Debug, Clone)]
pub struct Applied {
    pub change: Change,
    /// The op id was already applied; `change` is the original result.
    pub duplicate: bool,
}

/// Called on the writer thread after each commit, in seq order.
pub type CommitListener = Box<dyn Fn(&str, &Change) + Send + Sync>;

/// Blocking interface (call it from `spawn_blocking`). Every method is scoped to one owner.
pub trait Store: Send + Sync {
    /// # Errors
    /// On storage failure.
    fn create_owner(&self, id: &str, now: i64) -> Result<(), String>;
    /// # Errors
    /// On storage failure.
    fn create_session(&self, token_hash: &str, owner: &str, now: i64) -> Result<(), String>;
    /// # Errors
    /// On storage failure.
    fn session_owner(&self, token_hash: &str) -> Result<Option<OwnerId>, String>;
    /// Apply an op exactly once (by `op_id`).
    ///
    /// # Errors
    /// `ApplyError::Op` if the op is invalid; `ApplyError::Internal` on storage failure.
    fn apply(&self, owner: &str, op: &Op, now: i64) -> Result<Applied, ApplyError>;
    /// Changes with `seq > after`, oldest first, at most `limit`.
    ///
    /// # Errors
    /// On storage failure.
    fn changes_since(&self, owner: &str, after: u64, limit: usize) -> Result<Vec<Change>, String>;
}

const SCHEMA: &str = "
PRAGMA journal_mode = WAL;
CREATE TABLE IF NOT EXISTS owners (id TEXT PRIMARY KEY, created_at INTEGER NOT NULL);
CREATE TABLE IF NOT EXISTS sessions (
  token_hash TEXT PRIMARY KEY,
  owner_id TEXT NOT NULL REFERENCES owners(id),
  created_at INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS tags (
  id TEXT PRIMARY KEY,
  owner_id TEXT NOT NULL REFERENCES owners(id),
  name TEXT NOT NULL,
  color TEXT NOT NULL,
  description TEXT,
  created_at INTEGER NOT NULL,
  updated_at INTEGER NOT NULL,
  deleted_at INTEGER,
  UNIQUE (owner_id, id)
);
-- Backstop only (ASCII lower()); the real rule (Unicode case-insensitive) is enforced by ops::effect.
CREATE UNIQUE INDEX IF NOT EXISTS tags_live_name ON tags (owner_id, lower(name)) WHERE deleted_at IS NULL;
CREATE TABLE IF NOT EXISTS taggings (
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
);
CREATE INDEX IF NOT EXISTS taggings_live ON taggings (owner_id, tag_id, text_id, start_ord) WHERE deleted_at IS NULL;
CREATE TABLE IF NOT EXISTS palettes (
  owner_id TEXT PRIMARY KEY REFERENCES owners(id),
  tags TEXT NOT NULL,
  updated_at INTEGER NOT NULL
);
-- Ownership of `tagging_id` is checked by ops::effect (owner-scoped reads); the FK is a backstop.
CREATE TABLE IF NOT EXISTS comments (
  id TEXT PRIMARY KEY,
  owner_id TEXT NOT NULL REFERENCES owners(id),
  tagging_id TEXT NOT NULL REFERENCES taggings(id),
  body TEXT NOT NULL,
  created_at INTEGER NOT NULL,
  edited_at INTEGER,
  deleted_at INTEGER
);
CREATE INDEX IF NOT EXISTS comments_by_tagging ON comments (owner_id, tagging_id);
CREATE TABLE IF NOT EXISTS changes (
  owner_id TEXT NOT NULL,
  seq INTEGER NOT NULL,
  op_id TEXT NOT NULL,
  kind TEXT NOT NULL,
  payload TEXT NOT NULL,
  created_at INTEGER NOT NULL,
  PRIMARY KEY (owner_id, seq),
  UNIQUE (owner_id, op_id)
);
PRAGMA user_version = 2;
";

fn connect(path: &Path) -> rusqlite::Result<Connection> {
    let conn = Connection::open(path)?;
    conn.busy_timeout(std::time::Duration::from_secs(5))?;
    conn.execute_batch("PRAGMA foreign_keys = ON; PRAGMA synchronous = NORMAL;")?;
    Ok(conn)
}

type Job = Box<dyn FnOnce(&mut Connection) + Send>;

pub struct SqliteStore {
    writer: Mutex<mpsc::Sender<Job>>,
    readers: Mutex<Vec<Connection>>,
    path: std::path::PathBuf,
    listener: Arc<CommitListener>,
}

impl SqliteStore {
    /// Open (creating if needed) the database at `path`.
    ///
    /// # Errors
    /// If the database cannot be opened or migrated.
    pub fn open(path: &Path, listener: CommitListener) -> Result<Self, String> {
        let mut conn = connect(path).map_err(|e| format!("{}: {e}", path.display()))?;
        conn.execute_batch(SCHEMA).map_err(|e| format!("schema: {e}"))?;
        let (tx, rx) = mpsc::channel::<Job>();
        std::thread::Builder::new()
            .name("sqlite-writer".into())
            .spawn(move || {
                while let Ok(job) = rx.recv() {
                    job(&mut conn);
                }
            })
            .map_err(|e| e.to_string())?;
        Ok(SqliteStore {
            writer: Mutex::new(tx),
            readers: Mutex::new(vec![]),
            path: path.to_path_buf(),
            listener: Arc::new(listener),
        })
    }

    fn write<T: Send + 'static>(&self, f: impl FnOnce(&mut Connection) -> T + Send + 'static) -> Result<T, String> {
        let (tx, rx) = mpsc::sync_channel(1);
        let job: Job = Box::new(move |conn| {
            let _ = tx.send(f(conn));
        });
        self.writer.lock().map_err(|_| "writer lock poisoned")?.send(job).map_err(|_| "writer thread is gone")?;
        rx.recv().map_err(|_| "writer thread dropped the job".to_string())
    }

    fn read<T>(&self, f: impl FnOnce(&Connection) -> rusqlite::Result<T>) -> Result<T, String> {
        let conn = self.readers.lock().map_err(|_| "reader lock poisoned")?.pop();
        let conn = match conn {
            Some(c) => c,
            None => connect(&self.path).map_err(|e| e.to_string())?,
        };
        let out = f(&conn).map_err(|e| e.to_string());
        if let Ok(mut pool) = self.readers.lock() {
            if pool.len() < 8 {
                pool.push(conn);
            }
        }
        out
    }
}

impl SqliteStore {
    /// Load rows and recorded changes for `owner` as-is (no `effect`): fixtures, migrations, imports.
    ///
    /// # Errors
    /// On storage failure, or if an id belongs to another owner.
    pub fn import(&self, owner: &str, rows: &[Row], changes: &[Change]) -> Result<(), String> {
        let (owner, rows, changes) = (owner.to_string(), rows.to_vec(), changes.to_vec());
        self.write(move |conn| -> Result<(), String> {
            let tx = conn.transaction().map_err(|e| e.to_string())?;
            let mut rows = rows;
            rows.sort_by(|a, b| a.key().cmp(&b.key())); // tags before the taggings that reference them
            for r in &rows {
                if !write_row(&tx, &owner, r).map_err(|e| e.to_string())? {
                    return Err(format!("{:?} belongs to another owner", r.key()));
                }
            }
            for c in &changes {
                let payload = serde_json::to_string(c).map_err(|e| e.to_string())?;
                tx.execute(
                    "INSERT INTO changes (owner_id, seq, op_id, kind, payload, created_at) VALUES (?1, ?2, ?3, ?4, ?5, 0)",
                    params![owner, to_sql_int(c.seq), c.op_id, c.kind, payload],
                )
                .map_err(|e| e.to_string())?;
            }
            tx.commit().map_err(|e| e.to_string())
        })?
    }
}

impl Store for SqliteStore {
    fn create_owner(&self, id: &str, now: i64) -> Result<(), String> {
        let id = id.to_string();
        self.write(move |c| c.execute("INSERT INTO owners (id, created_at) VALUES (?1, ?2)", params![id, now]))?
            .map(|_| ())
            .map_err(|e| e.to_string())
    }

    fn create_session(&self, token_hash: &str, owner: &str, now: i64) -> Result<(), String> {
        let (h, o) = (token_hash.to_string(), owner.to_string());
        self.write(move |c| {
            c.execute("INSERT INTO sessions (token_hash, owner_id, created_at) VALUES (?1, ?2, ?3)", params![h, o, now])
        })?
        .map(|_| ())
        .map_err(|e| e.to_string())
    }

    fn session_owner(&self, token_hash: &str) -> Result<Option<OwnerId>, String> {
        self.read(|c| {
            c.query_row("SELECT owner_id FROM sessions WHERE token_hash = ?1", [token_hash], |r| r.get(0)).optional()
        })
    }

    fn apply(&self, owner: &str, op: &Op, now: i64) -> Result<Applied, ApplyError> {
        let (owner, op, listener) = (owner.to_string(), op.clone(), self.listener.clone());
        self.write(move |conn| {
            let applied = apply_tx(conn, &owner, &op, now)?;
            if !applied.duplicate {
                listener(&owner, &applied.change);
            }
            Ok(applied)
        })
        .map_err(ApplyError::Internal)?
    }

    fn changes_since(&self, owner: &str, after: u64, limit: usize) -> Result<Vec<Change>, String> {
        self.read(|c| {
            let mut stmt =
                c.prepare_cached("SELECT payload FROM changes WHERE owner_id = ?1 AND seq > ?2 ORDER BY seq LIMIT ?3")?;
            let rows =
                stmt.query_map(params![owner, to_sql_int(after), to_sql_int(limit as u64)], |r| r.get::<_, String>(0))?;
            rows.map(|p| parse_change(&p?)).collect()
        })
    }
}

fn to_sql_int(n: u64) -> i64 {
    i64::try_from(n).unwrap_or(i64::MAX)
}

fn parse_change(payload: &str) -> rusqlite::Result<Change> {
    serde_json::from_str(payload)
        .map_err(|e| rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, Box::new(e)))
}

fn apply_tx(conn: &mut Connection, owner: &str, op: &Op, now: i64) -> Result<Applied, ApplyError> {
    let tx = conn.transaction()?;
    let existing: Option<String> = tx
        .query_row("SELECT payload FROM changes WHERE owner_id = ?1 AND op_id = ?2", [owner, &op.op_id], |r| r.get(0))
        .optional()?;
    if let Some(p) = existing {
        return Ok(Applied { change: parse_change(&p)?, duplicate: true });
    }
    let mut change = {
        let src = TxRows { tx: &tx, owner, err: RefCell::new(None) };
        let result = effect(&src, op);
        if let Some(e) = src.err.into_inner() {
            return Err(e.into()); // a failed read: the computed effect cannot be trusted
        }
        result.map_err(ApplyError::Op)?
    };
    let seq: i64 =
        tx.query_row("SELECT COALESCE(MAX(seq), 0) + 1 FROM changes WHERE owner_id = ?1", [owner], |r| r.get(0))?;
    change.seq = u64::try_from(seq).map_err(|e| ApplyError::Internal(e.to_string()))?;
    for rc in &change.rows {
        if !write_row(&tx, owner, &rc.after)? {
            return Err(ApplyError::Op(OpError::IdExists)); // the id belongs to another owner
        }
    }
    let payload = serde_json::to_string(&change).map_err(|e| ApplyError::Internal(e.to_string()))?;
    tx.execute(
        "INSERT INTO changes (owner_id, seq, op_id, kind, payload, created_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![owner, seq, change.op_id, change.kind, payload, now],
    )?;
    tx.commit()?;
    Ok(Applied { change, duplicate: false })
}

/// Upsert a row for `owner`. Returns false if the id exists for a different owner.
fn write_row(tx: &Transaction, owner: &str, row: &Row) -> rusqlite::Result<bool> {
    let n = match row {
        Row::Tag(t) => tx.execute(
            "INSERT INTO tags (id, owner_id, name, color, created_at, updated_at, deleted_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
             ON CONFLICT (id) DO UPDATE SET name = excluded.name, color = excluded.color,
               updated_at = excluded.updated_at, deleted_at = excluded.deleted_at
             WHERE tags.owner_id = excluded.owner_id",
            params![t.id, owner, t.name, t.color, t.created_at, t.updated_at, t.deleted_at],
        )?,
        Row::Tagging(t) => tx.execute(
            "INSERT INTO taggings (id, owner_id, tag_id, text_id, start_ord, end_ord, created_at, deleted_at, merged_into)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
             ON CONFLICT (id) DO UPDATE SET start_ord = excluded.start_ord, end_ord = excluded.end_ord,
               deleted_at = excluded.deleted_at, merged_into = excluded.merged_into
             WHERE taggings.owner_id = excluded.owner_id",
            params![t.id, owner, t.tag, t.text_id, t.start, t.end, t.created_at, t.deleted_at, t.merged_into],
        )?,
        Row::Comment(c) => tx.execute(
            "INSERT INTO comments (id, owner_id, tagging_id, body, created_at, edited_at, deleted_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
             ON CONFLICT (id) DO UPDATE SET body = excluded.body, edited_at = excluded.edited_at,
               deleted_at = excluded.deleted_at
             WHERE comments.owner_id = excluded.owner_id",
            params![c.id, owner, c.tagging, c.body, c.created_at, c.edited_at, c.deleted_at],
        )?,
        Row::Palette(p) => tx.execute(
            "INSERT INTO palettes (owner_id, tags, updated_at) VALUES (?1, ?2, ?3)
             ON CONFLICT (owner_id) DO UPDATE SET tags = excluded.tags, updated_at = excluded.updated_at",
            params![owner, serde_json::to_string(&p.tags).unwrap_or_default(), p.updated_at],
        )?,
    };
    Ok(n == 1)
}

/// `RowSource` over a transaction. The trait is infallible, so the first SQL error is recorded and the caller
/// discards the effect.
struct TxRows<'a> {
    tx: &'a Transaction<'a>,
    owner: &'a str,
    err: RefCell<Option<rusqlite::Error>>,
}

const TAG_COLS: &str = "id, name, color, created_at, updated_at, deleted_at";
const TAGGING_COLS: &str = "id, tag_id, text_id, start_ord, end_ord, created_at, deleted_at, merged_into";

fn tag_row(r: &rusqlite::Row) -> rusqlite::Result<TagRow> {
    Ok(TagRow {
        id: r.get(0)?,
        name: r.get(1)?,
        color: r.get(2)?,
        created_at: r.get(3)?,
        updated_at: r.get(4)?,
        deleted_at: r.get(5)?,
    })
}

fn tagging_row(r: &rusqlite::Row) -> rusqlite::Result<TaggingRow> {
    Ok(TaggingRow {
        id: r.get(0)?,
        tag: r.get(1)?,
        text_id: r.get(2)?,
        start: r.get(3)?,
        end: r.get(4)?,
        created_at: r.get(5)?,
        deleted_at: r.get(6)?,
        merged_into: r.get(7)?,
    })
}

impl TxRows<'_> {
    fn ok<T: Default>(&self, r: rusqlite::Result<T>) -> T {
        r.unwrap_or_else(|e| {
            self.err.borrow_mut().get_or_insert(e);
            T::default()
        })
    }

    fn list<T>(
        &self,
        sql: &str,
        params: impl rusqlite::Params,
        f: fn(&rusqlite::Row) -> rusqlite::Result<T>,
    ) -> Vec<T> {
        let r = self.tx.prepare_cached(sql).and_then(|mut s| s.query_map(params, f)?.collect());
        self.ok(r)
    }
}

impl RowSource for TxRows<'_> {
    fn tag(&self, id: &str) -> Option<TagRow> {
        let sql = format!("SELECT {TAG_COLS} FROM tags WHERE owner_id = ?1 AND id = ?2");
        self.ok(self.tx.query_row(&sql, [self.owner, id], tag_row).optional())
    }

    fn tagging(&self, id: &str) -> Option<TaggingRow> {
        let sql = format!("SELECT {TAGGING_COLS} FROM taggings WHERE owner_id = ?1 AND id = ?2");
        self.ok(self.tx.query_row(&sql, [self.owner, id], tagging_row).optional())
    }

    fn palette(&self) -> Option<PaletteRow> {
        let r = self
            .tx
            .query_row("SELECT tags, updated_at FROM palettes WHERE owner_id = ?1", [self.owner], |r| {
                Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?))
            })
            .optional();
        self.ok(r)
            .map(|(tags, updated_at)| PaletteRow { tags: serde_json::from_str(&tags).unwrap_or_default(), updated_at })
    }

    fn comment(&self, id: &str) -> Option<CommentRow> {
        let sql = "SELECT id, tagging_id, body, created_at, edited_at, deleted_at FROM comments
                   WHERE owner_id = ?1 AND id = ?2";
        let r = self
            .tx
            .query_row(sql, [self.owner, id], |r| {
                Ok(CommentRow {
                    id: r.get(0)?,
                    tagging: r.get(1)?,
                    body: r.get(2)?,
                    created_at: r.get(3)?,
                    edited_at: r.get(4)?,
                    deleted_at: r.get(5)?,
                })
            })
            .optional();
        self.ok(r)
    }

    fn live_tag_named(&self, name: &str) -> Option<TagRow> {
        // Unicode case-insensitive, matching the TS implementation; an owner has few tags.
        let sql = format!("SELECT {TAG_COLS} FROM tags WHERE owner_id = ?1 AND deleted_at IS NULL");
        let wanted = name.to_lowercase();
        self.list(&sql, [self.owner], tag_row).into_iter().find(|t| t.name.to_lowercase() == wanted)
    }

    fn taggings_near(&self, tag: &str, text_id: &str, start: u32, end: u32) -> Vec<TaggingRow> {
        let sql = format!(
            "SELECT {TAGGING_COLS} FROM taggings WHERE owner_id = ?1 AND tag_id = ?2 AND text_id = ?3
             AND deleted_at IS NULL AND start_ord <= ?4 AND end_ord >= ?5"
        );
        self.list(&sql, params![self.owner, tag, text_id, end, start], tagging_row)
    }

    fn live_taggings_of_tag(&self, tag: &str) -> Vec<TaggingRow> {
        let sql =
            format!("SELECT {TAGGING_COLS} FROM taggings WHERE owner_id = ?1 AND tag_id = ?2 AND deleted_at IS NULL");
        self.list(&sql, [self.owner, tag], tagging_row)
    }

    fn change(&self, op_id: &str) -> Option<Change> {
        let r = self
            .tx
            .query_row("SELECT payload FROM changes WHERE owner_id = ?1 AND op_id = ?2", [self.owner, op_id], |r| {
                r.get::<_, String>(0)
            })
            .optional();
        let p = self.ok(r)?;
        let change = parse_change(&p);
        self.ok(change.map(Some))
    }
}
