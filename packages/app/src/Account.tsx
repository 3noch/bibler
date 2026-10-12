// Account: a top-bar button and its card. Anonymous users keep working as before (their tags sync to this
// device's anonymous owner); signing in attaches them to a Google account so they are safe and reach other devices.
import { Show } from "solid-js";
import type { TagStore } from "./store.ts";
import { CloseIcon, GoogleIcon, UserIcon } from "./icons.tsx";

/** Shown only when the server offers sign-in (or the user is already signed in). */
export function AccountButton(props: { store: TagStore; open: boolean; onToggle: () => void }) {
  const a = () => props.store.account;
  const label = () => (a() ? `Account: ${a()!.email ?? a()!.name ?? "signed in"}` : "Sign in");
  return (
    <Show when={a() || props.store.providers.length > 0}>
      <button class="icon account-btn" classList={{ on: props.open, signed: !!a() }} onClick={props.onToggle} title={label()} aria-label={label()}>
        <Show when={a()} fallback={<UserIcon />}>
          <span class="avatar" aria-hidden="true">{initial(a()!.name ?? a()!.email ?? "?")}</span>
        </Show>
      </button>
    </Show>
  );
}

function initial(s: string): string {
  return [...s.trim()][0]?.toUpperCase() ?? "?";
}

export function AccountCard(props: { store: TagStore; onClose: () => void }) {
  const a = () => props.store.account;
  return (
    <div class="help account" role="dialog" aria-label="Account">
      <button class="icon close" onClick={props.onClose} aria-label="Close">
        <CloseIcon />
      </button>
      <Show
        when={a()}
        fallback={
          <>
            <h2>Keep your tags</h2>
            <p>
              Your tags are saved on this device, but not to an account. Sign in to keep them safe and to use them on your
              other devices. Tags you made here come with you.
            </p>
            <button class="google" onClick={() => props.store.signIn()}>
              <GoogleIcon /> Sign in with Google
            </button>
          </>
        }
      >
        {(acc) => (
          <>
            <div class="who">
              <span class="avatar big" aria-hidden="true">{initial(acc().name ?? acc().email ?? "?")}</span>
              <div>
                <Show when={acc().name}>
                  <strong>{acc().name}</strong>
                </Show>
                <div class="muted">{acc().email ?? "Google account"}</div>
              </div>
            </div>
            <p class="muted">Your tags sync to this account on every device where you sign in.</p>
            <button class="secondary" onClick={() => void props.store.signOut()}>
              Sign out
            </button>
          </>
        )}
      </Show>
    </div>
  );
}
