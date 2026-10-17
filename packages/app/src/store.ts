// Synced tag store: a Solid wrapper around SyncState (packages/scripture/src/syncState.ts) plus the WebSocket
// connection. Edits apply optimistically and sync in the background; everything also works offline and catches
// up on reconnect.
//
// Spaces: the user's own tags (personal space) or a group's, shared with its members (see groups.ts). One space is
// open at a time; switching replaces its sync state, caches and socket in place (no reload). Caches are per space
// (the personal space keeps the original keys).
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
import { batch, createMemo, createSignal } from "solid-js";
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
/** The open group (absent: the personal space). */
const SPACE_KEY = "bible-tags/space/v1";
const HIDDEN_KEY = "bible-tags/hidden/v1";
/** The last session and group list from the server, for starting offline (else a group's role is unknown and it
 *  opens view-only, and your own comments aren't recognized as yours). */
const SESSION_KEY = "bible-tags/session/v1";
const GROUPS_KEY = "bible-tags/groups/v1";
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

export type Role = "admin" | "edit" | "read";
export type GroupSummary = { id: string; name: string; role: Role };
export type Member = { owner: string; role: Role; name: string | null; email: string | null };
export type GroupDetail = GroupSummary & { members: Member[]; invites?: { edit?: string; read?: string } };

/** A groups API call (api::groups); throws with the server's error code. */
async function groupsApi<T>(method: string, path: string, body?: unknown): Promise<T> {
  const res = await fetch(`/api${path}`, {
    method,
    credentials: "same-origin",
    headers: body === undefined ? undefined : { "Content-Type": "application/json" },
    body: body === undefined ? undefined : JSON.stringify(body),
  });
  const json = (await res.json().catch(() => ({}))) as T & { error?: string };
  if (!res.ok) throw new Error(json.error ?? `HTTP ${res.status}`);
  return json;
}

const GROUP_ERRORS: Record<string, string> = {
  bad_name: "A group name needs 1–100 characters.",
  bad_invite: "That invite link is no longer valid. Ask for a new one.",
  last_admin: "A group needs another admin first.",
  forbidden: "Only the group's admins can do that.",
};

function storageGet(key: string): string | null {
  try {
    return localStorage.getItem(key);
  } catch {
    return null;
  }
}

function storageSet(key: string, value: string | null) {
  try {
    if (value === null) localStorage.removeItem(key);
    else localStorage.setItem(key, value);
  } catch {
    // storage blocked: the setting lasts for this page only
  }
}

/** `?join=<token>` (an invite link): join that group and open it. Returns a message if it failed. */
async function takeJoin(): Promise<string | undefined> {
  const url = new URL(location.href);
  const token = url.searchParams.get("join");
  if (!token) return undefined;
  url.searchParams.delete("join");
  history.replaceState(history.state, "", url);
  try {
    await fetchSession(); // an anonymous owner is fine
    const g = await groupsApi<GroupDetail>("POST", "/join", { token });
    storageSet(SPACE_KEY, g.id);
    return `You joined “${g.name}”${g.role === "read" ? " (view only)" : ""}.`;
  } catch (e) {
    return GROUP_ERRORS[(e as Error).message] ?? "Couldn't join that group (offline?).";
  }
}

/** One open space: its sync state, caches and connection. Replaced as a whole when switching spaces. */
interface Opened {
  /** The open group, or null for the personal space. */
  space: string | null;
  /** Rows' owner here: the group, or (personal space) this session's owner. */
  owner: string;
  sync: SyncState;
  keyOf: (k: string) => string;
  close: () => void;
}

export async function openTagStore() {
  let session: Session | undefined;
  const joined = LOCAL_ONLY ? undefined : await takeJoin();
  let groups: GroupSummary[] = [];
  let initialSpace: string | null = LOCAL_ONLY ? null : storageGet(SPACE_KEY);
  if (!LOCAL_ONLY) {
    try {
      session = await fetchSession();
      storageSet(SESSION_KEY, JSON.stringify(session));
      groups = (await groupsApi<{ groups: GroupSummary[] }>("GET", "/groups")).groups;
      storageSet(GROUPS_KEY, JSON.stringify(groups));
      if (initialSpace && !groups.some((g) => g.id === initialSpace)) {
        storageSet(SPACE_KEY, (initialSpace = null)); // no longer a member
      }
    } catch {
      // Offline: keep working on local state, with what the server said last time.
      session ??= read<Session>(localStorage, SESSION_KEY);
      groups = read<GroupSummary[]>(localStorage, GROUPS_KEY) ?? [];
    }
  }

  const [version, setVersion] = createSignal(0);
  const [notices, setNotices] = createSignal<{ id: number; text: string }[]>([]);
  let noticeId = 0;
  const notify = (text: string) => {
    const id = ++noticeId;
    setNotices((n) => [...n, { id, text }]);
    setTimeout(() => setNotices((n) => n.filter((x) => x.id !== id)), 6000);
  };
  const [groupList, setGroupList] = createSignal(groups);
  const roleIn = (space: string | null): Role => (space ? (groupList().find((g) => g.id === space)?.role ?? "read") : "admin");
  const [role, setRole] = createSignal<Role>(roleIn(initialSpace));
  const [members, setMembers] = createSignal<Member[]>([]);
  const hiddenKeyOf = (space: string | null) => `${HIDDEN_KEY}:${space ?? "me"}`;
  const loadHidden = (space: string | null) => new Set<string>(JSON.parse(storageGet(hiddenKeyOf(space)) ?? "[]") as string[]);

  let saveTimer: number | undefined;
  /** Set once signing out: nothing may be written back after the device's data is cleared. */
  let signedOut = false;
  /** Write a space's state to this device (base cache, shared outbox, this tab's undo). */
  const save = (o: Opened) =>
    withStorageLock(() => {
      if (signedOut) return;
      const { sync, owner, keyOf } = o;
      try {
        const snap = sync.snapshot();
        const stored = LOCAL_ONLY ? undefined : read<Base>(localStorage, keyOf(BASE_KEY));
        if (!LOCAL_ONLY && (!stored || stored.owner !== owner || stored.seq <= snap.seq)) {
          const { owner: o, seq, rows, changes, confirmed } = snap;
          localStorage.setItem(keyOf(BASE_KEY), JSON.stringify({ owner: o, seq, rows, changes, confirmed } satisfies Base));
        }
        const theirs = read<Outbox>(localStorage, keyOf(OUTBOX_KEY));
        const ops = theirs?.owner === owner ? [...theirs.ops] : [];
        const ids = new Set(ops.map((o) => o.op_id));
        for (const op of snap.pending) if (!ids.has(op.op_id)) ops.push(op);
        const live = ops.filter((o) => !sync.isConfirmed(o.op_id) && !sync.dropped.has(o.op_id));
        localStorage.setItem(keyOf(OUTBOX_KEY), JSON.stringify({ owner, ops: live } satisfies Outbox));
        sessionStorage.setItem(keyOf(UNDO_KEY), JSON.stringify({ owner, undo: snap.undo, redo: snap.redo } satisfies Undo));
      } catch {
        notify("Could not save locally (storage full or blocked); changes still sync while online.");
      }
    });

  /** Open a space from this device's caches and connect it. */
  const open = (space: string | null): Opened => {
    const keyOf = (k: string) => (space ? `${k}:${space}` : k);
    let base: Base | undefined;
    let owner: string;
    if (LOCAL_ONLY) {
      const db = loadLocalDb();
      owner = "local";
      base = { owner, seq: db.seq, rows: db.rows, changes: db.changes, confirmed: db.changes.map((c) => c.op_id) };
    } else {
      base = read<Base>(localStorage, keyOf(BASE_KEY));
      // A group's caches are keyed by the group (its rows belong to it); the personal space's by the owner.
      const id = space ?? session?.owner ?? base?.owner;
      if (!id) throw new Error("first run needs the server once");
      owner = id;
    }
    const undo = read<Undo>(sessionStorage, keyOf(UNDO_KEY));
    const snap: Snapshot | undefined = base && {
      ...base,
      pending: [],
      undo: undo?.owner === owner ? undo.undo : [],
      redo: undo?.owner === owner ? undo.redo : [],
    };
    const sync = new SyncState(owner, ulid, Date.now, snap);
    const outbox = read<Outbox>(localStorage, keyOf(OUTBOX_KEY));
    if (outbox?.owner === owner) sync.adopt(outbox.ops);
    const opened: Opened = { space, owner, sync, keyOf, close: () => {} };
    sync.onNotice(notify);
    sync.onChange(() => {
      setVersion((v) => v + 1);
      clearTimeout(saveTimer);
      saveTimer = window.setTimeout(() => save(opened), 200);
    });
    // Another tab queued ops (e.g. while offline): take them too, so whichever tab is online sends them.
    const onStorage = (e: StorageEvent) => {
      if (e.key !== keyOf(OUTBOX_KEY) || !e.newValue) return;
      const theirs = JSON.parse(e.newValue) as Outbox;
      if (theirs.owner === owner) sync.adopt(theirs.ops);
    };
    window.addEventListener("storage", onStorage);
    const disconnect = LOCAL_ONLY ? (connectLocal(sync, notify), () => {}) : connectForever(sync, space, () => void refreshGroups());
    opened.close = () => {
      disconnect();
      window.removeEventListener("storage", onStorage);
    };
    return opened;
  };

  const [current, setCurrent] = createSignal(open(initialSpace));
  /** The open space's sync state (reactive: switching spaces replaces it). */
  const sync = () => current().sync;
  /** Tags hidden from the text, per space (this device only). */
  const [hidden, setHiddenSet] = createSignal(loadHidden(initialSpace));
  const setHidden = (ids: Iterable<string>) => {
    const next = new Set(ids);
    setHiddenSet(next);
    storageSet(hiddenKeyOf(current().space), next.size ? JSON.stringify([...next]) : null);
  };

  const refreshGroups = async () => {
    if (LOCAL_ONLY) return;
    try {
      const list = (await groupsApi<{ groups: GroupSummary[] }>("GET", "/groups")).groups;
      setGroupList(list);
      storageSet(GROUPS_KEY, JSON.stringify(list));
      const space = current().space;
      if (space) {
        const g = list.find((x) => x.id === space);
        if (!g) {
          notify("You're no longer a member of this group.");
          switchSpace(null);
          return;
        }
        setRole(g.role);
        const detail = await groupsApi<GroupDetail>("GET", `/groups/${space}`);
        if (current().space === space) setMembers(detail.members);
      }
    } catch {
      // offline
    }
  };
  /** Open another space (null: your own tags) in place: the old one is saved and disconnected. `known`: a group
   *  just created or joined, so its role is known before the list is refreshed. */
  const switchSpace = (id: string | null, known?: GroupSummary) => {
    const old = current();
    if (old.space === id) return;
    if (known && !groupList().some((g) => g.id === known.id)) setGroupList((l) => [...l, known]);
    // Open the new space before closing the old one: opening can fail (your own tags have never been loaded on
    // this device and the server can't be reached), and then you stay where you were, still syncing.
    let next: Opened;
    try {
      next = open(id);
    } catch {
      notify("That space hasn't been opened on this device yet; it needs a connection the first time.");
      return;
    }
    clearTimeout(saveTimer);
    save(old);
    old.close();
    storageSet(SPACE_KEY, id);
    batch(() => {
      setRole(roleIn(id));
      setMembers([]);
      setHiddenSet(loadHidden(id));
      setCurrent(next);
      setVersion((v) => v + 1);
    });
    void refreshGroups();
  };

  /** Names of the spaces other than the open one whose outbox (on this device) holds unsent ops. */
  const unsentElsewhere = (): string[] => {
    const open = current().keyOf(OUTBOX_KEY);
    const names: string[] = [];
    for (const k of Object.keys(localStorage)) {
      if (k === open || !(k === OUTBOX_KEY || k.startsWith(`${OUTBOX_KEY}:`))) continue;
      const id = k === OUTBOX_KEY ? null : k.slice(OUTBOX_KEY.length + 1);
      const outbox = read<Outbox>(localStorage, k);
      // A personal outbox left by an earlier owner can never be sent (only its owner's space adopts it).
      if (!outbox?.ops.length || outbox.owner !== (id ?? session?.owner ?? outbox.owner)) continue;
      names.push(id ? `“${groupList().find((g) => g.id === id)?.name ?? "a group"}”` : "My tags");
    }
    return names;
  };

  const authError = takeAuthError();
  if (authError) notify(authError);
  if (joined) notify(joined);
  if (initialSpace) void refreshGroups();

  const view = createMemo(() => (version(), sync().view), undefined, { equals: false });
  const tags = createMemo(() => [...view().tags.values()].filter((t) => t.deleted_at === null));
  // Only this text's taggings: another text's ordinals mean other words.
  const taggings = createMemo(() => [...view().taggings.values()].filter((t) => t.deleted_at === null && t.text_id === TEXT_ID));
  /** What the text shows: taggings of tags that aren't hidden. */
  const visibleTaggings = createMemo(() => {
    const h = hidden();
    return h.size ? taggings().filter((t) => !h.has(t.tag)) : taggings();
  });
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
    read_only: "You can view this group but not change it",
    not_author: "Only its author can change a comment",
  };
  const submit = (intents: OpIntent[]) => {
    if (role() === "read") {
      notify(MESSAGES.read_only!);
      return false;
    }
    const err = sync().submit(intents);
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
      // Unsent edits would be lost: they belong to this device's anonymous owner, and only what reached the server
      // moves to the account. That includes edits queued in spaces that aren't open (they sync when opened).
      const elsewhere = unsentElsewhere();
      if (elsewhere.length) {
        notify(`Some edits in ${elsewhere.join(", ")} haven't been sent yet. Open it while online, then sign in.`);
        return;
      }
      if (sync().pending.length > 0 || sync().status !== "synced") {
        notify("Still syncing your changes. Try again in a moment.");
        return;
      }
      location.href = `/api/auth/google?return=${encodeURIComponent(location.hash)}`;
    },
    /** End the session and forget this account's data on this device; the page restarts anonymous. */
    async signOut() {
      const unsent = sync().pending.length > 0 || unsentElsewhere().length > 0;
      if (unsent && !confirm("Some changes haven't synced yet and will be lost. Sign out anyway?")) return;
      const res = await fetch("/api/logout", { method: "POST", credentials: "same-origin" }).catch(() => undefined);
      if (!res?.ok) {
        notify("Couldn't sign out (offline?). Try again when connected.");
        return;
      }
      // Stop syncing first, so nothing writes this account's data back before the reload.
      current().close();
      signedOut = true;
      clearTimeout(saveTimer);
      withStorageLock(() => {
        const ours = [BASE_KEY, OUTBOX_KEY, SPACE_KEY, SESSION_KEY, GROUPS_KEY, HIDDEN_KEY];
        for (const k of Object.keys(localStorage)) if (ours.some((o) => k === o || k.startsWith(`${o}:`))) localStorage.removeItem(k);
        for (const k of Object.keys(sessionStorage)) if (k.startsWith(UNDO_KEY)) sessionStorage.removeItem(k);
        location.reload();
      });
    },
    /** This session's owner (comment authors are owners); unknown in a group while offline at startup. */
    get me() {
      return session?.owner ?? (current().space ? undefined : current().owner);
    },
    /** The open group's id, or null for your own tags. */
    get space() {
      return current().space;
    },
    /** Your role here ("admin" in your own space). */
    role,
    readOnly: () => role() === "read",
    /** The groups you belong to. */
    groups: groupList,
    /** The open group's members (empty in your own space). */
    members,
    /** The open group, if any. */
    group: () => groupList().find((g) => g.id === current().space),
    switchSpace,
    refreshGroups,
    /** Groups API (api::groups); rejects with a readable message. */
    async groupCall<T>(method: string, path: string, body?: unknown): Promise<T> {
      try {
        const out = await groupsApi<T>(method, path, body);
        void refreshGroups();
        return out;
      } catch (e) {
        const msg = GROUP_ERRORS[(e as Error).message] ?? `Couldn't update the group (${(e as Error).message}).`;
        notify(msg);
        throw new Error(msg);
      }
    },
    hidden,
    setHidden,
    tags,
    taggings,
    visibleTaggings,
    tagById,
    byTag,
    palette,
    commentsByTagging,
    /** The live tagging of `tag` starting at `start` (a single tag's instances are its taggings). */
    taggingAt: (tag: string, start: number) => byTag().get(tag)?.find((t) => t.start === start),
    notices,
    status: () => (version(), sync().status),
    pendingCount: () => (version(), sync().pending.length),
    canUndo: () => (version(), sync().canUndo()),
    canRedo: () => (version(), sync().canRedo()),

    /** Paint intervals (each within one book: `TextIndex.byBook`) as one undo step. */
    paint: (tag: string, ivs: readonly Interval[]) =>
      submit(ivs.map(([start, end]) => ({ kind: "paint" as const, tag, text_id: TEXT_ID, start, end, new_id: ulid(), now: Date.now() }))),
    /** Erase one tag from intervals (each within one book) as one undo step. */
    erase: (tag: string, ivs: readonly Interval[]) =>
      submit(ivs.map(([start, end]) => ({ kind: "erase" as const, tag, text_id: TEXT_ID, start, end, split_id: ulid(), now: Date.now() }))),
    /** Erase every tag over intervals (each within one book) as one undo step. */
    eraseAll(ivs: readonly Interval[]) {
      const now = Date.now();
      return submit(
        ivs.flatMap(([start, end]) => {
          const hit = new Set(visibleTaggings().filter((t) => t.start < end && t.end > start).map((t) => t.tag));
          return [...hit].map((tag) => ({ kind: "erase" as const, tag, text_id: TEXT_ID, start, end, split_id: ulid(), now }));
        }),
      );
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
      return submit(intents) ? sync().view.tags.get(id) : undefined;
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
    undo: () => sync().undoStep(),
    redo: () => sync().redoStep(),
  };
}

/** Keep a WebSocket open: reconnect with backoff, and immediately when the browser comes back online.
 *  `onOpen` runs on each connection (a group refreshes the role, which may have changed). Returns a function that
 *  closes it for good (switching spaces). */
function connectForever(sync: SyncState, space: string | null, onOpen: () => void): () => void {
  let delay = 1000;
  let timer: number | undefined;
  let ws: WebSocket | null = null;
  let closed = false;
  const open = () => {
    clearTimeout(timer);
    if (ws || closed) return;
    const proto = location.protocol === "https:" ? "wss" : "ws";
    const query = space ? `?space=${encodeURIComponent(space)}` : "";
    const sock = new WebSocket(`${proto}://${location.host}/api/sync${query}`);
    ws = sock;
    sock.onopen = () => {
      if (closed) return sock.close();
      sync.connected((m) => sock.send(JSON.stringify(m)));
      onOpen();
    };
    sock.onmessage = (e) => {
      if (closed) return;
      const msg = JSON.parse(e.data as string);
      if (msg.type === "ready") delay = 1000;
      if (!sync.receive(msg)) sock.close();
    };
    sock.onclose = (e) => {
      ws = null;
      if (closed) return;
      if (space && !e.wasClean) onOpen(); // refused? (removed from the group)
      sync.disconnected();
      timer = window.setTimeout(open, delay);
      delay = Math.min(delay * 2, 30_000);
    };
  };
  const online = () => {
    delay = 1000;
    if (!ws) open();
  };
  window.addEventListener("online", online);
  open();
  return () => {
    closed = true;
    clearTimeout(timer);
    window.removeEventListener("online", online);
    ws?.close();
  };
}
