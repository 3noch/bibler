#![allow(dead_code)] // each test binary uses a different subset

use std::fs;
use std::path::PathBuf;

/// A text dir holding text `t1`: one book (id 2) of `words` words in one chapter with one verse (at most 255), plus a client
/// bundle dir. Returns (static dir, text dir, data dir) under a fresh temp dir.
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
    fs::write(text.join("t1/index.0123456789abcdef.bin"), index_v3(words)).unwrap();
    fs::write(text.join("t1/02-GEN.0123456789abcdef.json"), format!("[{}]", "\"word\",".repeat(500) + "0")).unwrap();
    limited_text(&text.join("lim"));
    (dist, text, root.join("data"))
}

/// Text `lim`, limited like the ESV (500 verses or half a book at once): one book (id 2), one chapter, an empty
/// title then 4 verses of 2 words each (`a b`, `c d`, `e f`, `g h`), so at most 2 verses at once.
fn limited_text(dir: &std::path::Path) {
    let v = |verse: u32| 2u32 << 24 | 1 << 16 | verse << 8;
    let mut b = b"BTIX".to_vec();
    let u32s = |b: &mut Vec<u8>, xs: &[u32]| xs.iter().for_each(|x| b.extend_from_slice(&x.to_le_bytes()));
    u32s(&mut b, &[3, 1, 1, 5, 8]); // version, books, chapters, slots, tokens
    u32s(&mut b, &[v(1), v(1), v(2), v(3), v(4), v(4) + 2]); // verseStart (+ sentinel)
    u32s(&mut b, &[v(4) + 2]); // bookEnd
    u32s(&mut b, &[0, 5]); // chapterFirstVerse (+ sentinel)
    b.extend_from_slice(&[0; 10]); // chapterOfVerse: u16 x5
    b.extend_from_slice(&[0, 0, 1, 0]); // bookFirstChapter: u16 x2
    b.push(0); // bookOfChapter
    b.push(2); // bookId
    b.extend_from_slice(&[0, 2, 2, 2, 2]); // slotWords
    fs::create_dir_all(dir).unwrap();
    fs::write(dir.join("index.0123456789abcdef.bin"), b).unwrap();
    let book = serde_json::json!({
        "book": "GEN", "id": 2, "first_ord": v(1), "first_slot": 0, "words": ["a", "b", "c", "d", "e", "f", "g", "h"],
        "italic": [], "divine_name": [], "words_of_jesus": [], "joined": [], "pilcrow": [], "breaks": [],
        "headings": [], "notes": [{"after": v(3), "ref": "1.3", "text": "a note on 1:3"}],
    });
    fs::write(dir.join("02-GEN.0123456789abcdef.json"), book.to_string()).unwrap();
    let manifest = serde_json::json!({
        "text_id": "lim", "index": "index.0123456789abcdef.bin", "limits": {"verses": 500, "book_percent": 50},
        "books": [{"file": "02-GEN.0123456789abcdef.json"}],
    });
    fs::write(dir.join("manifest.json"), manifest.to_string()).unwrap();
}

/// BTIX v3 for one book (id 2), one chapter, slots [0 (empty title), 1 (`words` words)].
fn index_v3(words: u32) -> Vec<u8> {
    let start = 2u32 << 24 | 1 << 16 | 1 << 8; // 1:1, word 0
    let mut b = b"BTIX".to_vec();
    let u32s = |b: &mut Vec<u8>, xs: &[u32]| xs.iter().for_each(|x| b.extend_from_slice(&x.to_le_bytes()));
    u32s(&mut b, &[3, 1, 1, 2, words]); // version, books, chapters, slots, tokens
    u32s(&mut b, &[start, start, start + words]); // verseStart (the empty title starts at 1:1) + sentinel
    u32s(&mut b, &[start + words]); // bookEnd
    u32s(&mut b, &[0, 2]); // chapterFirstVerse (+ sentinel)
    b.extend_from_slice(&[0, 0, 0, 0]); // chapterOfVerse: u16 x2
    b.extend_from_slice(&[0, 0, 1, 0]); // bookFirstChapter: u16 x2
    b.push(0); // bookOfChapter
    b.push(2); // bookId
    b.extend_from_slice(&[0, u8::try_from(words).unwrap()]); // slotWords
    b
}
