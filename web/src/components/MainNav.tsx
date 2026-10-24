// The sidebar's main links, with the counts that wait behind Inbox and Outbox, and the Log.

import { For, type JSX, Show } from "solid-js";
import { inboxCount } from "../capture/store";
import { outboxCount } from "../outbox/store";
import { isActiveLink } from "../workspace/links";
import { CalendarIcon, InboxIcon, LogIcon, OutboxIcon, SearchIcon } from "./Icon";

/** The sidebar's main links. Each opens in the active pane; a middle-click opens a background tab. */
export const NAV: { href: string; label: string; icon: () => JSX.Element; badge?: () => number | null }[] = [
  { href: "/daily", label: "Daily notes", icon: CalendarIcon },
  { href: "/search", label: "Search", icon: SearchIcon },
  { href: "/inbox", label: "Inbox", icon: InboxIcon, badge: inboxCount },
  // The items an agent drafted, waiting for review (outbox X7).
  { href: "/outbox", label: "Outbox", icon: OutboxIcon, badge: outboxCount },
  // What happened in the vault, the AI's work above all (activity Y4).
  { href: "/log", label: "Log", icon: LogIcon },
];

/** One `<li>` per main link, each with its badge when something waits. */
export function MainNavLinks(props: { onNavigate?: () => void }) {
  return (
    <For each={NAV}>
      {(item) => (
        <li>
          <a href={item.href} class="nav-link" classList={{ active: isActiveLink(item.href) }} onClick={() => props.onNavigate?.()}>
            {item.icon()}
            <span>{item.label}</span>
            <Show when={item.badge?.()}>
              {(n) => (
                <span class="nav-badge" title={`${n()} waiting`}>
                  {n()}
                </span>
              )}
            </Show>
          </a>
        </li>
      )}
    </For>
  );
}
