use std::fs;
use std::path::PathBuf;
use text_build::{build, default_source, golden, golden_path, index_bytes, make_ord, Golden, HeadingPos, Text};

fn kjv() -> Text {
    build(&default_source()).expect("build KJV")
}

#[test]
fn golden_matches_vendored_source() {
    let t = kjv();
    let raw = fs::read_to_string(golden_path()).expect("golden file");
    let want: Golden = serde_json::from_str(&raw).unwrap();
    assert_eq!(golden(&t), want, "tokenization changed: this shifts user ordinals (docs/text-format.md)");
}

#[test]
fn every_ordinal_round_trips() {
    let t = kjv();
    for (i, b) in t.books.iter().enumerate() {
        assert_eq!(b.first_ord, t.ord_of_word(b.first_word), "{}", b.code);
        let last = b.first_word + b.tokens - 1;
        assert_eq!(t.book_end[i], t.ord_of_word(last) + 1, "a book ends right after its last word");
        let mut prev = None;
        for w in b.first_word..=last {
            let ord = t.ord_of_word(w);
            assert_eq!(t.word(ord), Some(t.words[w as usize].as_str()));
            let (book, chapter, verse) = t.locate(ord).unwrap();
            assert_eq!(book, i);
            // The ordinal is the word's address: chapter, verse and position in the verse.
            let (s, e) = t.interval(book, chapter, verse);
            assert_eq!(ord, make_ord(b.id, chapter, verse, ord - s), "{} {chapter}:{verse}", b.code);
            assert!(s <= ord && ord < e, "ord {ord} -> {book} {chapter}:{verse} -> [{s}, {e})");
            // Intervals tile: the previous word's end is this word, and the ordinals between are not words.
            if let Some(p) = prev {
                assert_eq!(t.end_after_word(w - 1), ord);
                assert!((p + 1..ord).all(|o| t.word(o).is_none() && t.locate(o) == t.locate(p)), "gap after {p}");
            }
            prev = Some(ord);
        }
        assert_eq!(t.end_after_word(last), t.book_end[i]);
        // A book's last verse ends at the book's end, not at the next book's first ordinal.
        let last_ch = b.chapters;
        let ch_index = (b.first_chapter + last_ch - 1) as usize;
        let last_v = t.chapter_first_verse[ch_index + 1] - t.chapter_first_verse[ch_index] - 1;
        assert_eq!(t.interval(i, last_ch, last_v).1, t.book_end[i], "{} last verse", b.code);
        // Ordinals in the gap after a book (and before the first) are not words.
        assert_eq!(t.locate(t.book_end[i]), None);
        assert_eq!(t.locate(b.first_ord.wrapping_sub(1)), None);
    }
    assert_eq!(t.locate(0), None);
    assert_eq!(*t.verse_start.last().unwrap(), *t.book_end.last().unwrap());
    // Every verse >= 1 is non-empty; slot 0 is empty unless it is a Psalm title.
    let psa = t.books.iter().position(|b| b.code == "PSA").unwrap();
    for ch in 0..t.chapters() as usize {
        let (first, next) = (t.chapter_first_verse[ch], t.chapter_first_verse[ch + 1]);
        for slot in first..next {
            let len = t.slot_end(slot as usize) - t.verse_start[slot as usize];
            if slot == first {
                assert!(len == 0 || t.book_of_chapter[ch] as usize == psa, "non-Psalm title in chapter {ch}");
            } else {
                assert!(len > 0, "empty verse slot {slot}");
            }
        }
    }
}

#[test]
fn kjv_landmarks() {
    let t = kjv();
    let book = |c: &str| t.books.iter().position(|b| b.code == c).unwrap();
    let words = |b: &str, c: u32, v: u32| {
        let (s, e) = t.interval(book(b), c, v);
        (s..e).filter_map(|o| t.word(o)).collect::<Vec<_>>().join(" ")
    };
    assert_eq!(words("GEN", 1, 1), "In the beginning God created the heaven and the earth.");
    assert_eq!(words("PSA", 3, 0), "A Psalm of David, when he fled from Absalom his son.");
    assert_eq!(words("PSA", 1, 0), "");
    assert_eq!(words("HAB", 3, 1), "A prayer of Habakkuk the prophet upon Shigionoth.");
    assert_eq!(words("JHN", 11, 35), "Jesus wept.");
    assert_eq!(words("REV", 22, 21), "The grace of our Lord Jesus Christ be with you all. Amen.");
    let ids: Vec<u8> = t.books.iter().map(|b| b.id).collect();
    assert_eq!((ids[0], ids[38], ids[39], ids[65]), (2, 40, 70, 96), "GEN, MAL, MAT, REV ids leave 41..=69 free");
    assert_eq!(t.books[0].first_ord, make_ord(2, 1, 1, 0), "Gen 1:1, first word");
    assert_eq!(t.locate(make_ord(2, 1, 1, 0)), Some((0, 1, 1)));
    assert_eq!(t.locate(make_ord(2, 1, 1, 200)), Some((0, 1, 1)), "a gap belongs to the word before it");
    assert_eq!(t.locate(make_ord(2, 1, 0, 0)), None, "before Genesis' first word");
    assert_eq!(t.interval(book("JHN"), 11, 35), (make_ord(73, 11, 35, 0), make_ord(73, 11, 36, 0)));
    let longest = t.slot_words.iter().max().unwrap();
    assert_eq!(*longest, 90, "Esther 8:9: 8 bits per verse leave room");
    assert_eq!(t.locate(*t.book_end.last().unwrap() - 1), Some((65, 22, 21)));

    let titles = (1..=150).filter(|&c| !words("PSA", c, 0).is_empty()).count();
    assert_eq!(titles, 116);
    let before = t.headings.iter().filter(|h| h.pos == HeadingPos::Before).count();
    let after = t.headings.iter().filter(|h| h.pos == HeadingPos::After).count();
    assert_eq!((before, after), (22, 14), "Psalm 119 letters and epistle subscriptions");
    assert_eq!(t.pilcrow.len(), 2970);
    assert_eq!(t.notes.len(), 6959);
    let count = |bit: u8| t.flags.iter().filter(|&&f| f & bit != 0).count();
    assert_eq!(
        (count(1), count(2), count(4)),
        (27066, 6892, 41546),
        "italic, divine name, words of Jesus (hyphen parts counted apart)"
    );
}

#[test]
fn index_layout_is_exact() {
    let t = kjv();
    let bytes = index_bytes(&t);
    let (nb, nc, ns) = (t.books.len(), t.chapters() as usize, t.slots() as usize);
    assert_eq!(&bytes[..4], b"BTIX");
    assert_eq!(bytes.len(), 24 + 4 * (ns + 1) + 4 * nb + 4 * (nc + 1) + 2 * ns + 2 * (nb + 1) + nc + nb + ns);
    assert_eq!(&bytes[4..8], &3u32.to_le_bytes(), "index version");
}

// ---- tokenizer unit tests on synthetic USFM ----

fn synth(body: &str) -> Result<Text, String> {
    let dir = scratch_dir();
    fs::write(dir.join("02-TST.usfm"), format!("\\id TST Test\n\\toc1 Test\n\\toc2 Test\n\\toc3 Tst\n{body}")).unwrap();
    build(&dir)
}

fn scratch_dir() -> PathBuf {
    use std::sync::atomic::{AtomicU32, Ordering};
    static N: AtomicU32 = AtomicU32::new(0);
    let dir = std::env::temp_dir().join(format!(
        "text-build-test-{}-{}",
        std::process::id(),
        N.fetch_add(1, Ordering::SeqCst)
    ));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn markers_spacing_and_flags() {
    let t = synth(concat!(
        "\\c 1\n\\d A \\add title\\add*.\n\\p\n",
        "\\v 1 ¶ In the \\w beginning|strong=\"H7225\"\\w*, \\nd \\+w LORD|strong=\"H3068\"\\+w*\\nd*’s \\add it was\\add*.\n",
        "\\v 2 \\wj Follow me\\wj* said\\add ;\\add* he.\\f + \\fr 1.2 \\ft Or, \\+nd LORD\\+nd* note\\f*\n",
        "\\s1 Subscription.\n",
    ))
    .unwrap();
    assert_eq!(
        t.words,
        ["A", "title.", "In", "the", "beginning,", "LORD’s", "it", "was.", "Follow", "me", "said;", "he."]
    );
    let at = |v, w| make_ord(2, 1, v, w); // the synthetic book file is 02-TST
    assert_eq!(t.verse_start, [at(0, 0), at(1, 0), at(2, 0), at(2, 4)]);
    assert_eq!(t.slot_words, [2, 6, 4]);
    assert_eq!(t.book_end, [at(2, 4)]);
    let f = |i: usize| t.flags[i];
    assert_eq!((f(0), f(1)), (0, 1), "title italic");
    assert_eq!((f(4), f(5), f(6), f(7)), (0, 2, 1, 1), "beginning plain, LORD divine name, it was italic");
    assert_eq!((f(8), f(9), f(10)), (4, 4, 0), "words of Jesus; italic punctuation does not flag a plain word");
    assert_eq!(t.pilcrow, [1]);
    assert_eq!(t.breaks, [(1, "p".to_string())], "break attaches to the next slot with tokens");
    assert_eq!(t.notes.len(), 1);
    assert_eq!(
        (t.notes[0].after, t.notes[0].text.as_str(), t.notes[0].reference.as_str()),
        (at(2, 3), "Or, LORD note", "1.2")
    );
    assert_eq!(t.headings.len(), 1);
    assert_eq!((t.headings[0].slot, t.headings[0].pos), (2, HeadingPos::After));
}

#[test]
fn strictness() {
    let err = |body: &str| synth(body).unwrap_err();
    assert!(err("\\c 1\n\\v 2 skipped one\n").contains("out of sequence"));
    assert!(err("\\c 2\n\\v 1 x\n").contains("chapter 2 out of sequence"));
    assert!(err("\\c 1\n\\v 1 x \\bd bold\\bd*\n").contains("unexpected marker"));
    assert!(err("\\c 1\n\\v 1 x\n\\zz y\n").contains("unexpected line marker"));
    assert!(err("\\c 1\n\\v 1 x ¶ y\n").contains("pilcrow"));
    assert!(err("\\c 1\n\\v 1 x ;\n").contains("punctuation-only"));
    assert!(err("\\c 1\n\\v 1 \\add x\n").contains("unclosed"));
    assert!(err("\\c 1\n\\v 1\n").contains("empty verse"));
    assert!(err("\\c 1\n\\v 1 x- y\n").contains("hyphen before a space"));
    assert!(err("\\c 1\n\\v 1 x y-\n").contains("hyphen at the end"));
    assert!(err("\\c 1\n\\v 1 x -;\n").contains("punctuation-only"));
    assert!(err("\\c 1\n\\v 1 x\n\\d late title\n").contains("precede verse 1"));
}

/// A synthetic book body with `n` one-word chapters.
fn chapters(n: u32) -> String {
    use std::fmt::Write as _;
    (1..=n).fold(String::new(), |mut s, c| {
        let _ = writeln!(s, "\\c {c}\n\\v 1 x");
        s
    })
}

#[test]
fn refuses_input_that_overflows_the_ordinal_fields() {
    // Chapter, verse and word-in-verse are 8 bits each: 255 fit exactly; one more must be refused rather than
    // silently spill into the next field.
    let t = synth(&chapters(255)).unwrap();
    assert_eq!(*t.book_end.last().unwrap(), make_ord(2, 255, 1, 1));
    assert!(synth(&chapters(256)).unwrap_err().contains("more than 255 chapters"));
    let verses = |n: u32| {
        use std::fmt::Write as _;
        (1..=n).fold(String::new(), |mut s, v| {
            let _ = writeln!(s, "\\v {v} x");
            s
        })
    };
    assert!(synth(&format!("\\c 1\n{}", verses(255))).is_ok());
    assert!(synth(&format!("\\c 1\n{}", verses(256))).unwrap_err().contains("over 255"));
    let words = |n: usize| vec!["x"; n].join(" ");
    let t = synth(&format!("\\c 1\n\\v 1 {}\n\\v 2 y\n", words(255))).unwrap();
    assert_eq!(t.verse_start[2], make_ord(2, 1, 2, 0));
    assert_eq!(t.end_after_word(254), make_ord(2, 1, 2, 0), "the last word of a full verse ends at the next verse");
    assert!(synth(&format!("\\c 1\n\\v 1 {}\n", words(256))).unwrap_err().contains("over 255"));
}

#[test]
fn hyphens_split_words() {
    let t = synth("\\c 1\n\\v 1 to Beth-el-\\nd bethel\\nd*, and\n").unwrap();
    assert_eq!(t.words, ["to", "Beth-", "el-", "bethel,", "and"]);
    assert_eq!(t.flags, [0, 8, 8, 2, 0], "joined parts; each part keeps its own flags");
    assert_eq!(t.display_words(&t.books[0]), ["to", "Beth-el-bethel,", "and"]);
    let kjv = kjv();
    let gen = &kjv.books[0];
    let (s, e) = kjv.interval(0, 28, 19);
    let words: Vec<&str> = (s..e).filter_map(|o| kjv.word(o)).collect();
    let i = words.iter().position(|&w| w == "Beth-").unwrap();
    assert_eq!(&words[i - 1..i + 2], ["place", "Beth-", "el:"]);
    assert!(kjv.display_words(gen).iter().any(|w| w == "Beth-el:"));
}

#[test]
fn flag_runs_cross_verses() {
    let t = synth("\\c 1\n\\v 1 a \\wj b\\wj*\n\\v 2 \\wj c\\wj* d\n").unwrap();
    assert_eq!(t.flags, [0, 4, 4, 0]);
    let dir = scratch_dir();
    text_build::write_artifacts(&t, &dir).unwrap();
    let file = fs::read_dir(&dir)
        .unwrap()
        .map(|e| e.unwrap().path())
        .find(|p| p.to_string_lossy().contains("02-TST"))
        .unwrap();
    let json: serde_json::Value = serde_json::from_str(&fs::read_to_string(file).unwrap()).unwrap();
    // One run from "b" (1:1, word 1) to "d" (1:2, word 1): ends point at the next word, so runs merge across verses.
    assert_eq!(json["words_of_jesus"], serde_json::json!([[make_ord(2, 1, 1, 1), make_ord(2, 1, 2, 1)]]));
}
