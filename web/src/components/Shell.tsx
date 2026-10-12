import { A, Navigate, useLocation, useNavigate } from "@solidjs/router";
import { For, type ParentProps, Show, createSignal } from "solid-js";
import { logOut, sessionState, types } from "../session";

/** The signed-in layout: a header with search, the types sidebar, and the page. */
export function Shell(props: ParentProps) {
  const navigate = useNavigate();
  const location = useLocation();
  const [menuOpen, setMenuOpen] = createSignal(false);
  const [query, setQuery] = createSignal("");

  function onSearch(ev: SubmitEvent) {
    ev.preventDefault();
    const q = query().trim();
    if (q) navigate(`/search?q=${encodeURIComponent(q)}`);
  }

  return (
    <Show
      when={sessionState() === "in"}
      fallback={
        <Show when={sessionState() === "out"} fallback={<p class="muted boot">Loading…</p>}>
          <Navigate href={`/login?next=${encodeURIComponent(location.pathname + location.search)}`} />
        </Show>
      }
    >
      <div class="shell" classList={{ "menu-open": menuOpen() }}>
        <header class="topbar">
          <button
            type="button"
            class="menu-button"
            aria-label="Menu"
            aria-expanded={menuOpen()}
            aria-controls="sidebar"
            onClick={() => setMenuOpen(!menuOpen())}
          >
            ☰
          </button>
          <A href="/" class="brand">
            TheVault
          </A>
          <form class="search" role="search" onSubmit={onSearch}>
            <input
              type="search"
              placeholder="Search notes"
              aria-label="Search notes"
              value={query()}
              onInput={(e) => setQuery(e.currentTarget.value)}
            />
          </form>
          <button type="button" class="logout" onClick={() => void logOut()}>
            Log out
          </button>
        </header>
        <div id="sidebar" class="sidebar">
          <nav aria-label="Types">
            <h2>Types</h2>
            <ul>
              <For each={types()}>
                {(t) => (
                  <li>
                    <A
                      href={`/types/${t.name}`}
                      activeClass="active"
                      data-colour={t.colour ?? undefined}
                      onClick={() => setMenuOpen(false)}
                    >
                      {t.plural}
                    </A>
                  </li>
                )}
              </For>
            </ul>
            <ul class="sidebar-admin">
              <li>
                <A href="/types/new" activeClass="active" end onClick={() => setMenuOpen(false)}>
                  New type
                </A>
              </li>
              <li>
                <A href="/taxonomies" activeClass="active" onClick={() => setMenuOpen(false)}>
                  Taxonomies
                </A>
              </li>
            </ul>
          </nav>
          <nav aria-label="Vault">
            <h2 class="sidebar-admin">Vault</h2>
            <ul>
              <For
                each={[
                  ["/scripts", "Scripts"],
                  ["/tasks", "Tasks"],
                  ["/secrets", "Secrets"],
                  ["/tokens", "API tokens"],
                  ["/trash", "Trash"],
                  ["/flagged", "Flagged notes"],
                ]}
              >
                {([href, label]) => (
                  <li>
                    <A href={href!} activeClass="active" onClick={() => setMenuOpen(false)}>
                      {label}
                    </A>
                  </li>
                )}
              </For>
            </ul>
          </nav>
        </div>
        <div class="scrim" onClick={() => setMenuOpen(false)} />
        <main class="content">{props.children}</main>
      </div>
    </Show>
  );
}
