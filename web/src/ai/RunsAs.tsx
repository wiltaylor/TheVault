// The "Runs as" list of a proposal card that changes a saved script (TV-70, TV-32 Q7): every type
// handler, task, device file and tool that runs the script, with the hosts, secrets, paths and
// schedule each gives it, and a warning line for each run that may reach any host. The script and
// the tool proposal cards share it.

import { For, Show } from "solid-js";
import type { ScriptUse } from "./store";

/** A user as a heading: `Task nightly`, `Handler after_update of type item`. */
export function userLabel(u: ScriptUse): string {
  switch (u.kind) {
    case "handler":
      return u.event ? `Handler ${u.event} of type ${u.name}` : `Handler of type ${u.name}`;
    case "task":
      return `Task ${u.name}`;
    case "device":
      return u.event ? `Device file ${u.name}: ${u.event}` : `Device file ${u.name}`;
    default:
      return `Tool ${u.name}`;
  }
}

/** What a user's run may reach and when it runs, one line each. */
export function grantsOf(u: ScriptUse): string[] {
  const out: string[] = [];
  if (u.schedule) out.push(`Schedule: ${u.schedule}${u.timezone ? ` (${u.timezone})` : ""}${u.paused ? ", paused" : ""}`);
  if (u.allow_domains.includes("*")) out.push("Hosts: any host, including your local network");
  else if (u.allow_domains.length > 0) out.push(`Hosts: ${u.allow_domains.join(", ")}`);
  if (u.secrets.length > 0) out.push(`Secrets: ${u.secrets.join(", ")}`);
  if (u.read_paths.length > 0) out.push(`Reads: ${u.read_paths.join(", ")}`);
  if (u.write_paths.length > 0) out.push(`Writes: ${u.write_paths.join(", ")}`);
  if (out.length === 0) out.push(u.kind === "device" ? "Runs on the device" : "No hosts, secrets or paths");
  return out;
}

export function RunsAs(props: { uses: ScriptUse[]; warnings: string[]; heading: string; empty?: string }) {
  return (
    <>
      <Show when={props.warnings.length > 0}>
        <ul class="ai-type-issues" aria-label="Warnings">
          <For each={props.warnings}>{(w) => <li role="note">{w}</li>}</For>
        </ul>
      </Show>
      <Show when={props.uses.length > 0} fallback={<Show when={props.empty}>{(e) => <p class="muted">{e()}</p>}</Show>}>
        <p class="muted">{props.heading}</p>
        <ul class="ai-runs-as" aria-label="Runs as">
          <For each={props.uses}>
            {(u) => (
              <li>
                <strong>{userLabel(u)}</strong> <span class="muted">{u.file}</span>
                <ul>
                  <For each={grantsOf(u)}>{(g) => <li class="muted">{g}</li>}</For>
                </ul>
              </li>
            )}
          </For>
        </ul>
      </Show>
    </>
  );
}
