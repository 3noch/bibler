// The menu's version stamp: the commit the app was built from and, with a server, the server's (they are built and
// deployed separately, so they can differ: e.g. a cached app after a deploy). One line when they agree.
import { createResource, Show } from "solid-js";

interface Build {
  commit: string;
  committedAt: string;
}

const short = (b: Build) => {
  if (!b.commit) return "unknown";
  const d = new Date(b.committedAt);
  const p = (n: number) => String(n).padStart(2, "0");
  const at = isNaN(d.getTime()) ? "" : ` · ${d.getFullYear()}-${p(d.getMonth() + 1)}-${p(d.getDate())} ${p(d.getHours())}:${p(d.getMinutes())}`;
  return b.commit.slice(0, 7) + at;
};

async function serverBuild(): Promise<Build | null> {
  try {
    const r = await fetch("/api/health", { cache: "no-store" });
    const j = (await r.json()) as { commit?: string; committed_at?: string };
    return { commit: j.commit ?? "", committedAt: j.committed_at ?? "" };
  } catch {
    return null;
  }
}

export function Version(props: { server: boolean }) {
  const meta = (name: string) => document.querySelector<HTMLMetaElement>(`meta[name="${name}"]`)?.content ?? "";
  const app: Build = { commit: meta("build-commit"), committedAt: meta("build-committed-at") }; // vite.config.ts
  const [server] = createResource(() => props.server || undefined, serverBuild);
  const same = () => !server.loading && server()?.commit === app.commit && !!app.commit;
  return (
    <div class="version" title={`App ${app.commit || "unknown"} (${app.committedAt || "?"})` + (server() ? `\nServer ${server()!.commit || "unknown"} (${server()!.committedAt || "?"})` : "")}>
      <Show when={props.server && !same()} fallback={<div>Version {short(app)}</div>}>
        <div>App {short(app)}</div>
        <Show when={!server.loading}>
          <div>Server {server() ? short(server()!) : "unreachable"}</div>
        </Show>
      </Show>
    </div>
  );
}
