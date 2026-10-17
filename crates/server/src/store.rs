//! Persistence for the self-hosted server, laid out like the Cloudflare deployment so both run the same SQL:
//! - `accounts.sqlite`: owners and sessions (D1 there).
//! - `spaces/<space>.sqlite`: one database per space (a Durable Object there), schema and ops in the `space` crate.
//!   A personal space's id is its owner's id.
//!
//! One writer thread owns every write connection (ops are serialized, so each space's `seq` is trivially ordered and
//! the commit listener sees changes in seq order); reads use small per-space pools of extra connections (WAL).

use ops::{Change, Op, Row};
use rusqlite::{params, Connection, OptionalExtension};
use space::{Applied, ApplyError, Db, Value};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{mpsc, Arc, Mutex};

pub type OwnerId = String;

/// Called on the writer thread after each commit, in seq order: `(space, change)`.
pub type CommitListener = Box<dyn Fn(&str, &Change) + Send + Sync>;

/// Blocking interface (call it from `spawn_blocking`).
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
    /// Apply `author`'s op to `space`'s rows exactly once (by `op_id`). The space is the data owner (a personal
    /// space's id is its owner's), so a group's members share its rows (`space::apply_by`).
    ///
    /// # Errors
    /// `ApplyError::Op`/`Refused` if the op is invalid or not allowed; `ApplyError::Internal` on storage failure.
    fn apply(&self, space: &str, author: &str, op: &Op, now: i64) -> Result<Applied, ApplyError>;
    /// Run a statement on the accounts database (`api::groups`).
    ///
    /// # Errors
    /// On SQL failure.
    fn accounts_exec(&self, sql: &str, params: Vec<Value>) -> Result<(), String>;
    /// # Errors
    /// On SQL failure.
    fn accounts_query(&self, sql: &str, params: Vec<Value>) -> Result<Vec<Vec<Value>>, String>;
    /// For a client whose last seq is `after`: `None` if the changes after it can be replayed, else the space's
    /// whole state (`space::reset`; `pending` = the client's unconfirmed op ids).
    ///
    /// # Errors
    /// On storage failure.
    fn catch_up(&self, space: &str, after: u64, pending: &[String]) -> Result<Option<space::Reset>, String>;
    /// The space's changes with `seq > after`, oldest first, at most `limit`.
    ///
    /// # Errors
    /// On storage failure.
    fn changes_since(&self, space: &str, after: u64, limit: usize) -> Result<Vec<Change>, String>;
    /// # Errors
    /// On storage failure.
    fn delete_session(&self, token_hash: &str) -> Result<(), String>;
    /// The owner signed in with this external identity, if any.
    ///
    /// # Errors
    /// On storage failure.
    fn identity_owner(&self, provider: &str, subject: &str) -> Result<Option<OwnerId>, String>;
    /// Attach an external identity to `owner` (making it an account).
    ///
    /// # Errors
    /// On storage failure (including an identity that is already attached).
    fn add_identity(&self, id: &Identity, owner: &str, now: i64) -> Result<(), String>;
    /// The identity `owner` signs in with (the first one), or `None` for an anonymous owner.
    ///
    /// # Errors
    /// On storage failure.
    fn owner_identity(&self, owner: &str) -> Result<Option<Identity>, String>;
    /// Make `to` the author of `from`'s comments in group `space` (`space::reassign_author`), publishing the change.
    ///
    /// # Errors
    /// On storage failure.
    fn reassign_author(&self, space: &str, from: &str, to: &str, now: i64) -> Result<(), String>;
    /// Copy `from`'s live data (in space `from_space`) into `into`'s (in `into_space`), see `space::merge`.
    /// Returns the number of new changes.
    ///
    /// # Errors
    /// On storage failure.
    fn merge(&self, from_space: &str, from: &str, into_space: &str, into: &str, now: i64) -> Result<usize, String>;
}

pub use api::Identity;

use api::accounts as sql;

fn connect(path: &Path) -> Result<Connection, String> {
    let conn = Connection::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
    conn.busy_timeout(std::time::Duration::from_secs(5)).map_err(|e| e.to_string())?;
    conn.execute_batch("PRAGMA journal_mode = WAL; PRAGMA foreign_keys = ON; PRAGMA synchronous = NORMAL;")
        .map_err(|e| e.to_string())?;
    Ok(conn)
}

/// Space ids name files: the same shape as client ids.
#[must_use]
pub fn valid_space(id: &str) -> bool {
    (1..=64).contains(&id.len()) && id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

/// Write connections, owned by the writer thread.
struct Writer {
    accounts: Connection,
    spaces: HashMap<String, Connection>,
}

/// Open connections kept per kind (each is a few file handles plus page cache).
const MAX_OPEN_SPACES: usize = 256;
const MAX_POOLED_READERS: usize = 4;

impl Writer {
    fn space(&mut self, dir: &Path, id: &str) -> Result<&Connection, String> {
        if !self.spaces.contains_key(id) {
            if self.spaces.len() >= MAX_OPEN_SPACES {
                self.spaces.clear(); // crude, but correct: connections reopen on demand
            }
            let conn = open_space(dir, id)?;
            self.spaces.insert(id.to_string(), conn);
        }
        Ok(&self.spaces[id])
    }
}

fn space_path(dir: &Path, id: &str) -> Result<PathBuf, String> {
    if !valid_space(id) {
        return Err(format!("bad space id {id:?}"));
    }
    Ok(dir.join("spaces").join(format!("{id}.sqlite")))
}

fn open_space(dir: &Path, id: &str) -> Result<Connection, String> {
    let conn = connect(&space_path(dir, id)?)?;
    space::init(&conn)?;
    Ok(conn)
}

type Job = Box<dyn FnOnce(&mut Writer) + Send>;

pub struct SqliteStore {
    writer: Mutex<mpsc::Sender<Job>>,
    thread: Mutex<Option<std::thread::JoinHandle<()>>>,
    readers: Mutex<HashMap<String, Vec<Connection>>>,
    dir: PathBuf,
    listener: Arc<CommitListener>,
    retention_ms: i64,
}

impl SqliteStore {
    /// Open (creating if needed) the data directory `dir`.
    ///
    /// # Errors
    /// If the databases cannot be opened or migrated.
    pub fn open(dir: &Path, listener: CommitListener) -> Result<Self, String> {
        std::fs::create_dir_all(dir.join("spaces")).map_err(|e| format!("{}: {e}", dir.display()))?;
        let accounts = connect(&dir.join("accounts.sqlite"))?;
        for statement in sql::schema_statements() {
            accounts.execute_batch(statement).map_err(|e| format!("accounts schema: {e}"))?;
        }
        let mut w = Writer { accounts, spaces: HashMap::new() };
        let (tx, rx) = mpsc::channel::<Job>();
        let thread = std::thread::Builder::new()
            .name("sqlite-writer".into())
            .spawn(move || {
                while let Ok(job) = rx.recv() {
                    job(&mut w);
                }
            })
            .map_err(|e| e.to_string())?;
        Ok(SqliteStore {
            writer: Mutex::new(tx),
            thread: Mutex::new(Some(thread)),
            readers: Mutex::new(HashMap::new()),
            dir: dir.to_path_buf(),
            listener: Arc::new(listener),
            retention_ms: space::RETENTION_MS,
        })
    }

    /// Keep changes replayable for `ms` (default `space::RETENTION_MS`).
    #[must_use]
    pub fn retention(mut self, ms: i64) -> Self {
        self.retention_ms = ms;
        self
    }

    /// Close every connection and stop the writer thread (later calls fail).
    pub fn close(&self) {
        if let Ok(mut r) = self.readers.lock() {
            r.clear();
        }
        let _ = self.write(|w| w.spaces.clear());
        if let Ok(mut tx) = self.writer.lock() {
            *tx = mpsc::channel().0; // drop the sender: the thread's loop ends
        }
        if let Some(t) = self.thread.lock().ok().and_then(|mut t| t.take()) {
            let _ = t.join();
        }
    }

    fn write<T: Send + 'static>(&self, f: impl FnOnce(&mut Writer) -> T + Send + 'static) -> Result<T, String> {
        let (tx, rx) = mpsc::sync_channel(1);
        let job: Job = Box::new(move |w| {
            let _ = tx.send(f(w));
        });
        self.writer.lock().map_err(|_| "writer lock poisoned")?.send(job).map_err(|_| "writer thread is gone")?;
        rx.recv().map_err(|_| "writer thread dropped the job".to_string())
    }

    /// Run `f` on a read connection to `key` (a space id, or "" for accounts).
    fn read<T>(&self, key: &str, f: impl FnOnce(&Connection) -> Result<T, String>) -> Result<T, String> {
        let pooled = self.readers.lock().map_err(|_| "reader lock poisoned")?.get_mut(key).and_then(Vec::pop);
        let conn = match pooled {
            Some(c) => c,
            None if key.is_empty() => connect(&self.dir.join("accounts.sqlite"))?,
            None => {
                let path = space_path(&self.dir, key)?;
                if !path.exists() {
                    // Let the writer create and initialize it, so a reader never sees a half-made schema.
                    let (dir, id) = (self.dir.clone(), key.to_string());
                    self.write(move |w| w.space(&dir, &id).map(|_| ()))??;
                }
                connect(&path)?
            }
        };
        let out = f(&conn);
        if let Ok(mut pool) = self.readers.lock() {
            if pool.len() >= MAX_OPEN_SPACES {
                pool.clear();
            }
            let list = pool.entry(key.to_string()).or_default();
            if list.len() < MAX_POOLED_READERS {
                list.push(conn);
            }
        }
        out
    }

    /// Load `owner`'s rows and recorded changes into `space` as-is (no `effect`): migrations and imports.
    ///
    /// # Errors
    /// On storage failure, or if an id belongs to another member.
    pub fn import(&self, space: &str, owner: &str, rows: &[Row], changes: &[Change]) -> Result<(), String> {
        let (dir, space, owner, rows, changes) =
            (self.dir.clone(), space.to_string(), owner.to_string(), rows.to_vec(), changes.to_vec());
        self.write(move |w| {
            let conn = w.space(&dir, &space)?;
            let tx = conn.unchecked_transaction().map_err(|e| e.to_string())?;
            space::import(&*tx, &owner, &rows, &changes)?;
            tx.commit().map_err(|e| e.to_string())
        })?
    }

    /// Owners and session hashes, as-is (migrations).
    ///
    /// # Errors
    /// On storage failure.
    pub fn import_accounts(
        &self,
        owners: Vec<(String, i64)>,
        sessions: Vec<(String, String, i64)>,
    ) -> Result<(), String> {
        self.write(move |w| {
            let tx = w.accounts.unchecked_transaction().map_err(|e| e.to_string())?;
            for (id, at) in &owners {
                tx.execute(sql::CREATE_OWNER, params![id, at]).map_err(|e| e.to_string())?;
            }
            for (hash, owner, at) in &sessions {
                tx.execute(sql::CREATE_SESSION, params![hash, owner, at]).map_err(|e| e.to_string())?;
            }
            tx.commit().map_err(|e| e.to_string())
        })?
    }
}

impl Store for SqliteStore {
    fn create_owner(&self, id: &str, now: i64) -> Result<(), String> {
        let id = id.to_string();
        self.write(move |w| w.accounts.execute(sql::CREATE_OWNER, params![id, now]))?
            .map(|_| ())
            .map_err(|e| e.to_string())
    }

    fn create_session(&self, token_hash: &str, owner: &str, now: i64) -> Result<(), String> {
        let (h, o) = (token_hash.to_string(), owner.to_string());
        self.write(move |w| w.accounts.execute(sql::CREATE_SESSION, params![h, o, now]))?
            .map(|_| ())
            .map_err(|e| e.to_string())
    }

    fn session_owner(&self, token_hash: &str) -> Result<Option<OwnerId>, String> {
        self.read("", |c| {
            c.query_row(sql::SESSION_OWNER, [token_hash], |r| r.get(0)).optional().map_err(|e| e.to_string())
        })
    }

    fn apply(&self, space: &str, author: &str, op: &Op, now: i64) -> Result<Applied, ApplyError> {
        let (dir, space, author, op, listener) =
            (self.dir.clone(), space.to_string(), author.to_string(), op.clone(), self.listener.clone());
        let prune_before = now.saturating_sub(self.retention_ms);
        self.write(move |w| {
            let conn = w.space(&dir, &space)?;
            let tx = conn.unchecked_transaction().map_err(|e| e.to_string())?;
            let applied = space::apply_by(&*tx, &space, &author, &op, now)?;
            if !applied.duplicate {
                space::prune(&*tx, prune_before, space::PRUNE_BATCH)?; // a little per op keeps up
            }
            tx.commit().map_err(|e| e.to_string())?;
            if !applied.duplicate {
                listener(&space, &applied.change);
            }
            Ok(applied)
        })
        .map_err(ApplyError::Internal)?
    }

    fn accounts_exec(&self, sql: &str, params: Vec<Value>) -> Result<(), String> {
        let sql = sql.to_string();
        self.write(move |w| w.accounts.exec(&sql, &params))?
    }

    fn accounts_query(&self, sql: &str, params: Vec<Value>) -> Result<Vec<Vec<Value>>, String> {
        self.read("", |c| c.query(sql, &params))
    }

    fn catch_up(&self, space: &str, after: u64, pending: &[String]) -> Result<Option<space::Reset>, String> {
        self.read(space, |c| {
            let tx = c.unchecked_transaction().map_err(|e| e.to_string())?; // one snapshot for every read
            if after >= space::pruned_through(&*tx)? {
                return Ok(None);
            }
            space::reset(&*tx, space, pending).map(Some)
        })
    }

    fn changes_since(&self, space: &str, after: u64, limit: usize) -> Result<Vec<Change>, String> {
        self.read(space, |c| space::changes_since(c, after, limit))
    }

    fn delete_session(&self, token_hash: &str) -> Result<(), String> {
        let h = token_hash.to_string();
        self.write(move |w| w.accounts.execute(sql::DELETE_SESSION, [h]))?.map(|_| ()).map_err(|e| e.to_string())
    }

    fn identity_owner(&self, provider: &str, subject: &str) -> Result<Option<OwnerId>, String> {
        self.read("", |c| {
            c.query_row(sql::IDENTITY_OWNER, [provider, subject], |r| r.get(0)).optional().map_err(|e| e.to_string())
        })
    }

    fn add_identity(&self, id: &Identity, owner: &str, now: i64) -> Result<(), String> {
        let (id, owner) = (id.clone(), owner.to_string());
        self.write(move |w| {
            w.accounts.execute(sql::ADD_IDENTITY, params![id.provider, id.subject, owner, id.email, id.name, now])
        })?
        .map(|_| ())
        .map_err(|e| e.to_string())
    }

    fn owner_identity(&self, owner: &str) -> Result<Option<Identity>, String> {
        self.read("", |c| {
            c.query_row(sql::OWNER_IDENTITY, [owner], |r| {
                Ok(Identity { provider: r.get(0)?, subject: r.get(1)?, email: r.get(2)?, name: r.get(3)? })
            })
            .optional()
            .map_err(|e| e.to_string())
        })
    }

    fn reassign_author(&self, space: &str, from: &str, to: &str, now: i64) -> Result<(), String> {
        let (dir, space, from, to, listener) =
            (self.dir.clone(), space.to_string(), from.to_string(), to.to_string(), self.listener.clone());
        self.write(move |w| {
            let conn = w.space(&dir, &space)?;
            let tx = conn.unchecked_transaction().map_err(|e| e.to_string())?;
            let change = space::reassign_author(&*tx, &space, &from, &to, now)?;
            tx.commit().map_err(|e| e.to_string())?;
            if let Some(c) = &change {
                listener(&space, c);
            }
            Ok(())
        })?
    }

    fn merge(&self, from_space: &str, from: &str, into_space: &str, into: &str, now: i64) -> Result<usize, String> {
        let rows = self.read(from_space, |c| space::export(c, from))?;
        if rows.is_empty() {
            return Ok(0);
        }
        let (dir, into_space, into, from, listener) =
            (self.dir.clone(), into_space.to_string(), into.to_string(), from.to_string(), self.listener.clone());
        self.write(move |w| {
            let conn = w.space(&dir, &into_space)?;
            let tx = conn.unchecked_transaction().map_err(|e| e.to_string())?;
            let changes = space::merge(&*tx, &into, &from, &rows, now)?;
            tx.commit().map_err(|e| e.to_string())?;
            for c in &changes {
                listener(&into_space, c);
            }
            Ok(changes.len())
        })?
    }
}
