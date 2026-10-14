// Account: shown at the top of the menu. Anonymous users keep working as before (their tags sync to this device's
// anonymous owner); signing in attaches them to a Google account so they are safe and reach other devices.
import { Show } from "solid-js";
import type { TagStore } from "./store.ts";
import { GoogleIcon } from "./icons.tsx";

function initial(s: string): string {
  return [...s.trim()][0]?.toUpperCase() ?? "?";
}

/** Shown only when the server offers sign-in (or the user is already signed in). */
export function AccountSection(props: { store: TagStore }) {
  const a = () => props.store.account;
  return (
    <Show when={a() || props.store.providers.length > 0}>
      <section class="menu-section account">
        <Show
          when={a()}
          fallback={
            <>
              <p class="muted">
                Your tags are saved on this device, but not to an account. Sign in to keep them safe and to use them on
                your other devices. Tags you made here come with you.
              </p>
              <button class="google" onClick={() => props.store.signIn()}>
                <GoogleIcon /> Sign in with Google
              </button>
            </>
          }
        >
          {(acc) => (
            <div class="who">
              <span class="avatar big" aria-hidden="true">{initial(acc().name ?? acc().email ?? "?")}</span>
              <div class="who-text">
                <Show when={acc().name}>
                  <strong>{acc().name}</strong>
                </Show>
                <div class="muted">{acc().email ?? "Google account"}</div>
              </div>
              <button class="link" onClick={() => void props.store.signOut()}>
                Sign out
              </button>
            </div>
          )}
        </Show>
      </section>
    </Show>
  );
}
