// A skill proposal's card (skills S9): the SKILL.md or file lines removed and added, as the type
// proposal card shows them, or for a binary file its size and type, with Apply and Discard. A
// delete shows what goes. Apply writes nothing when the skill changed since the proposal.

import { For, Match, Show, Switch } from "solid-js";
import { sizeText } from "../skills";
import type { DiffLine } from "./diff";
import { type Part, type SkillProposal, useChatSession } from "./store";
import { wclDiff } from "./TypeProposalCard";

/** The skill screen of skill `name`. */
export const skillHref = (name: string) => `/skills/${encodeURIComponent(name)}`;

/** The card's heading, before the skill's name. */
function head(p: SkillProposal): string {
  switch (p.op) {
    case "create":
      return "Proposed new skill ";
    case "change":
      return "Proposed change to SKILL.md of skill ";
    case "file":
      return `Proposed ${p.exists ? "change to" : "new file"} ${p.path ?? ""} in skill `;
    case "file_delete":
      return `Proposed delete of ${p.path ?? ""} from skill `;
    default:
      return "Proposed delete of skill ";
  }
}

/** A file's size and type, as a binary file shows them. */
const meta = (size: number | null | undefined, type: string | null | undefined) =>
  [typeof size === "number" ? sizeText(size) : null, type ?? null].filter(Boolean).join(", ");

export function SkillProposalCard(props: { part: Extract<Part, { kind: "skill_proposal" }>; turn: number; index: number }) {
  const session = useChatSession();
  const p = () => props.part.proposal;
  // Text shows as lines; a binary file, or text too long to show, as its size and type.
  const textual = () => {
    const x = p();
    if (x.op === "file") return x.after !== null && (x.before !== null || !x.exists);
    if (x.op === "file_delete") return x.before !== null;
    return true;
  };
  const rows = () => {
    const x = p();
    return x.op === "delete" || x.op === "file_delete" ? wclDiff(x.before, "") : wclDiff(x.before, x.after ?? "");
  };
  const deleted = () => {
    const x = p();
    return (x.op === "delete" || x.op === "file_delete") && x.files ? x.files : [];
  };
  const open = () => props.part.state === "open" || props.part.state === "failed" || props.part.state === "applying";
  return (
    <section class="ai-proposal" aria-label={`${head(p())}${p().name}`}>
      <p class="ai-proposal-head">
        {head(p())}
        <Show when={p().op !== "create" && p().skill_exists !== false} fallback={<code>{p().name}</code>}>
          <a href={skillHref(p().name)}>{p().name}</a>
        </Show>
      </p>
      <Show when={p().op === "file" && p().skill_exists === false}>
        <p class="muted">The skill does not exist yet: apply its SKILL.md first.</p>
      </Show>
      <Show
        when={textual()}
        fallback={
          <p class="ai-skill-binary muted">
            <Show when={p().exists && p().op === "file"}>
              {`Replaces ${meta(p().before_size, p().before_media_type)} with `}
            </Show>
            {meta(p().size, p().media_type)}
          </p>
        }
      >
        <pre class="ai-diff-lines">
          <For each={rows()}>
            {(l) => (
              <Switch fallback={<span class="ai-same">{`  ${(l as DiffLine).text}\n`}</span>}>
                <Match when={l.type === "skip"}>
                  <span class="muted">{`  … ${(l as { count: number }).count} unchanged ${(l as { count: number }).count === 1 ? "line" : "lines"}\n`}</span>
                </Match>
                <Match when={l.type === "del"}>
                  <del>{`- ${(l as DiffLine).text}\n`}</del>
                </Match>
                <Match when={l.type === "add"}>
                  <ins>{`+ ${(l as DiffLine).text}\n`}</ins>
                </Match>
              </Switch>
            )}
          </For>
        </pre>
      </Show>
      <Show when={deleted().length > 0 && (p().op === "delete" || deleted().length > 1)}>
        <ul class="ai-skill-files" aria-label="Files deleted">
          <For each={deleted()}>{(f) => <li class="muted">{f}</li>}</For>
        </ul>
      </Show>
      <Show when={p().issues.length > 0}>
        <ul class="ai-type-issues">
          <For each={p().issues}>{(i) => <li class="muted">{i.at ? `${i.at}: ${i.message}` : i.message}</li>}</For>
        </ul>
      </Show>
      <Show when={props.part.error}>
        <p role="alert" class="ai-error">
          {props.part.error}
        </p>
      </Show>
      <Switch>
        <Match when={open()}>
          <div class="ai-proposal-actions">
            <button type="button" class="primary" disabled={props.part.state === "applying"} onClick={() => void session.applySkillProposal(props.turn, props.index)}>
              Apply
            </button>
            <button type="button" disabled={props.part.state === "applying"} onClick={() => session.discardSkillProposal(props.turn, props.index)}>
              Discard
            </button>
          </div>
        </Match>
        <Match when={props.part.state === "stale"}>
          <div class="ai-proposal-actions">
            <button type="button" onClick={() => session.discardSkillProposal(props.turn, props.index)}>
              Discard
            </button>
          </div>
        </Match>
        <Match when={props.part.state === "applied"}>
          <p role="status" class="ai-applied">
            Applied.{" "}
            <Show when={p().op !== "delete"}>
              <a href={skillHref(p().name)}>Open skill</a>
            </Show>
          </p>
        </Match>
        <Match when={props.part.state === "discarded"}>
          <p role="status" class="muted">
            Discarded.
          </p>
        </Match>
      </Switch>
    </section>
  );
}
