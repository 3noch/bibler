#!/usr/bin/env python3
"""Writes fixtures/ops.json: the behavioral spec for ops.effect (crates/ops, packages/scripture/src/ops.ts).

Every expected value below is written out by hand from the rules (not produced by either implementation).
Regenerate after editing:  python3 fixtures/ops.py
"""
import json
import pathlib

K = "kjv-1769-v1"


def tag(id, name, color="#aa0000", c=1, u=None, d=None):
    return {"table": "tag", "id": id, "name": name, "color": color, "created_at": c,
            "updated_at": c if u is None else u, "deleted_at": d}


def tg(id, t, s, e, c, d=None, m=None, text=K):
    return {"table": "tagging", "id": id, "tag": t, "text_id": text, "start": s, "end": e,
            "created_at": c, "deleted_at": d, "merged_into": m}


def pal(tags, u=1):
    return {"table": "palette", "tags": tags, "updated_at": u}


def rc(before, after):
    return {"before": before, "after": after}


def key(row):
    return (["tag", "tagging", "palette"].index(row["table"]), row.get("id", "palette"))


def apply(rows, change):
    out = {key(r): r for r in rows}
    for r in change["rows"]:
        out[key(r["after"])] = r["after"]
    return sorted(out.values(), key=key)


def replace(rows, *new):
    """rows with each of `new` substituted (by key) or added."""
    return apply(rows, {"rows": [{"after": r} for r in new]})


def ch(seq, op_id, kind, rows):
    return {"seq": seq, "op_id": op_id, "kind": kind, "rows": rows}


A, B, X = tag("A", "Alpha"), tag("B", "Beta"), tag("X", "Gone", d=5)
a1, a2, b1 = tg("a1", "A", 10, 20, 100), tg("a2", "A", 30, 40, 200), tg("b1", "B", 15, 25, 150)
x1 = tg("x1", "X", 50, 60, 50, d=5)
P0 = pal(["A", "B"])
S0 = [A, B, X, a1, a2, b1, x1, P0]

cases = []


def case(name, op, rows=None, error=None, state=S0, changes=()):
    c = {"name": name, "state": {"rows": state, "changes": list(changes)}, "op": op}
    if error:
        c["error"] = error
    else:
        c["rows"] = rows
    cases.append(c)


def paint(t, s, e, new="n1", now=1000, text=K, op="op"):
    return {"op_id": op, "kind": "paint", "tag": t, "text_id": text, "start": s, "end": e, "new_id": new, "now": now}


def erase(t, s, e, split="s1", now=1000, text=K, op="op"):
    o = {"op_id": op, "kind": "erase", "tag": t, "text_id": text, "start": s, "end": e, "now": now}
    if split is not None:
        o["split_id"] = split
    return o


# ---- paint / erase (row-level wrappers around normalize; ids and times from the op)
case("paint on empty text creates a tagging", paint("A", 50, 55),
     [rc(None, tg("n1", "A", 50, 55, 1000))])
case("paint bridging two taggings merges into the older; the other is tombstoned with merged_into",
     paint("A", 18, 32),
     [rc(a1, tg("a1", "A", 10, 40, 100)), rc(a2, tg("a2", "A", 30, 40, 200, d=1000, m="a1"))])
case("paint abutting extends", paint("A", 20, 25), [rc(a1, tg("a1", "A", 10, 25, 100))])
case("paint inside an existing tagging changes nothing", paint("A", 12, 14), [])
case("paint ignores other tags' taggings", paint("B", 30, 35), [rc(None, tg("n1", "B", 30, 35, 1000))])
case("paint in another text_id does not merge across texts", paint("A", 10, 20, text="other-v1"),
     [rc(None, tg("n1", "A", 10, 20, 1000, text="other-v1"))])
case("paint with an unknown tag", paint("Q", 1, 2), error="unknown_tag")
case("paint with a deleted tag", paint("X", 1, 2), error="tag_deleted")
case("paint reusing an existing id (even a deleted one)", paint("A", 70, 80, new="x1"), error="id_exists")
case("paint with an empty interval", paint("A", 5, 5), error="empty_interval")
case("erase in the middle splits; right piece keeps the original created_at", erase("A", 12, 15),
     [rc(a1, tg("a1", "A", 10, 12, 100)), rc(None, tg("s1", "A", 15, 20, 100))])
case("erase covering a tagging tombstones it without merged_into", erase("A", 5, 25),
     [rc(a1, tg("a1", "A", 10, 20, 100, d=1000))])
case("erase that must split but has no split_id", erase("A", 12, 15, split=None), error="missing_split_id")
case("erase with a split_id that already exists", erase("A", 12, 15, split="b1"), error="id_exists")
case("erase over nothing", erase("A", 0, 5), [])

# ---- tags
def create(id, name, color="#00ff00", now=1000):
    return {"op_id": "op", "kind": "tag_create", "id": id, "name": name, "color": color, "now": now}


def update(id, name=None, color=None, now=1000):
    o = {"op_id": "op", "kind": "tag_update", "id": id, "now": now}
    if name is not None:
        o["name"] = name
    if color is not None:
        o["color"] = color
    return o


case("tag_create", create("C", "Gamma"), [rc(None, tag("C", "Gamma", "#00ff00", c=1000))])
case("tag_create trims the name", create("C", "  Gamma  "), [rc(None, tag("C", "Gamma", "#00ff00", c=1000))])
case("tag_create: names are unique case-insensitively among live tags", create("C", " alpha "), error="name_taken")
case("tag_create may reuse a deleted tag's name", create("C", "Gone"), [rc(None, tag("C", "Gone", "#00ff00", c=1000))])
case("tag_create with a blank name", create("C", "   "), error="bad_name")
case("tag_create with a too-long name", create("C", "x" * 101), error="bad_name")
case("tag_create accepts a 100-character name", create("C", "é" * 100), [rc(None, tag("C", "é" * 100, "#00ff00", c=1000))])
case("tag_create counts code points, not UTF-16 units (60 astral chars is fine)", create("C", "\U0001D504" * 60),
     [rc(None, tag("C", "\U0001D504" * 60, "#00ff00", c=1000))])
case("tag_create trims Unicode White_Space like Rust (U+3000), not U+FEFF", create("C", "\u3000Gamma\u3000"),
     [rc(None, tag("C", "Gamma", "#00ff00", c=1000))])
case("tag_create with a bad color", create("C", "Gamma", color="red"), error="bad_color")
case("tag_create reusing a deleted tag's id", create("X", "Gamma"), error="id_exists")
case("tag_update renames and bumps updated_at", update("A", name="Alpha2"),
     [rc(A, tag("A", "Alpha2", c=1, u=1000))])
case("tag_update may change the case of its own name", update("A", name="ALPHA"),
     [rc(A, tag("A", "ALPHA", c=1, u=1000))])
case("tag_update to another live tag's name", update("A", name="beta"), error="name_taken")
case("tag_update recolors", update("B", color="#123abc"), [rc(B, tag("B", "Beta", "#123abc", c=1, u=1000))])
case("tag_update that changes nothing", update("A", name="Alpha", color="#aa0000"), [])
case("tag_update of a deleted tag", update("X", name="Back"), error="tag_deleted")
case("tag_delete tombstones the tag, its live taggings, and removes it from the palette",
     {"op_id": "op", "kind": "tag_delete", "id": "A", "now": 1000},
     [rc(A, tag("A", "Alpha", c=1, u=1000, d=1000)),
      rc(a1, tg("a1", "A", 10, 20, 100, d=1000)),
      rc(a2, tg("a2", "A", 30, 40, 200, d=1000)),
      rc(P0, pal(["B"], 1000))])

# ---- palette
def pset(tags, now=1000):
    return {"op_id": "op", "kind": "palette_set", "tags": tags, "now": now}


case("palette_set reorders", pset(["B", "A"]), [rc(P0, pal(["B", "A"], 1000))])
case("palette_set with no previous palette", pset(["A"]), [rc(None, pal(["A"], 1000))],
     state=[A, B, X, a1, a2, b1, x1])
case("palette_set unchanged", pset(["A", "B"]), [])
case("palette_set with a deleted tag", pset(["A", "X"]), error="tag_deleted")
case("palette_set with a duplicate", pset(["A", "A"]), error="bad_palette")
case("palette_set with more than 9 tags", pset([f"t{i}" for i in range(10)]), error="bad_palette")

# ---- revert (undo / redo)
def revert(target, now=2000, op="op"):
    return {"op_id": op, "kind": "revert", "target": target, "now": now}


merge = ch(1, "m1", "paint", [rc(a1, tg("a1", "A", 10, 40, 100)), rc(a2, tg("a2", "A", 30, 40, 200, d=1000, m="a1"))])
S1 = apply(S0, merge)
case("revert a merge restores both taggings", revert("m1"),
     [rc(tg("a1", "A", 10, 40, 100), a1), rc(tg("a2", "A", 30, 40, 200, d=1000, m="a1"), a2)],
     state=S1, changes=[merge])
case("revert is refused if a row changed since (no clobbering)", revert("m1"), error="conflict",
     state=apply(S1, ch(2, "z", "paint", [rc(tg("a1", "A", 10, 40, 100), tg("a1", "A", 10, 45, 100))])),
     changes=[merge])
case("revert of an unknown op", revert("nope"), error="unknown_op")

created = ch(1, "c1", "paint", [rc(None, tg("n1", "A", 50, 55, 1000))])
case("revert of a creation tombstones the row", revert("c1"),
     [rc(tg("n1", "A", 50, 55, 1000), tg("n1", "A", 50, 55, 1000, d=2000))],
     state=apply(S0, created), changes=[created])

erased = ch(1, "e1", "erase", [rc(a1, tg("a1", "A", 10, 20, 100, d=1000))])
case("revert that would overlap a tagging made since is refused", revert("e1"), error="conflict",
     state=apply(S0, erased) + [tg("z", "A", 12, 14, 1500)], changes=[erased])
case("revert that would abut a tagging made since is refused", revert("e1"), error="conflict",
     state=apply(S0, erased) + [tg("z", "A", 20, 22, 1500)], changes=[erased])
case("revert that would restore a tagging of a tag deleted since is refused", revert("e1"), error="conflict",
     state=replace(apply(S0, erased), tag("A", "Alpha", u=1500, d=1500)), changes=[erased])
case("revert next to a tagging of another tag is fine", revert("e1"),
     [rc(tg("a1", "A", 10, 20, 100, d=1000), a1)],
     state=apply(S0, erased), changes=[erased])

C = tag("C", "Gamma", "#00ff00", c=1000)
made = ch(1, "t1", "tag_create", [rc(None, C)])
case("reverting a tag's creation while it has live taggings is refused", revert("t1"), error="conflict",
     state=S0 + [C, tg("c9", "C", 1, 2, 1100)], changes=[made])
case("reverting a tag's creation while it is in the palette is refused", revert("t1"), error="conflict",
     state=[A, B, X, C, pal(["A", "C"], 1200)], changes=[made])
case("reverting a tag's creation", revert("t1"), [rc(C, tag("C", "Gamma", "#00ff00", c=1000, u=2000, d=2000))],
     state=S0 + [C], changes=[made])
case("reverting a tag's deletion is refused if its name was taken since", revert("t2"), error="name_taken",
     state=[tag("A", "Alpha", d=1000, u=1000), tag("N", "Alpha", c=1500), B, b1],
     changes=[ch(1, "t2", "tag_delete", [rc(A, tag("A", "Alpha", d=1000, u=1000))])])

deleted = ch(1, "d1", "tag_delete", [
    rc(A, tag("A", "Alpha", c=1, u=1000, d=1000)),
    rc(a1, tg("a1", "A", 10, 20, 100, d=1000)),
    rc(a2, tg("a2", "A", 30, 40, 200, d=1000)),
    rc(P0, pal(["B"], 1000))])
case("revert of tag_delete restores the tag, its taggings and the palette", revert("d1"),
     [rc(tag("A", "Alpha", c=1, u=1000, d=1000), A),
      rc(tg("a1", "A", 10, 20, 100, d=1000), a1),
      rc(tg("a2", "A", 30, 40, 200, d=1000), a2),
      rc(pal(["B"], 1000), P0)],
     state=apply(S0, deleted), changes=[deleted])

undo = ch(2, "u1", "revert", [rc(tg("a1", "A", 10, 40, 100), a1), rc(tg("a2", "A", 30, 40, 200, d=1000, m="a1"), a2)])
case("revert of a revert redoes", revert("u1", now=3000),
     [rc(a1, tg("a1", "A", 10, 40, 100)), rc(a2, tg("a2", "A", 30, 40, 200, d=1000, m="a1"))],
     state=apply(S1, undo), changes=[merge, undo])

doc = {
    "_doc": [
        "Behavioral spec for ops.effect, consumed by the Rust (crates/ops) and TS (packages/scripture) suites.",
        "Generated by fixtures/ops.py (expected values written by hand there). Do not edit the JSON directly.",
        "state.rows = the owner's rows (live and deleted); state.changes = recorded changes (revert targets).",
        "Expected `rows` are the change's {before, after} pairs sorted by (table: tag < tagging < palette, id).",
    ],
    "cases": cases,
}
path = pathlib.Path(__file__).with_name("ops.json")
path.write_text(json.dumps(doc, indent=1, ensure_ascii=False) + "\n")
print(f"wrote {len(cases)} cases to {path}")
