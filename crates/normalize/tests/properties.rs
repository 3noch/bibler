//! Randomized paint/erase sequences checked against a brute-force coverage model.
//! Same invariants as packages/scripture/test/properties.test.ts.

use normalize::{apply, normalize, Changeset, Intent, Tagging};
use std::collections::HashMap;

/// mulberry32, seeded for reproducibility.
struct Rng(u32);

impl Rng {
    fn next_u32(&mut self) -> u32 {
        self.0 = self.0.wrapping_add(0x6d2b_79f5);
        let mut t = self.0;
        t = (t ^ (t >> 15)).wrapping_mul(t | 1);
        t ^= t.wrapping_add((t ^ (t >> 7)).wrapping_mul(t | 0x3d));
        t ^ (t >> 14)
    }
    fn next(&mut self) -> f64 {
        f64::from(self.next_u32()) / 4_294_967_296.0
    }
    /// Uniform in `0..n` (multiply-shift, no float rounding).
    fn int(&mut self, n: u32) -> u32 {
        u32::try_from((u64::from(self.next_u32()) * u64::from(n)) >> 32).unwrap()
    }
}

const N: u32 = 60;
const TAGS: [&str; 2] = ["a", "b"];

fn track(
    cs: &Changeset,
    comment_owner: &mut HashMap<String, String>,
    tombstones: &mut HashMap<String, Option<String>>,
    age_of: &mut HashMap<String, i64>,
) {
    for t in &cs.create {
        comment_owner.insert(format!("c:{}", t.id), t.id.clone());
        age_of.insert(t.id.clone(), t.created_at);
    }
    for d in &cs.delete {
        tombstones.insert(d.id.clone(), d.merged_into.clone());
    }
    for m in &cs.move_comments {
        for owner in comment_owner.values_mut() {
            if *owner == m.from {
                owner.clone_from(&m.to);
            }
        }
    }
}

fn run_seed(seed: u32, steps: u32) {
    let mut rng = Rng(seed);
    let mut state: Vec<Tagging> = Vec::new();
    let mut model: HashMap<&str, Vec<bool>> = TAGS.iter().map(|t| (*t, vec![false; N as usize])).collect();

    // Every created tagging owns one comment "c:<id>".
    let mut comment_owner: HashMap<String, String> = HashMap::new();
    let mut tombstones: HashMap<String, Option<String>> = HashMap::new();
    let mut age_of: HashMap<String, i64> = HashMap::new();

    for step in 0..steps {
        let tag = TAGS[rng.int(u32::try_from(TAGS.len()).unwrap()) as usize];
        let s = rng.int(N - 1);
        let e = s + 1 + rng.int((N - s).min(12));
        let now = i64::from(step / 3); // deliberate created_at ties
        let is_paint = rng.next() < 0.6;
        let intent = if is_paint {
            Intent::Paint { tag: tag.into(), start: s, end: e, new_id: format!("n{step}"), now }
        } else {
            Intent::Erase { tag: tag.into(), start: s, end: e, split_id: Some(format!("s{step}")) }
        };

        let cs = normalize(&state, &intent).unwrap();

        // Locality: only overlapping/abutting same-tag taggings are needed.
        let local: Vec<Tagging> =
            state.iter().filter(|t| t.tag == tag && t.start <= e && t.end >= s).cloned().collect();
        assert_eq!(normalize(&local, &intent).unwrap(), cs, "locality (seed {seed}, step {step})");

        // Input order is irrelevant: output is canonical.
        let reversed: Vec<Tagging> = local.iter().rev().cloned().collect();
        assert_eq!(normalize(&reversed, &intent).unwrap(), cs, "order independence (seed {seed}, step {step})");

        // Merge survivor is the oldest involved tagging by (created_at, id).
        for d in &cs.delete {
            if let Some(winner) = &d.merged_into {
                assert!(
                    (age_of[winner], winner) < (age_of[&d.id], &d.id),
                    "survivor must be oldest (seed {seed}, step {step})"
                );
            }
        }

        // Erase-split: the right piece inherits the original's created_at.
        if !is_paint {
            for c in &cs.create {
                let src = local.iter().find(|t| t.start < s && t.end > e).expect("split has a source");
                assert_eq!(c.created_at, src.created_at, "split keeps age (seed {seed}, step {step})");
            }
        }

        track(&cs, &mut comment_owner, &mut tombstones, &mut age_of);
        state = apply(&state, &cs);

        let cov = model.get_mut(tag).unwrap();
        for i in s..e {
            cov[i as usize] = is_paint;
        }

        // Idempotence: repeating the intent is a no-op.
        let dup = match &intent {
            Intent::Paint { tag, start, end, now, .. } => {
                Intent::Paint { tag: tag.clone(), start: *start, end: *end, new_id: "dup".into(), now: *now }
            }
            Intent::Erase { tag, start, end, .. } => {
                Intent::Erase { tag: tag.clone(), start: *start, end: *end, split_id: Some("dup".into()) }
            }
        };
        assert_eq!(normalize(&state, &dup).unwrap(), Changeset::default(), "idempotence (seed {seed}, step {step})");

        // Invariants per tag.
        for t in TAGS {
            let mut mine: Vec<&Tagging> = state.iter().filter(|x| x.tag == t).collect();
            mine.sort_by_key(|x| x.start);
            for (i, cur) in mine.iter().enumerate() {
                assert!(cur.start < cur.end, "non-empty");
                if let Some(next) = mine.get(i + 1) {
                    assert!(cur.end < next.start, "disjoint & non-adjacent (seed {seed}, step {step})");
                }
            }
            let mut got = vec![false; N as usize];
            for x in &mine {
                for i in x.start..x.end {
                    got[i as usize] = true;
                }
            }
            assert_eq!(got, model[t], "coverage matches model (seed {seed}, step {step}, tag {t})");
        }

        // Comment conservation.
        for (c, owner) in &comment_owner {
            let live = state.iter().any(|x| &x.id == owner);
            let plain_deleted = matches!(tombstones.get(owner), Some(None));
            assert!(live || plain_deleted, "comment {c} orphaned on {owner} (seed {seed}, step {step})");
        }
    }
}

#[test]
fn randomized_paint_erase_preserves_invariants() {
    for seed in 1..=200 {
        run_seed(seed, 150);
    }
}
