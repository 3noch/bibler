// The comment thread of one tagging: shown under the current snippet of a single tag's list (Nearby, tag sheet,
// single-tag search). Comments are ops like any edit: optimistic, synced, undoable. Scripture references in a comment
// show as links (`findRefs`, found when shown; nothing is stored).
import { createContext, createMemo, createSignal, For, Show, useContext } from "solid-js";
import { Dynamic } from "solid-js/web";
import { findRefs, type Interval, type TextIndex } from "@bible-tags/scripture";
import type { Comment, TagStore } from "./store.ts";
import { PencilIcon, TrashIcon } from "./icons.tsx";
import { memberName } from "./Groups.tsx";

const MAX = 10_000;

/** What a reference in a comment links to: the text being read, its address (`href`, so a link can be opened or
 *  copied like any), and what a tap does (`go`: a jump, so Back returns to the comment). Provided by App. */
export const RefLinks = createContext<{ index: TextIndex; href: (iv: Interval) => string; go: (iv: Interval) => void }>();

/** Text with its scripture references as links (a comment, a translator's note, a heading's parallel passages):
 *  a paragraph, or a span when `inline`. */
export function LinkedText(props: { text: string; class: string; inline?: boolean }) {
  const links = useContext(RefLinks);
  const parts = createMemo(() => {
    if (!links) return [props.text];
    const out: (string | { label: string; interval: Interval })[] = [];
    let at = 0;
    for (const r of findRefs(links.index, props.text)) {
      if (r.start > at) out.push(props.text.slice(at, r.start));
      out.push({ label: props.text.slice(r.start, r.end), interval: r.interval });
      at = r.end;
    }
    if (at < props.text.length) out.push(props.text.slice(at));
    return out;
  });
  return (
    <Dynamic component={props.inline ? "span" : "p"} class={props.class}>
      <For each={parts()}>
        {(p) =>
          typeof p === "string" ? (
            p
          ) : (
            <a
              class="ref-link"
              href={links!.href(p.interval)}
              onClick={(e) => {
                if (e.metaKey || e.ctrlKey || e.shiftKey || e.button !== 0) return; // a new tab or window, as usual
                e.preventDefault();
                links!.go(p.interval);
              }}
            >
              {p.label}
            </a>
          )
        }
      </For>
    </Dynamic>
  );
}

function when(ms: number): string {
  const d = new Date(ms);
  const now = new Date();
  if (d.toDateString() === now.toDateString()) return d.toLocaleTimeString(undefined, { hour: "numeric", minute: "2-digit" });
  const opts: Intl.DateTimeFormatOptions = { month: "short", day: "numeric" };
  if (d.getFullYear() !== now.getFullYear()) opts.year = "numeric";
  return d.toLocaleDateString(undefined, opts);
}

/** A textarea that grows with its text; ⌘/Ctrl+Enter submits, Esc cancels. */
function Editor(props: { initial?: string; placeholder: string; submitLabel: string; onSubmit: (body: string) => boolean; onCancel?: () => void; autofocus?: boolean }) {
  const [body, setBody] = createSignal(props.initial ?? "");
  let el!: HTMLTextAreaElement;
  const fit = () => {
    el.style.height = "auto";
    el.style.height = `${el.scrollHeight}px`;
  };
  const submit = () => {
    if (!body().trim()) return;
    if (props.onSubmit(body())) {
      setBody("");
      queueMicrotask(fit);
    }
  };
  return (
    <form class="comment-editor" onSubmit={(e) => (e.preventDefault(), submit())}>
      <textarea
        ref={(t) => {
          el = t;
          queueMicrotask(() => {
            fit();
            if (props.autofocus) {
              t.focus();
              t.setSelectionRange(t.value.length, t.value.length);
            }
          });
        }}
        rows={1}
        maxLength={MAX}
        value={body()}
        placeholder={props.placeholder}
        aria-label={props.placeholder}
        onInput={(e) => (setBody(e.currentTarget.value), fit())}
        onKeyDown={(e) => {
          if (e.key === "Enter" && (e.metaKey || e.ctrlKey)) {
            e.preventDefault();
            submit();
          } else if (e.key === "Escape" && props.onCancel) {
            e.preventDefault();
            e.stopPropagation();
            props.onCancel();
          }
        }}
      />
      <Show when={body().trim() || props.onCancel}>
        <div class="comment-actions">
          <Show when={props.onCancel}>
            <button type="button" class="link" onClick={() => props.onCancel!()}>
              Cancel
            </button>
          </Show>
          <button type="submit" class="primary small" disabled={!body().trim()} title="⌘/Ctrl+Enter">
            {props.submitLabel}
          </button>
        </div>
      </Show>
    </form>
  );
}

function CommentItem(props: { store: TagStore; comment: Comment }) {
  const [editing, setEditing] = createSignal(false);
  /** Unstamped = still optimistic, i.e. ours. */
  const mine = () => !props.comment.author || props.comment.author === props.store.me;
  /** In a group, who wrote it. */
  const author = () =>
    props.store.space ? (mine() ? "You" : memberName(props.store.members().find((m) => m.owner === props.comment.author))) : null;
  return (
    <li class="comment">
      <Show
        when={editing()}
        fallback={
          <>
            <LinkedText class="comment-body" text={props.comment.body} />
            <div class="comment-meta">
              <span>
                <Show when={author()}>
                  <strong class="comment-author">{author()}</strong> ·{" "}
                </Show>
                {when(props.comment.created_at)}
                {props.comment.edited_at !== null ? " · edited" : ""}
              </span>
              <span class="comment-tools" hidden={!mine() || props.store.readOnly()}>
                <button class="icon small" onClick={() => setEditing(true)} aria-label="Edit comment" title="Edit">
                  <PencilIcon />
                </button>
                <button class="icon small" onClick={() => props.store.deleteComment(props.comment.id)} aria-label="Delete comment" title="Delete (undo restores it)">
                  <TrashIcon />
                </button>
              </span>
            </div>
          </>
        }
      >
        <Editor
          initial={props.comment.body}
          placeholder="Edit comment"
          submitLabel="Save"
          autofocus
          onSubmit={(b) => {
            const ok = b.trim() === props.comment.body || props.store.editComment(props.comment.id, b);
            if (ok) setEditing(false);
            return ok;
          }}
          onCancel={() => setEditing(false)}
        />
      </Show>
    </li>
  );
}

export function CommentThread(props: { store: TagStore; tagging: string }) {
  const comments = () => props.store.commentsByTagging().get(props.tagging) ?? [];
  return (
    <div class="comments">
      <Show when={comments().length}>
        <ol class="comment-list">
          <For each={comments()}>{(c) => <CommentItem store={props.store} comment={c} />}</For>
        </ol>
      </Show>
      <Show when={!props.store.readOnly()}>
        <Editor placeholder="Add a comment…" submitLabel="Post" onSubmit={(b) => props.store.addComment(props.tagging, b)} />
      </Show>
    </div>
  );
}
