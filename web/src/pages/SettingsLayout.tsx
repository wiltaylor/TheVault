import { A, Navigate, useLocation } from "@solidjs/router";
import { For, type ParentProps } from "solid-js";

const SECTIONS: [string, string][] = [
  ["/settings", "General"],
  ["/settings/types", "Types"],
  ["/settings/taxonomies", "Taxonomies"],
  ["/settings/tasks", "Tasks"],
  ["/settings/secrets", "Secrets"],
  ["/settings/tokens", "API tokens"],
];

/** The settings area: tabs for each section above the section's own page. */
export function SettingsLayout(props: ParentProps) {
  return (
    <div class="settings-layout">
      <nav class="settings-tabs" aria-label="Settings sections">
        <For each={SECTIONS}>
          {([href, label]) => (
            <A href={href} activeClass="active" end={href === "/settings"}>
              {label}
            </A>
          )}
        </For>
      </nav>
      {props.children}
    </div>
  );
}

/** Send a link to a section's old address, such as `/scripts/x`, to its place under `/settings`. */
export function MovedToSettings() {
  const location = useLocation();
  return <Navigate href={`/settings${location.pathname}${location.search}`} />;
}
