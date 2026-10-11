use std::fs;
use std::path::PathBuf;
use text_build::{build, default_source, golden, golden_path, index_bytes, Golden, HeadingPos, Text, BOOK_SHIFT};

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
        assert_eq!(b.first_ord, u32::from(b.id) << BOOK_SHIFT, "{}", b.code);
        assert_eq!(t.book_end[i], b.first_ord + b.tokens);
        for ord in b.first_ord..b.first_ord + b.tokens {
            let (book, chapter, verse) = t.locate(ord).unwrap();
            assert_eq!(book, i);
            let (s, e) = t.interval(book, chapter, verse);
            assert!(s <= ord && ord < e, "ord {ord} -> {book} {chapter}:{verse} -> [{s}, {e})");
        }
        // A book's last verse ends at the book's end, not at the next book's first ordinal.
        let last_ch = b.chapters;
        let ch_index = (b.first_chapter + last_ch - 1) as usize;
        let last_v = t.chapter_first_verse[ch_index + 1] - t.chapter_first_verse[ch_index] - 1;
        assert_eq!(t.interval(i, last_ch, last_v).1, t.book_end[i], "{} last verse", b.code);
        // Ordinals in the gap after a book (and before the first) are not words.
        assert_eq!(t.locate(b.first_ord + b.tokens), None);
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
        (s..e).map(|o| t.word(o).unwrap()).collect::<Vec<_>>().join(" ")
    };
    assert_eq!(words("GEN", 1, 1), "In the beginning God created the heaven and the earth.");
    assert_eq!(words("PSA", 3, 0), "A Psalm of David, when he fled from Absalom his son.");
    assert_eq!(words("PSA", 1, 0), "");
    assert_eq!(words("HAB", 3, 1), "A prayer of Habakkuk the prophet upon Shigionoth.");
    assert_eq!(words("JHN", 11, 35), "Jesus wept.");
    assert_eq!(words("REV", 22, 21), "The grace of our Lord Jesus Christ be with you all. Amen.");
    let ids: Vec<u8> = t.books.iter().map(|b| b.id).collect();
    assert_eq!((ids[0], ids[38], ids[39], ids[65]), (2, 40, 70, 96), "GEN, MAL, MAT, REV ids leave 41..=69 free");
    assert_eq!(t.locate(2 << BOOK_SHIFT), Some((0, 1, 1)));
    assert_eq!(t.locate(*t.book_end.last().unwrap() - 1), Some((65, 22, 21)));

    let titles = (1..=150).filter(|&c| !words("PSA", c, 0).is_empty()).count();
    assert_eq!(titles, 116);
    let before = t.headings.iter().filter(|h| h.pos == HeadingPos::Before).count();
    let after = t.headings.iter().filter(|h| h.pos == HeadingPos::After).count();
    assert_eq!((before, after), (22, 14), "Psalm 119 letters and epistle subscriptions");
    assert_eq!(t.pilcrow.len(), 2970);
    assert_eq!(t.notes.len(), 6959);
    let count = |bit: u8| t.flags.iter().filter(|&&f| f & bit != 0).count();
    assert_eq!((count(1), count(2), count(4)), (27063, 6892, 41545), "italic, divine name, words of Jesus");
}

#[test]
fn index_layout_is_exact() {
    let t = kjv();
    let bytes = index_bytes(&t);
    let (nb, nc, ns) = (t.books.len(), t.chapters() as usize, t.slots() as usize);
    assert_eq!(&bytes[..4], b"BTIX");
    assert_eq!(bytes.len(), 24 + 4 * (ns + 1) + 4 * nb + 4 * (nc + 1) + 2 * ns + 2 * (nb + 1) + nc + nb);
    assert_eq!(&bytes[4..8], &2u32.to_le_bytes(), "index version");
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
    let base = 2 << BOOK_SHIFT; // the synthetic book file is 02-TST
    assert_eq!(t.verse_start, [base, base + 2, base + 8, base + 12]);
    assert_eq!(t.book_end, [base + 12]);
    let f = |i: usize| t.flags[i];
    assert_eq!((f(0), f(1)), (0, 1), "title italic");
    assert_eq!((f(4), f(5), f(6), f(7)), (0, 2, 1, 1), "beginning plain, LORD divine name, it was italic");
    assert_eq!((f(8), f(9), f(10)), (4, 4, 0), "words of Jesus; italic punctuation does not flag a plain word");
    assert_eq!(t.pilcrow, [1]);
    assert_eq!(t.breaks, [(1, "p".to_string())], "break attaches to the next slot with tokens");
    assert_eq!(t.notes.len(), 1);
    assert_eq!(
        (t.notes[0].after, t.notes[0].text.as_str(), t.notes[0].reference.as_str()),
        (base + 11, "Or, LORD note", "1.2")
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
fn refuses_input_that_overflows_the_index_widths() {
    // Chapter indexes and the chapter-count sentinel are u16: 65,535 chapters fit exactly; one more must be
    // refused rather than silently wrapped.
    let t = synth(&chapters(65_535)).unwrap();
    assert_eq!(t.book_first_chapter, [0, 65_535]);
    assert_eq!(t.chapter_of_verse.last(), Some(&65_534));
    let err = synth(&chapters(65_536)).unwrap_err();
    assert!(err.contains("too many chapters"), "{err}");
}
