// The note page's Outbox buttons (outbox X8): Reply on an email, Propose a change on an event,
// and Send, Approve and send or Retry on an Outbox item itself.

import { useNavigate } from "@solidjs/router";
import { Show, createSignal } from "solid-js";
import type { NoteDoc } from "../api";
import { showToast } from "../components/Toasts";
import { typeDef } from "../session";
import { errorText } from "../pages/TypeEditor";
import { createDraft } from "./actions";
import { EMAIL, EVENT, OUTBOX, type Status, changeDraft, replyDraft, statusOf } from "./model";

export function NoteOutboxActions(props: {
  id: string;
  type: string;
  doc: NoteDoc;
  disabled?: boolean;
  /** Set the open item's status, through the note page's own save. */
  setStatus: (status: Status, extra?: Record<string, undefined>) => void;
}) {
  const navigate = useNavigate();
  const [busy, setBusy] = createSignal(false);
  // The Outbox is a built-in type; a vault that has it removed offers none of these.
  const hasOutbox = () => !!typeDef(OUTBOX);

  async function draft(doc: NoteDoc) {
    setBusy(true);
    try {
      const made = await createDraft(doc);
      navigate(`/notes/${made.id}`);
    } catch (e) {
      showToast(errorText(e));
    } finally {
      setBusy(false);
    }
  }

  const status = () => statusOf(props.doc.fields);
  const off = () => busy() || !!props.disabled;
  return (
    <Show when={hasOutbox()}>
      <Show when={props.type === EMAIL}>
        <button type="button" class="outbox-note-action" disabled={off()} onClick={() => void draft(replyDraft(props.id, props.doc))}>
          Reply
        </button>
      </Show>
      <Show when={props.type === EVENT}>
        <button type="button" class="outbox-note-action" disabled={off()} onClick={() => void draft(changeDraft(props.id, props.doc))}>
          Propose a change
        </button>
      </Show>
      <Show when={props.type === OUTBOX}>
        <Show when={status() === "draft"}>
          <button type="button" class="primary outbox-note-action" disabled={off()} onClick={() => props.setStatus("approved")}>
            Send
          </button>
        </Show>
        <Show when={status() === "review"}>
          <button type="button" class="primary outbox-note-action" disabled={off()} onClick={() => props.setStatus("approved")}>
            Approve and send
          </button>
        </Show>
        <Show when={status() === "failed"}>
          <button
            type="button"
            class="primary outbox-note-action"
            disabled={off()}
            onClick={() => props.setStatus("approved", { error: undefined })}
          >
            Retry
          </button>
        </Show>
      </Show>
    </Show>
  );
}
