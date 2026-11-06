import { For, Show, createSignal } from "solid-js";
import { type Json, api } from "../api";
import { applyEdits } from "../notes";

const answers = [
  { value: "accepted", action: "Accept", label: "Accepted" },
  { value: "tentative", action: "Maybe", label: "Maybe" },
  { value: "declined", action: "Decline", label: "Declined" },
] as const;

/** The same note edit on web and offline clients. Only the home server writes to Google. */
export function EventResponse(props: {
  response: Json | undefined;
  disabled?: boolean;
  onAnswer: (response: string) => void | Promise<void>;
}) {
  const [changing, setChanging] = createSignal(false);
  const [busy, setBusy] = createSignal(false);
  const [error, setError] = createSignal("");
  const current = () => answers.find((a) => a.value === props.response);
  async function answer(value: string) {
    setBusy(true);
    setError("");
    try {
      await props.onAnswer(value);
      setChanging(false);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  }
  return (
    <Show when={props.response === "needs_action" || current()}>
      <div role="group" aria-label="Invitation response" onClick={(e) => e.stopPropagation()}>
        <Show when={props.response === "needs_action" || changing()} fallback={
          <><span>Your response: {current()?.label}</span>{" "}
            <button type="button" disabled={props.disabled || busy()} onClick={() => setChanging(true)}>Change response</button></>
        }>
          <span>Going? </span>
          <For each={answers}>{(a) =>
            <button type="button" disabled={props.disabled || busy()} onClick={() => void answer(a.value)}>{a.action}</button>
          }</For>
        </Show>
        <Show when={error()}><p role="alert">{error()}</p></Show>
      </div>
    </Show>
  );
}

export function EventCardResponse(props: { id: string; response: Json | undefined }) {
  return <EventResponse response={props.response} onAnswer={async (response) => {
    const note = await api.note(props.id);
    await api.updateNote(props.id, applyEdits(note.note, { my_response: response }), note.version);
  }} />;
}
