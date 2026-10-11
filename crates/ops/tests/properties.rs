//! Random op sequences against the in-memory reference store.
mod common;

use common::Mem;
use ops::{effect, Intent, Op, Row, Table, TaggingRow};
use std::collections::{HashMap, HashSet};

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        // xorshift64*
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_f491_4f6c_dd1d)
    }
    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }
    fn pick<'a, T>(&mut self, xs: &'a [T]) -> &'a T {
        &xs[usize::try_from(self.below(xs.len() as u64)).unwrap()]
    }
}

const TEXTS: [&str; 2] = ["kjv-1769-v1", "other-v1"];
const N: u64 = 60;

fn live(rows: &[Row]) -> Vec<Row> {
    let mut out: Vec<Row> = rows
        .iter()
        .filter(|r| match r {
            Row::Tag(t) => t.deleted_at.is_none(),
            Row::Tagging(t) => t.deleted_at.is_none(),
            Row::Palette(_) => true,
        })
        .cloned()
        .collect();
    out.sort_by(|a, b| a.key().cmp(&b.key()));
    out
}

fn check_invariants(mem: &Mem, ctx: &str) {
    let tags: HashMap<&str, &ops::TagRow> = mem
        .rows
        .iter()
        .filter_map(|r| match r {
            Row::Tag(t) => Some((t.id.as_str(), t)),
            _ => None,
        })
        .collect();
    let mut by_tag: HashMap<(&str, &str), Vec<&TaggingRow>> = HashMap::new();
    for r in &mem.rows {
        if let Row::Tagging(t) = r {
            if t.deleted_at.is_none() {
                assert!(tags[t.tag.as_str()].deleted_at.is_none(), "{ctx}: live tagging {} of deleted tag", t.id);
                assert!(t.start < t.end, "{ctx}: empty tagging");
                by_tag.entry((t.tag.as_str(), t.text_id.as_str())).or_default().push(t);
            }
        }
    }
    for list in by_tag.values_mut() {
        list.sort_by_key(|t| t.start);
        for w in list.windows(2) {
            assert!(w[0].end < w[1].start, "{ctx}: {} and {} overlap or abut", w[0].id, w[1].id);
        }
    }
    let mut names = HashSet::new();
    for t in tags.values().filter(|t| t.deleted_at.is_none()) {
        assert!(names.insert(t.name.to_lowercase()), "{ctx}: duplicate live name {}", t.name);
    }
    if let Some(Row::Palette(p)) = mem.row((Table::Palette, ops::PALETTE_ID)) {
        for id in &p.tags {
            assert!(tags.get(id.as_str()).is_some_and(|t| t.deleted_at.is_none()), "{ctx}: palette has dead tag {id}");
        }
    }
}

fn random_op(rng: &mut Rng, mem: &Mem, step: usize, own: &[String]) -> Op {
    let now = i64::try_from(step).unwrap() * 10;
    let tag_ids: Vec<String> = mem
        .rows
        .iter()
        .filter_map(|r| match r {
            Row::Tag(t) => Some(t.id.clone()),
            _ => None,
        })
        .collect();
    let intent = match rng.below(20) {
        0..=1 => Intent::TagCreate {
            id: format!("t{step}"),
            name: format!("name{}", rng.below(6)),
            color: "#336699".into(),
            now,
        },
        2 if !tag_ids.is_empty() => Intent::TagDelete { id: rng.pick(&tag_ids).clone(), now },
        3 if !tag_ids.is_empty() => Intent::TagUpdate {
            id: rng.pick(&tag_ids).clone(),
            name: Some(format!("name{}", rng.below(6))),
            color: None,
            now,
        },
        4 if !tag_ids.is_empty() => {
            let n = rng.below(3);
            Intent::PaletteSet { tags: (0..n).map(|_| rng.pick(&tag_ids).clone()).collect(), now }
        }
        5..=7 if !own.is_empty() => Intent::Revert { target: rng.pick(own).clone(), now },
        _ if !tag_ids.is_empty() => {
            let tag = rng.pick(&tag_ids).clone();
            let text_id = (*rng.pick(&TEXTS)).to_string();
            let start = u32::try_from(rng.below(N - 1)).unwrap();
            let end = start + 1 + u32::try_from(rng.below((N - u64::from(start)).min(10))).unwrap();
            if rng.below(10) < 6 {
                Intent::Paint { tag, text_id, start, end, new_id: format!("p{step}"), now }
            } else {
                Intent::Erase { tag, text_id, start, end, split_id: Some(format!("s{step}")), now }
            }
        }
        _ => Intent::TagCreate {
            id: format!("t{step}"),
            name: format!("name{}", rng.below(6)),
            color: "#336699".into(),
            now,
        },
    };
    Op { op_id: format!("op{step}"), intent }
}

fn run(seed: u64, steps: usize) {
    let mut rng = Rng(seed.wrapping_mul(0x9e37_79b9_7f4a_7c15) | 1);
    let mut mem = Mem::default();
    let start = live(&mem.rows);
    let mut applied: Vec<String> = vec![];
    for step in 0..steps {
        let op = random_op(&mut rng, &mem, step, &applied);
        let ctx = format!("seed {seed} step {step} {:?}", op.intent);
        let Ok(change) = effect(&mem, &op) else { continue };
        for rc in &change.rows {
            assert_eq!(rc.before.as_ref(), mem.row(rc.after.key()), "{ctx}: `before` must be the prior row");
        }
        mem.apply(change);
        applied.push(op.op_id);
        check_invariants(&mem, &ctx);
    }
    // Undo everything, newest first: with no other writer, every revert must succeed.
    for (k, target) in applied.iter().rev().enumerate() {
        let op = Op { op_id: format!("undo{k}"), intent: Intent::Revert { target: target.clone(), now: 1_000_000 } };
        let change = effect(&mem, &op).unwrap_or_else(|e| panic!("seed {seed}: undo of {target} failed: {e}"));
        mem.apply(change);
        check_invariants(&mem, &format!("seed {seed} undo {target}"));
    }
    let mut end = live(&mem.rows);
    end.retain(|r| !matches!(r, Row::Palette(p) if p.tags.is_empty()));
    assert_eq!(end, start, "seed {seed}: undoing everything must restore the starting state");
}

#[test]
fn random_ops_keep_invariants_and_undo_all_restores() {
    for seed in 1..=300 {
        run(seed, 120);
    }
}
