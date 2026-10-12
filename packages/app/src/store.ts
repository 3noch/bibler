// Synced tag store: a Solid wrapper around SyncState (packages/scripture/src/syncState.ts) plus the WebSocket
// connection. Edits apply optimistically and sync in the background; everything also works offline and catches
// up on reconnect.
//
// Persistence (several tabs share one origin):
//   localStorage  BASE_KEY    confirmed rows at a seq (a cache; any tab may write it, never to a lower seq)
//   localStorage  OUTBOX_KEY  unconfirmed ops of every tab (union; an op leaves only once confirmed or dropped),
//                             updated under a Web Lock. Any tab may (re)send any op: ops are idempotent.
//   sessionStorage UNDO_KEY   this tab's undo/redo history
//
// Offline-only builds (VITE_SYNC=local, e.g. GitHub Pages): no server, no account. `localServer.ts` stands in for the
// sync server inside the browser (same protocol, same canonical `effect`) and owns the confirmed state, so BASE_KEY
// is not written. Data then lives only in this browser.
import { createMemo, createSignal } from "solid-js";
import { resolveTagging, SyncState, type CommentRow, type Interval, type Op, type OpIntent, type Snapshot, type TagRow, type TaggingRow } from "@bible-tags/scripture";
import { TEXT_ID } from "./text.ts";
import { connectLocal, loadLocalDb } from "./localServer.ts";
import { ulid } from "./ulid.ts";

export type Tag = TagRow;
export type Comment = CommentRow;

const intervals = new WeakMap<TaggingRow, Interval>();
/** A tagging's [start, end), the same array for the same row object. Unchanged rows keep their objects across
 *  edits, so lists built from these keep their identity and keyed UI (Solid `<For>`) doesn't remount rows (which
 *  would, e.g., drop focus from a comment box while posting). */
export function intervalOf(t: TaggingRow): Interval {
  let iv = intervals.get(t);
  if (!iv) intervals.set(t, (iv = [t.start, t.end]));
  return iv;
}

/** Built without a server (offline-only): see localServer.ts. */
export const LOCAL_ONLY = import.meta.env.VITE_SYNC === "local";

const BASE_KEY = "bible-tags/base/v1";
const OUTBOX_KEY = "bible-tags/outbox/v1";
const UNDO_KEY = "bible-tags/undo/v1";
const LEGACY_KEY = "bible-tags/local/v1"; // the local-only prototype's data, imported once
const COLORS = ["#e6a23c", "#4fa3e0", "#6cc070", "#e06c9f", "#9b7be0", "#e0c84f", "#4fc9c0", "#e07b4f", "#8fa0b0"];

type Base = Pick<Snapshot, "owner" | "seq" | "rows" | "changes" | "confirmed">;
type Outbox = { owner: string; ops: Op[] };
type Undo = Pick<Snapshot, "owner" | "undo" | "redo">;

function read<T>(storage: Storage, key: string): T | undefined {
  try {
    const raw = storage.getItem(key);
    return raw ? (JSON.parse(raw) as T) : undefined;
  } catch {
    return undefined;
  }
}

/** Run `f` exclusively across tabs (Web Locks), or directly where unsupported. */
function withStorageLock(f: () => void): void {
  if (navigator.locks) void navigator.locks.request("bible-tags-storage", async () => f());
  else f();
}

/** A signed-in account (`null` while anonymous). */
export type Account = { provider: string; email: string | null; name: string | null };
type Session = { owner: string; account: Account | null; providers: string[] };

async function fetchSession(): Promise<Session> {
  const res = await fetch("/api/session", { method: "POST", credentials: "same-origin" });
  if (!res.ok) throw new Error(`session: HTTP ${res.status}`);
  const s = (await res.json()) as Partial<Session> & { owner: string };
  return { owner: s.owner, account: s.account ?? null, providers: s.providers ?? [] };
}

const AUTH_ERRORS: Record<string, string> = {
  cancelled: "Sign-in was cancelled.",
  expired: "Sign-in took too long or was interrupted. Please try again.",
};

/** The sign-in callback reports failures as `?auth_error=code`: take it out of the URL and return a message. */
function takeAuthError(): string | undefined {
  const url = new URL(location.href);
  const code = url.searchParams.get("auth_error");
  if (!code) return undefined;
  url.searchParams.delete("auth_error");
  history.replaceState(history.state, "", url);
  return AUTH_ERRORS[code] ?? "Sign-in failed. Please try again.";
}

export type TagStore = Awaited<ReturnType<typeof openTagStore>>;

export async function openTagStore() {
  let base: Base | undefined;
  let owner: string;
  let session: Session | undefined;
  if (LOCAL_ONLY) {
    const db = loadLocalDb();
    owner = "local";
    base = { owner, seq: db.seq, rows: db.rows, changes: db.changes, confirmed: db.changes.map((c) => c.op_id) };
  } else {
    base = read<Base>(localStorage, BASE_KEY);
    try {
      session = await fetchSession();
      owner = session.owner;
    } catch (e) {
      if (!base) throw e; // first run needs the server once
      owner = base.owner; // offline: keep working on local state
    }
  }
  const undo = read<Undo>(sessionStorage, UNDO_KEY);
  const snap: Snapshot | undefined = base && {
    ...base,
    pending: [],
    undo: undo?.owner === owner ? undo.undo : [],
    redo: undo?.owner === owner ? undo.redo : [],
  };
  const sync = new SyncState(owner, ulid, Date.now, snap);
  const outbox = read<Outbox>(localStorage, OUTBOX_KEY);
  if (outbox?.owner === owner) sync.adopt(outbox.ops);

  const [version, setVersion] = createSignal(0);
  const [notices, setNotices] = createSignal<{ id: number; text: string }[]>([]);
  let noticeId = 0;
  const notify = (text: string) => {
    const id = ++noticeId;
    setNotices((n) => [...n, { id, text }]);
    setTimeout(() => setNotices((n) => n.filter((x) => x.id !== id)), 6000);
  };
  sync.onNotice(notify);
  const authError = takeAuthError();
  if (authError) notify(authError);

  let saveTimer: number | undefined;
  const save = () =>
    withStorageLock(() => {
      try {
        const snap = sync.snapshot();
        const stored = LOCAL_ONLY ? undefined : read<Base>(localStorage, BASE_KEY);
        if (!LOCAL_ONLY && (!stored || stored.owner !== owner || stored.seq <= snap.seq)) {
          const { owner: o, seq, rows, changes, confirmed } = snap;
          localStorage.setItem(BASE_KEY, JSON.stringify({ owner: o, seq, rows, changes, confirmed } satisfies Base));
        }
        const theirs = read<Outbox>(localStorage, OUTBOX_KEY);
        const ops = theirs?.owner === owner ? [...theirs.ops] : [];
        const ids = new Set(ops.map((o) => o.op_id));
        for (const op of snap.pending) if (!ids.has(op.op_id)) ops.push(op);
        const live = ops.filter((o) => !sync.isConfirmed(o.op_id) && !sync.dropped.has(o.op_id));
        localStorage.setItem(OUTBOX_KEY, JSON.stringify({ owner, ops: live } satisfies Outbox));
        sessionStorage.setItem(UNDO_KEY, JSON.stringify({ owner, undo: snap.undo, redo: snap.redo } satisfies Undo));
      } catch {
        notify("Could not save locally (storage full or blocked); changes still sync while online.");
      }
    });
  sync.onChange(() => {
    setVersion((v) => v + 1);
    clearTimeout(saveTimer);
    saveTimer = window.setTimeout(save, 200);
  });
  // Another tab queued ops (e.g. while offline): take them too, so whichever tab is online sends them.
  window.addEventListener("storage", (e) => {
    if (e.key !== OUTBOX_KEY || !e.newValue) return;
    const theirs = JSON.parse(e.newValue) as Outbox;
    if (theirs.owner === owner) sync.adopt(theirs.ops);
  });

  importLegacy(sync);
  if (LOCAL_ONLY) connectLocal(sync, notify);
  else connectForever(sync);

  const view = createMemo(() => (version(), sync.view), undefined, { equals: false });
  const tags = createMemo(() => [...view().tags.values()].filter((t) => t.deleted_at === null));
  const taggings = createMemo(() => [...view().taggings.values()].filter((t) => t.deleted_at === null));
  const tagById = createMemo(() => new Map(tags().map((t) => [t.id, t])));
  const byTag = createMemo(() => {
    const m = new Map<string, TaggingRow[]>();
    for (const t of taggings()) m.set(t.tag, [...(m.get(t.tag) ?? []), t]);
    for (const list of m.values()) list.sort((a, b) => a.start - b.start);
    return m;
  });
  const palette = createMemo(() => (view().pal?.tags ?? []).map((id) => tagById().get(id)).filter((t): t is Tag => !!t));
  /** Live comments by the live tagging they show on (following merges), oldest first. */
  const commentsByTagging = createMemo(() => {
    const v = view();
    const m = new Map<string, Comment[]>();
    for (const c of v.comments.values()) {
      if (c.deleted_at !== null) continue;
      let on: string;
      try {
        on = resolveTagging(v, c.tagging).id;
      } catch {
        continue; // on a deleted tagging: hidden until that deletion is undone
      }
      m.set(on, [...(m.get(on) ?? []), c]);
    }
    for (const list of m.values()) list.sort((a, b) => a.created_at - b.created_at || (a.id < b.id ? -1 : 1));
    return m;
  });

  const MESSAGES: Record<string, string> = {
    name_taken: "A tag with that name already exists",
    bad_body: "A comment can't be empty or longer than 10,000 characters",
    tagging_deleted: "That highlight was removed",
    comment_deleted: "That comment was deleted",
  };
  const submit = (intents: OpIntent[]) => {
    const err = sync.submit(intents);
    if (err) notify(MESSAGES[err.code] ?? `Not applied: ${err.code}`);
    return !err;
  };

  return {
    mode: LOCAL_ONLY ? ("local" as const) : ("server" as const),
    /** The signed-in account, `null` while anonymous, `undefined` if unknown (offline at startup). */
    account: session?.account,
    /** Configured sign-in methods (empty: sign-in unavailable). */
    providers: session?.providers ?? [],
    /** Off to Google; back on this page (same location) once signed in. This device's tags go with it: they
     *  become the account's, or are added to an existing account's. Refused while edits are still unsent. */
    signIn() {
      if (sync.pending.length > 0 || sync.status !== "synced") {
        notify("Still syncing your changes. Try again in a moment.");
        return;
      }
      location.href = `/api/auth/google?return=${encodeURIComponent(location.hash)}`;
    },
    /** End the session and forget this account's data on this device; the page restarts anonymous. */
    async signOut() {
      if (sync.pending.length > 0 && !confirm("Some changes haven't synced yet and will be lost. Sign out anyway?")) return;
      const res = await fetch("/api/logout", { method: "POST", credentials: "same-origin" }).catch(() => undefined);
      if (!res?.ok) {
        notify("Couldn't sign out (offline?). Try again when connected.");
        return;
      }
      clearTimeout(saveTimer);
      withStorageLock(() => {
        for (const k of [BASE_KEY, OUTBOX_KEY]) localStorage.removeItem(k);
        sessionStorage.removeItem(UNDO_KEY);
        location.reload();
      });
    },
    tags,
    taggings,
    tagById,
    byTag,
    palette,
    commentsByTagging,
    /** The live tagging of `tag` starting at `start` (a single tag's instances are its taggings). */
    taggingAt: (tag: string, start: number) => byTag().get(tag)?.find((t) => t.start === start),
    notices,
    status: () => (version(), sync.status),
    pendingCount: () => (version(), sync.pending.length),
    canUndo: () => (version(), sync.canUndo()),
    canRedo: () => (version(), sync.canRedo()),

    paint: (tag: string, start: number, end: number) =>
      submit([{ kind: "paint", tag, text_id: TEXT_ID, start, end, new_id: ulid(), now: Date.now() }]),
    erase: (tag: string, start: number, end: number) =>
      submit([{ kind: "erase", tag, text_id: TEXT_ID, start, end, split_id: ulid(), now: Date.now() }]),
    /** Erase every tag over [start, end) as one undo step. */
    eraseAll(start: number, end: number) {
      const hit = new Set(taggings().filter((t) => t.start < end && t.end > start).map((t) => t.tag));
      const now = Date.now();
      return submit([...hit].map((tag) => ({ kind: "erase" as const, tag, text_id: TEXT_ID, start, end, split_id: ulid(), now })));
    },
    /** Create a tag (and add it to the palette if there is room) as one undo step. */
    createTag(name: string): Tag | undefined {
      const used = new Set(tags().map((t) => t.color));
      const color = COLORS.find((c) => !used.has(c)) ?? COLORS[tags().length % COLORS.length]!;
      const id = ulid();
      const now = Date.now();
      const pal = view().pal?.tags ?? [];
      const intents: OpIntent[] = [{ kind: "tag_create", id, name, color, now }];
      if (pal.length < 9) intents.push({ kind: "palette_set", tags: [...pal, id], now });
      return submit(intents) ? sync.view.tags.get(id) : undefined;
    },
    updateTag: (id: string, patch: { name?: string; color?: string }) =>
      submit([{ kind: "tag_update", id, name: patch.name ?? null, color: patch.color ?? null, now: Date.now() }]),
    deleteTag: (id: string) => submit([{ kind: "tag_delete", id, now: Date.now() }]),
    /** Palette tag ids, in order (at most MAX_PALETTE). */
    paletteIds: () => view().pal?.tags ?? [],
    setPalette: (tags: string[]) => submit([{ kind: "palette_set", tags, now: Date.now() }]),
    addComment: (tagging: string, body: string) =>
      submit([{ kind: "comment_create", id: ulid(), tagging, body, now: Date.now() }]),
    editComment: (id: string, body: string) => submit([{ kind: "comment_update", id, body, now: Date.now() }]),
    deleteComment: (id: string) => submit([{ kind: "comment_delete", id, now: Date.now() }]),
    undo: () => sync.undoStep(),
    redo: () => sync.redoStep(),
  };
}

/** Keep a WebSocket open: reconnect with backoff, and immediately when the browser comes back online. */
function connectForever(sync: SyncState) {
  let delay = 1000;
  let timer: number | undefined;
  let ws: WebSocket | null = null;
  const open = () => {
    clearTimeout(timer);
    if (ws) return;
    const proto = location.protocol === "https:" ? "wss" : "ws";
    const sock = new WebSocket(`${proto}://${location.host}/api/sync`);
    ws = sock;
    sock.onopen = () => sync.connected((m) => sock.send(JSON.stringify(m)));
    sock.onmessage = (e) => {
      const msg = JSON.parse(e.data as string);
      if (msg.type === "ready") delay = 1000;
      if (!sync.receive(msg)) sock.close();
    };
    sock.onclose = () => {
      ws = null;
      sync.disconnected();
      timer = window.setTimeout(open, delay);
      delay = Math.min(delay * 2, 30_000);
    };
  };
  window.addEventListener("online", () => {
    delay = 1000;
    if (!ws) open();
  });
  open();
}

/** The local-only prototype stored tags in localStorage; replay them as ops once, then drop the old key. */
function importLegacy(sync: SyncState) {
  let raw: string | null = null;
  try {
    raw = localStorage.getItem(LEGACY_KEY);
  } catch {
    return;
  }
  if (!raw || sync.view.tags.size > 0) return;
  try {
    const old = JSON.parse(raw) as {
      tags: { id: string; name: string; color: string; created_at: number }[];
      taggings: { id: string; tag: string; start: number; end: number; created_at: number }[];
      palette: string[];
    };
    const intents: OpIntent[] = [
      ...old.tags.map((t) => ({ kind: "tag_create" as const, id: t.id, name: t.name, color: t.color, now: t.created_at })),
      ...old.taggings.map((t) => ({ kind: "paint" as const, tag: t.tag, text_id: TEXT_ID, start: t.start, end: t.end, new_id: t.id, now: t.created_at })),
      { kind: "palette_set" as const, tags: old.palette.slice(0, 9), now: Date.now() },
    ];
    if (!sync.submit(intents)) localStorage.removeItem(LEGACY_KEY);
  } catch {
    // Unreadable legacy data: leave it alone.
  }
}
