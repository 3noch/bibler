// Spaces: your own tags, or a group's (shared with its members; see api::groups on the server). The menu has a row
// for it and the top bar names the open group; the card switches spaces, creates groups and, for the open group,
// shows members, roles, invite links (one per role; a new one revokes the old) and "Leave".
import { createResource, createSignal, For, Show } from "solid-js";
import type { GroupDetail, Member, Role, TagStore } from "./store.ts";
import { CloseIcon, CopyIcon, GroupIcon } from "./icons.tsx";

const ROLE_LABEL: Record<Role, string> = { admin: "Admin", edit: "Can edit", read: "View only" };

/** How to show a member: their name, else email, else "Anonymous". */
export function memberName(m: Member | undefined): string {
  return m?.name ?? m?.email ?? "Anonymous";
}

/** The open group, named in the top bar (nothing in your own space): painting there is seen by its members. */
export function SpaceButton(props: { store: TagStore; open: boolean; onToggle: () => void }) {
  return (
    <Show when={props.store.group()}>
      {(g) => (
        <button class="space-btn group" classList={{ on: props.open }} onClick={props.onToggle} title={`Group: ${g().name}`} aria-label={`Group: ${g().name}`}>
          <GroupIcon />
          <span class="space-name">{g().name}</span>
          <Show when={props.store.readOnly()}>
            <span class="ro-badge">view only</span>
          </Show>
        </button>
      )}
    </Show>
  );
}

export function SpacesCard(props: { store: TagStore; onClose: () => void }) {
  const s = props.store;
  const [name, setName] = createSignal("");
  const create = async () => {
    const n = name().trim();
    if (!n) return;
    const g = await s.groupCall<GroupDetail>("POST", "/groups", { name: n }).catch(() => undefined);
    if (g) s.switchSpace(g.id);
  };
  return (
    <div class="help spaces" role="dialog" aria-label="Spaces">
      <button class="icon close" onClick={props.onClose} aria-label="Close">
        <CloseIcon />
      </button>
      <h2>Spaces</h2>
      <ul class="space-list">
        <li>
          <button class="space-row" classList={{ current: !s.space }} onClick={() => s.space && s.switchSpace(null)}>
            <span>My tags</span>
            <span class="muted">{s.space ? "" : "open"}</span>
          </button>
        </li>
        <For each={s.groups()}>
          {(g) => (
            <li>
              <button class="space-row" classList={{ current: s.space === g.id }} onClick={() => s.space !== g.id && s.switchSpace(g.id)}>
                <span>{g.name}</span>
                <span class="muted">{s.space === g.id ? "open · " : ""}{ROLE_LABEL[g.role]}</span>
              </button>
            </li>
          )}
        </For>
      </ul>
      <form class="new-group" onSubmit={(e) => (e.preventDefault(), void create())}>
        <input placeholder="New group name" value={name()} maxLength={100} onInput={(e) => setName(e.currentTarget.value)} aria-label="New group name" />
        <button type="submit" class="secondary" disabled={!name().trim()}>
          Create group
        </button>
      </form>
      <Show when={s.space}>{(id) => <GroupPanel store={s} id={id()} />}</Show>
    </div>
  );
}

function GroupPanel(props: { store: TagStore; id: string }) {
  const s = props.store;
  const [detail, { mutate, refetch }] = createResource(() => s.groupCall<GroupDetail>("GET", `/groups/${props.id}`).catch(() => undefined));
  const admin = () => detail()?.role === "admin";
  const call = (method: string, path: string, body?: unknown) =>
    s
      .groupCall<GroupDetail>(method, `/groups/${props.id}${path}`, body)
      .then((d) => (d && "members" in d ? mutate(d) : void refetch()))
      .catch(() => undefined);
  const [renaming, setRenaming] = createSignal(false);
  const [copied, setCopied] = createSignal<string | null>(null);
  const link = (token: string) => `${location.origin}${location.pathname}?join=${token}`;
  const copy = (role: string, token: string) =>
    void navigator.clipboard?.writeText(link(token)).then(() => {
      setCopied(role);
      setTimeout(() => setCopied(null), 1500);
    });
  const leave = () => {
    if (!s.me || !confirm("Leave this group? You'll need a new invite link to come back.")) return;
    void s.groupCall("DELETE", `/groups/${props.id}/members/${s.me}`).then(() => s.switchSpace(null), () => undefined);
  };

  return (
    <Show when={detail()}>
      {(d) => (
        <section class="group-panel">
          <Show
            when={renaming()}
            fallback={
              <h3>
                {d().name}
                <Show when={admin()}>
                  <button class="link" onClick={() => setRenaming(true)}>
                    Rename
                  </button>
                </Show>
              </h3>
            }
          >
            <form
              class="new-group"
              onSubmit={(e) => {
                e.preventDefault();
                const n = new FormData(e.currentTarget).get("name");
                void call("PATCH", "", { name: n }).then(() => setRenaming(false));
              }}
            >
              <input name="name" value={d().name} maxLength={100} aria-label="Group name" ref={(el) => queueMicrotask(() => el.select())} />
              <button type="submit" class="secondary">
                Save
              </button>
            </form>
          </Show>
          <p class="muted">
            {d().role === "read" ? "You can view this group's tags and comments as they change." : "Everyone here tags and comments together."}
          </p>
          <h4>Members</h4>
          <ul class="members">
            <For each={d().members}>
              {(m) => (
                <li>
                  <span class="member-name">
                    {memberName(m)}
                    <Show when={m.owner === s.me}> (you)</Show>
                  </span>
                  <Show when={admin() && m.owner !== s.me} fallback={<span class="muted">{ROLE_LABEL[m.role]}</span>}>
                    <select value={m.role} onChange={(e) => void call("PUT", `/members/${m.owner}`, { role: e.currentTarget.value })} aria-label={`Role of ${memberName(m)}`}>
                      <For each={["admin", "edit", "read"] as Role[]}>{(r) => <option value={r}>{ROLE_LABEL[r]}</option>}</For>
                    </select>
                    <button
                      class="icon small"
                      title="Remove from the group"
                      aria-label={`Remove ${memberName(m)}`}
                      onClick={() => confirm(`Remove ${memberName(m)} from the group?`) && void call("DELETE", `/members/${m.owner}`)}
                    >
                      <CloseIcon />
                    </button>
                  </Show>
                </li>
              )}
            </For>
          </ul>
          <Show when={admin()}>
            <h4>Invite links</h4>
            <p class="muted">Anyone with a link can join. Making a new link turns the old one off.</p>
            <For each={["edit", "read"] as const}>
              {(role) => (
                <div class="invite">
                  <span>{ROLE_LABEL[role]}</span>
                  <Show
                    when={d().invites?.[role]}
                    fallback={
                      <button class="link" onClick={() => void call("POST", "/invites", { role })}>
                        Create link
                      </button>
                    }
                  >
                    {(token) => (
                      <>
                        <input readOnly value={link(token())} aria-label={`${ROLE_LABEL[role]} invite link`} onFocus={(e) => e.currentTarget.select()} />
                        <button class="icon small" onClick={() => copy(role, token())} title="Copy link" aria-label="Copy link">
                          <CopyIcon />
                        </button>
                        <span class="muted copied">{copied() === role ? "Copied" : ""}</span>
                        <button class="link" onClick={() => void call("POST", "/invites", { role })} title="Make a new link (the old one stops working)">
                          New link
                        </button>
                      </>
                    )}
                  </Show>
                </div>
              )}
            </For>
          </Show>
          <button class="secondary leave" onClick={leave}>
            Leave group
          </button>
        </section>
      )}
    </Show>
  );
}
