import { A, Navigate, useLocation } from "@solidjs/router";
import { For, type ParentProps, createResource } from "solid-js";
import { appInfo } from "../inApp";

const SECTIONS: [string, string][] = [
  ["/settings", "General"],
  ["/settings/types", "Types"],
  ["/settings/taxonomies", "Taxonomies"],
  ["/settings/scheduled", "Scheduled"],
  ["/settings/tools", "Tools"],
  ["/settings/secrets", "Secrets"],
  ["/settings/tokens", "API tokens"],
  ["/settings/devices", "Devices"],
  ["/settings/integrations", "Integrations"],
  ["/settings/ai", "AI"],
];

/**
 * The sections the apps leave out: they hold credentials, or (Tools) write and run definitions,
 * which need `admin` on the home server, and an app's device token never has it (D2). They are
 * managed in the web app.
 */
export const WEB_APP_ONLY = ["/settings/tools", "/settings/secrets", "/settings/tokens", "/settings/integrations", "/settings/ai"];

/** The settings area: tabs for each section above the section's own page. */
export function SettingsLayout(props: ParentProps) {
  const [app] = createResource(appInfo);
  const sections = () => (app() ? SECTIONS.filter(([href]) => !WEB_APP_ONLY.includes(href)) : SECTIONS);
  return (
    <div class="settings-layout">
      <nav class="settings-tabs" aria-label="Settings sections">
        <For each={sections()}>
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

/** Send a link to the old tasks section, `/settings/tasks/x`, to `/settings/scheduled/x`. */
export function MovedToScheduled() {
  const location = useLocation();
  const rest = location.pathname.replace(/^\/settings\/tasks/, "");
  return <Navigate href={`/settings/scheduled${rest}${location.search}`} />;
}
