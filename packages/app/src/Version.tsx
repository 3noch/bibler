// The menu's version stamp: the commit the app was built from and, with a server, the server's (they are built and
// deployed separately, so they can differ: e.g. a cached app after a deploy). One line when they agree. The server's
// comes with each sync connection's `ready` (store.serverBuild), so showing it costs no request.
import { Show } from "solid-js";
import type { Build } from "@bible-tags/scripture";

const short = (b: Build) => {
  if (!b.commit) return "unknown";
  const d = new Date(b.committed_at);
  const p = (n: number) => String(n).padStart(2, "0");
  const at = isNaN(d.getTime()) ? "" : ` · ${d.getFullYear()}-${p(d.getMonth() + 1)}-${p(d.getDate())} ${p(d.getHours())}:${p(d.getMinutes())}`;
  return b.commit.slice(0, 7) + at;
};
const full = (b: Build) => `${b.commit || "unknown"} (${b.committed_at || "?"})`;

/** `server`: the server's build (null until a connection said it); undefined without a server (offline-only). */
export function Version(props: { server: Build | null | undefined }) {
  const meta = (name: string) => document.querySelector<HTMLMetaElement>(`meta[name="${name}"]`)?.content ?? "";
  const app: Build = { commit: meta("build-commit"), committed_at: meta("build-committed-at") }; // vite.config.ts
  const same = () => !!app.commit && props.server?.commit === app.commit;
  return (
    <div class="version" title={`App ${full(app)}` + (props.server ? `\nServer ${full(props.server)}` : "")}>
      <Show when={props.server !== undefined && !same()} fallback={<div>Version {short(app)}</div>}>
        <div>App {short(app)}</div>
        <div>Server {props.server ? short(props.server) : "not connected yet"}</div>
      </Show>
    </div>
  );
}
