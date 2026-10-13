import { A, Navigate, useLocation, useNavigate } from "@solidjs/router";
import { For, type ParentProps, Show, createSignal } from "solid-js";
import { pins } from "../pins";
import { logOut, sessionState, typeDef } from "../session";
import { theme, toggleTheme } from "../theme";
import { GearIcon, ThemeGlyph, TrashIcon, TypeIcon } from "./Icon";

/** The signed-in layout: a header with search, the sidebar with the theme switch and settings, and the page. */
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
          <nav aria-label="Main">
            <ul>
              <For
                each={[
                  ["/", "Home"],
                  ["/types", "Types"],
                ]}
              >
                {([href, label]) => (
                  <li>
                    <A href={href!} activeClass="active" end={href === "/"} onClick={() => setMenuOpen(false)}>
                      {label}
                    </A>
                  </li>
                )}
              </For>
            </ul>
          </nav>
          <Show when={pins().length > 0}>
            <nav aria-label="Pinned" class="pinned">
              <h2>Pinned</h2>
              <ul>
                <For each={pins()}>
                  {(p) => (
                    <li>
                      <A
                        href={`/notes/${p.id}`}
                        activeClass="active"
                        title={p.title}
                        onClick={() => setMenuOpen(false)}
                      >
                        <TypeIcon name={typeDef(p.type)?.icon} size={16} />
                        <span>{p.title || "Untitled"}</span>
                      </A>
                    </li>
                  )}
                </For>
              </ul>
            </nav>
          </Show>
          <div class="sidebar-footer">
            <button
              type="button"
              role="switch"
              class="theme-switch"
              aria-checked={theme() === "dark"}
              aria-label="Dark mode"
              title={theme() === "dark" ? "Switch to light mode" : "Switch to dark mode"}
              onClick={toggleTheme}
            >
              <span class="theme-switch-track" aria-hidden="true">
                <span class="theme-switch-thumb">
                  <ThemeGlyph dark={theme() === "dark"} />
                </span>
              </span>
            </button>
            <A
              href="/trash"
              class="footer-button"
              activeClass="active"
              aria-label="Trash"
              title="Trash"
              onClick={() => setMenuOpen(false)}
            >
              <TrashIcon />
            </A>
            <A
              href="/settings"
              class="footer-button"
              activeClass="active"
              aria-label="Settings"
              title="Settings"
              onClick={() => setMenuOpen(false)}
            >
              <GearIcon />
            </A>
          </div>
        </div>
        <div class="scrim" onClick={() => setMenuOpen(false)} />
        <main class="content">{props.children}</main>
      </div>
    </Show>
  );
}
