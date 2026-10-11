#![allow(dead_code)] // each test binary uses a different subset

use std::fs;
use std::path::PathBuf;

/// A text dir holding text `t1`: one book (id 2) of `words` words in one chapter with one verse, plus a client
/// bundle dir. Returns (static dir, text dir, db path) under a fresh temp dir.
pub fn fixture(name: &str, words: u32) -> (PathBuf, PathBuf, PathBuf) {
    static N: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
    let n = N.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    let root = std::env::temp_dir().join(format!("server-{name}-{}-{n}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    let (dist, text) = (root.join("dist"), root.join("text"));
    fs::create_dir_all(dist.join("assets")).unwrap();
    fs::create_dir_all(text.join("t1")).unwrap();
    fs::write(dist.join("index.html"), "<!doctype html>app").unwrap();
    fs::write(dist.join("assets/app-AbC123.js"), "x".repeat(2000)).unwrap();
    fs::write(text.join("t1/manifest.json"), r#"{"text_id":"t1","index":"index.0123456789abcdef.bin"}"#).unwrap();
    fs::write(text.join("t1/index.0123456789abcdef.bin"), index_v2(words)).unwrap();
    fs::write(text.join("t1/02-GEN.0123456789abcdef.json"), format!("[{}]", "\"word\",".repeat(500) + "0")).unwrap();
    (dist, text, root.join("db.sqlite"))
}

/// BTIX v2 for one book (id 2), one chapter, slots [0 (empty title), 1].
fn index_v2(words: u32) -> Vec<u8> {
    let base = 2u32 << 24;
    let mut b = b"BTIX".to_vec();
    let u32s = |b: &mut Vec<u8>, xs: &[u32]| xs.iter().for_each(|x| b.extend_from_slice(&x.to_le_bytes()));
    u32s(&mut b, &[2, 1, 1, 2, words]); // version, books, chapters, slots, tokens
    u32s(&mut b, &[base, base, base + words]); // verseStart (+ sentinel)
    u32s(&mut b, &[base + words]); // bookEnd
    u32s(&mut b, &[0, 2]); // chapterFirstVerse (+ sentinel)
    b.extend_from_slice(&[0, 0, 0, 0]); // chapterOfVerse: u16 x2
    b.extend_from_slice(&[0, 0, 1, 0]); // bookFirstChapter: u16 x2
    b.push(0); // bookOfChapter
    b.push(2); // bookId
    b
}
