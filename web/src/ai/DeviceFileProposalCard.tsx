import { For, Show } from "solid-js";
import { RunsAs } from "./RunsAs";
import { OnPhone } from "./ScriptProposalCard";
import { DiffLines } from "./ToolProposalCard";
import { type Part, useChatSession } from "./store";

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
      <For each={p().devices}>{(d) => <p class="muted">{`${d.name}: settings ${d.settings}. Permission report: ${d.health ?? "not reported"}`}</p>}</For>
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
