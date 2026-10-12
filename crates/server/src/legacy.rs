//! One-time import of the pre-spaces layout (a single database holding every owner, `seq` per owner) into a data
//! directory. A personal space keeps its owner's seqs unchanged, so clients' cached `seq`s stay valid. The old
//! file is only read.

use crate::store::SqliteStore;
use ops::{Change, CommentRow, PaletteRow, Row, TagRow, TaggingRow};
use rusqlite::{Connection, OpenFlags};
use std::path::Path;

type R<T> = Result<T, String>;

#[allow(clippy::needless_pass_by_value)] // used as `map_err(e)`
fn e(e: rusqlite::Error) -> String {
    e.to_string()
}

fn has_table(c: &Connection, name: &str) -> R<bool> {
    c.query_row("SELECT count(*) FROM sqlite_master WHERE type = 'table' AND name = ?1", [name], |r| r.get::<_, i64>(0))
        .map(|n| n > 0)
        .map_err(e)
}

/// Import everything from `old` into `store`. Returns the number of owners imported.
///
/// # Errors
/// If the old database cannot be read or the import fails.
pub fn migrate(old: &Path, store: &SqliteStore) -> R<usize> {
    let c = Connection::open_with_flags(old, OpenFlags::SQLITE_OPEN_READ_ONLY)
        .map_err(|e| format!("{}: {e}", old.display()))?;
    let owners: Vec<(String, i64)> = c
        .prepare("SELECT id, created_at FROM owners ORDER BY id")
        .and_then(|mut s| s.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?.collect())
        .map_err(e)?;
    let sessions: Vec<(String, String, i64)> = c
        .prepare("SELECT token_hash, owner_id, created_at FROM sessions")
        .and_then(|mut s| s.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?.collect())
        .map_err(e)?;
    let comments = has_table(&c, "comments")?;
    for (owner, _) in &owners {
        let mut rows: Vec<Row> = vec![];
        let mut s = c
            .prepare("SELECT id, name, color, created_at, updated_at, deleted_at FROM tags WHERE owner_id = ?1")
            .map_err(e)?;
        for t in s
            .query_map([owner], |r| {
                Ok(Row::Tag(TagRow {
                    id: r.get(0)?,
                    name: r.get(1)?,
                    color: r.get(2)?,
                    created_at: r.get(3)?,
                    updated_at: r.get(4)?,
                    deleted_at: r.get(5)?,
                }))
            })
            .map_err(e)?
        {
            rows.push(t.map_err(e)?);
        }
        let mut s = c
            .prepare(
                "SELECT id, tag_id, text_id, start_ord, end_ord, created_at, deleted_at, merged_into
                 FROM taggings WHERE owner_id = ?1",
            )
            .map_err(e)?;
        for t in s
            .query_map([owner], |r| {
                Ok(Row::Tagging(TaggingRow {
                    id: r.get(0)?,
                    tag: r.get(1)?,
                    text_id: r.get(2)?,
                    start: r.get(3)?,
                    end: r.get(4)?,
                    created_at: r.get(5)?,
                    deleted_at: r.get(6)?,
                    merged_into: r.get(7)?,
                }))
            })
            .map_err(e)?
        {
            rows.push(t.map_err(e)?);
        }
        let palette: Option<(String, i64)> = c
            .query_row("SELECT tags, updated_at FROM palettes WHERE owner_id = ?1", [owner], |r| {
                Ok((r.get(0)?, r.get(1)?))
            })
            .map(Some)
            .or_else(|err| if matches!(err, rusqlite::Error::QueryReturnedNoRows) { Ok(None) } else { Err(err) })
            .map_err(e)?;
        if let Some((tags, updated_at)) = palette {
            let tags = serde_json::from_str(&tags).map_err(|e| e.to_string())?;
            rows.push(Row::Palette(PaletteRow { tags, updated_at }));
        }
        if comments {
            let mut s = c
                .prepare(
                    "SELECT id, tagging_id, body, created_at, edited_at, deleted_at FROM comments WHERE owner_id = ?1",
                )
                .map_err(e)?;
            for t in s
                .query_map([owner], |r| {
                    Ok(Row::Comment(CommentRow {
                        id: r.get(0)?,
                        tagging: r.get(1)?,
                        body: r.get(2)?,
                        created_at: r.get(3)?,
                        edited_at: r.get(4)?,
                        deleted_at: r.get(5)?,
                    }))
                })
                .map_err(e)?
            {
                rows.push(t.map_err(e)?);
            }
        }
        let mut s = c.prepare("SELECT payload FROM changes WHERE owner_id = ?1 ORDER BY seq").map_err(e)?;
        let changes: Vec<Change> = s
            .query_map([owner], |r| r.get::<_, String>(0))
            .map_err(e)?
            .map(|p| serde_json::from_str(&p.map_err(e)?).map_err(|e| e.to_string()))
            .collect::<R<_>>()?;
        if !rows.is_empty() || !changes.is_empty() {
            store.import(owner, owner, &rows, &changes)?;
        }
    }
    let n = owners.len();
    store.import_accounts(owners, sessions)?;
    Ok(n)
}
