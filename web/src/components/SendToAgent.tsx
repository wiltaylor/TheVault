// Send to agent (TV-77 D7, TV-78): one dialog that sends notes to an agent with an instruction,
// which is required (TV-77 Q4 B). It offers the enabled agents, posts every note in one request
// (one thread for all of them, TV-77 Q1 B), and once sent links the new thread. The note page
// opens it for its note; lists that select notes open it with all of theirs. What the server
// refuses (a note the agent may not read, a token that may not write, too many runs) shows in the
// dialog, which stays open so the instruction is not lost.

import { Portal } from "solid-js/web";
import { For, Show, createMemo, createSignal, onCleanup, onMount } from "solid-js";
import { agentLabel, agents, refreshAgents } from "../agents";
import { ApiError, api } from "../api";
import { notesSent } from "../dispatch";
import { dispatchedHref } from "./AgentRobot";

/** The longest instruction the dialog takes, in characters. */
export const INSTRUCTION_MAX = 4_000;

/** What a refused send says, by what the server answered. */
export function sendErrorText(e: unknown): string {
  if (e instanceof ApiError) {
    if (e.status === 403) return "This device may not send notes to an agent: it needs a token that may write.";
    if (e.status === 429) return e.body.message ?? "The agent has too many runs now: try again later.";
    if (e.needsServer || e.body.error === "ai_needs_server") return "Sending notes to an agent needs the home server, and it cannot be reached.";
    return e.body.message ?? e.message;
  }
  return e instanceof Error ? e.message : String(e);
}

/** The dialog that sends `notes` to an agent. `onSent` gets the new thread's agent and id. */
export function SendToAgentDialog(props: { notes: readonly string[]; onClose: () => void; onSent?: (agent: string, thread: string) => void }) {
  const choices = createMemo(() => agents().filter((a) => a.enabled !== false && !a.error));
  const [picked, setPicked] = createSignal<string | null>(null);
  const agent = () => {
    const p = picked();
    if (p && choices().some((a) => a.name === p)) return p;
    return choices()[0]?.name ?? null;
  };
  const [instruction, setInstruction] = createSignal("");
  const [busy, setBusy] = createSignal(false);
  const [error, setError] = createSignal<string | null>(null);
  const [sent, setSent] = createSignal<{ agent: string; thread: string } | null>(null);
  const canSend = () => !busy() && agent() !== null && instruction().trim() !== "" && props.notes.length > 0;
  let text!: HTMLTextAreaElement;
  const returnTo = document.activeElement instanceof HTMLElement ? document.activeElement : null;
  onMount(() => {
    if (agents().length === 0) void refreshAgents();
    text.focus();
    onCleanup(() => setTimeout(() => returnTo?.isConnected && returnTo.focus()));
  });

  async function send(e?: Event) {
    e?.preventDefault();
    const name = agent();
    if (!canSend() || !name) return;
    setBusy(true);
    setError(null);
    try {
      const got = await api.dispatch(name, [...props.notes], instruction().trim());
      setSent({ agent: name, thread: got.thread.id });
      notesSent(props.notes);
      props.onSent?.(name, got.thread.id);
    } catch (err) {
      setError(sendErrorText(err));
    } finally {
      setBusy(false);
    }
  }

  const label = (name: string) => {
    const a = agents().find((x) => x.name === name);
    return a ? agentLabel(a) : name;
  };
  const what = () => (props.notes.length === 1 ? "this note" : `${props.notes.length} notes`);

  return (
    <Portal>
      <div
        class="capture-backdrop"
        onMouseDown={(e) => {
          if (e.target === e.currentTarget && !busy()) props.onClose();
        }}
      >
        <form
          class="capture-dialog send-agent-dialog"
          role="dialog"
          aria-modal="true"
          aria-labelledby="send-agent-heading"
          data-testid="send-agent-dialog"
          onSubmit={(e) => void send(e)}
          onKeyDown={(e) => {
            if (e.key === "Escape" && !busy()) {
              e.preventDefault();
              props.onClose();
            } else if (e.key === "Enter" && (e.ctrlKey || e.metaKey)) {
              void send(e);
            }
          }}
        >
          <header class="capture-head">
            <h2 id="send-agent-heading">Send {what()} to an agent</h2>
            <button type="button" class="capture-x" aria-label="Close" title="Close (Esc)" disabled={busy()} onClick={() => props.onClose()}>
              ×
            </button>
          </header>
          <Show
            when={sent()}
            fallback={
              <>
                <label class="send-agent-field">
                  <span>Agent</span>
                  <select value={agent() ?? ""} disabled={busy() || choices().length === 0} onChange={(e) => setPicked(e.currentTarget.value)}>
                    <For each={choices()}>{(a) => <option value={a.name}>{agentLabel(a)}</option>}</For>
                  </select>
                </label>
                <Show when={choices().length === 0}>
                  <p class="muted">No agent is enabled.</p>
                </Show>
                <label class="send-agent-field">
                  <span>Instruction</span>
                  <textarea
                    ref={text}
                    class="capture-text"
                    required
                    maxLength={INSTRUCTION_MAX}
                    placeholder="What the agent is to do with the notes"
                    value={instruction()}
                    disabled={busy()}
                    onInput={(e) => setInstruction(e.currentTarget.value)}
                  />
                </label>
                <Show when={error()}>
                  <p class="error" role="alert">
                    {error()}
                  </p>
                </Show>
                <div class="actions">
                  <button type="button" disabled={busy()} onClick={() => props.onClose()}>
                    Cancel
                  </button>
                  <button type="submit" class="primary" disabled={!canSend()} title={instruction().trim() ? undefined : "Say what the agent is to do"}>
                    {busy() ? "Sending…" : "Send"}
                  </button>
                </div>
              </>
            }
          >
            {(s) => (
              <>
                <p role="status">Sent to {label(s().agent)}.</p>
                <div class="actions">
                  <a class="button" href={dispatchedHref(s().agent, s().thread)} onClick={() => props.onClose()}>
                    Open thread
                  </a>
                  <button type="button" class="primary" onClick={() => props.onClose()}>
                    Close
                  </button>
                </div>
              </>
            )}
          </Show>
        </form>
      </div>
    </Portal>
  );
}

/** The note page's Send to agent button, and the dialog it opens for `notes`. */
export function SendToAgentButton(props: { notes: readonly string[]; disabled?: boolean }) {
  const [open, setOpen] = createSignal(false);
  return (
    <>
      <button type="button" class="outbox-note-action send-agent-button" disabled={props.disabled} onClick={() => setOpen(true)}>
        Send to agent
      </button>
      <Show when={open()}>
        <SendToAgentDialog notes={props.notes} onClose={() => setOpen(false)} />
      </Show>
    </>
  );
}
