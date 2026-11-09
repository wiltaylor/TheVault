import { For, Show } from "solid-js";
import { RunsAs } from "./RunsAs";
import { OnPhone } from "./ScriptProposalCard";
import { DiffLines } from "./ToolProposalCard";
import { type Part, useChatSession } from "./store";

function commands(settings: string): string {
  try {
    const value: unknown = JSON.parse(settings);
    const list = value && typeof value === "object" && "commands" in value ? value.commands : [];
    return Array.isArray(list) ? list.filter((v): v is string => typeof v === "string").join(", ") || "none" : "none";
  } catch {
    return "not reported";
  }
}

function healthProblems(health: string | null | undefined): string[] | null {
  if (!health) return null;
  try {
    const value: unknown = JSON.parse(health);
    const problems = value && typeof value === "object" && "problems" in value ? value.problems : null;
    return Array.isArray(problems) ? problems.filter((p): p is string => typeof p === "string") : null;
  } catch {
    return null;
  }
}

export function DeviceFileProposalCard(props: { part: Extract<Part, { kind: "device_file_proposal" }>; turn: number; index: number }) {
  const session = useChatSession();
  const p = () => props.part.proposal;
  const open = () => ["open", "failed", "applying"].includes(props.part.state);
  return (
    <section class="ai-proposal" aria-label="Proposed Android device file">
      <p class="ai-proposal-head">Proposed Android device file and scripts</p>
      <p class="muted">{`Proposed by ${p().proposer ?? "agent"}. Runs on your phone at its next sync, with no further prompt there.`}</p>
      <Show when={p().requires_admin}><p role="note">A signed-in admin must apply this change because the current or proposed phone code sends SMS or writes contacts.</p></Show>
      <DiffLines before={p().before} after={p().after} label="devices/android.wcl" />
      <For each={p().warnings}>{(warning) => <p role="note">{warning}</p>}</For>
      <For each={p().devices}>{(d) => <div>
        <p class="muted">{`${d.name}: enabled commands: ${commands(d.settings)}.`}</p>
        <Show when={healthProblems(d.health) !== null} fallback={<p class="muted">Capture health not reported.</p>}>
          <ul aria-label={`${d.name} capture health problems`}>
            <For each={healthProblems(d.health) ?? []} fallback={<li>No problems reported.</li>}>{(problem) => <li>{problem}</li>}</For>
          </ul>
        </Show>
      </div>}</For>
      <For each={p().snapshots}>{(s) => <div>
        <p><code>{`scripts/${s.name}.ws`}</code></p>
        <DiffLines before={s.before} after={s.after} label={s.name} />
        <Show when={s.device}>{(d) => <OnPhone device={d()} />}</Show>
        <RunsAs uses={s.runs_as} warnings={s.warnings} heading="Runs as after Apply" empty="No longer runs on this phone." />
      </div>}</For>
      <Show when={props.part.error}><p role="alert">{props.part.error}</p></Show>
      <Show when={open()}>
        <button type="button" disabled={props.part.state === "applying"} onClick={() => void session.applyDeviceFileProposal(props.turn, props.index)}>Apply</button>
      </Show>
      <Show when={open() || props.part.state === "stale"}>
        <button type="button" disabled={props.part.state === "applying"} onClick={() => session.discardDeviceFileProposal(props.turn, props.index)}>Discard</button>
      </Show>
      <Show when={props.part.state === "applied" || props.part.state === "discarded"}><p role="status">{props.part.state === "applied" ? "Applied." : "Discarded."}</p></Show>
    </section>
  );
}
