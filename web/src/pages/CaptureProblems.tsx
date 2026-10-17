import { For, Show, createSignal, onMount } from "solid-js";
import { type CaptureProblem, api } from "../api";
import { when } from "../admin/format";
import { errorText } from "./TypeEditor";

// Settings > Devices > This device, "Capture problems" (ticket 88): the device events whose handler
// failed on every pass, and those the exclusion list or skip pattern dropped, newest first. A failed
// one can be retried, three more passes with the first at once, or dismissed. The count of failed
// ones is a badge on This device.

const [failedCount, setFailedCount] = createSignal(0);

/** How many device events failed for good and are not dismissed, as the page last heard. */
export { failedCount };

/** What a device event is, in words. */
export function eventLabel(event: string): string {
  switch (event) {
    case "sms_received":
      return "SMS received";
    case "sms_sent":
      return "SMS sent";
    case "mms_received":
      return "MMS received";
    case "mms_sent":
      return "MMS sent";
    case "rcs_received":
      return "RCS message";
    case "call_ended":
      return "Call";
    case "contact_changed":
      return "Contact changed";
    case "contact_deleted":
      return "Contact deleted";
    case "share_received":
      return "Share";
    default:
      return event;
  }
}

/** How one problem stands, in words. */
export function problemStatus(p: CaptureProblem): string {
  if (p.status === "skipped") return "Skipped";
  if (p.status === "failed") return p.attempts === 1 ? "Failed after 1 try" : `Failed after ${p.attempts} tries`;
  if (p.status === "done") return "Saved";
  if (p.status === "queued") return "Waiting for another try";
  return "Dismissed";
}

/** The failed count as a badge, when any failed; for This device's heading and its card. */
export function CaptureProblemsBadge() {
  return (
    <Show when={failedCount() > 0}>
      <span
        class="capture-problems-badge"
        role="status"
        aria-label={failedCount() === 1 ? "1 capture problem" : `${failedCount()} capture problems`}
      >
        {failedCount()}
      </span>
    </Show>
  );
}

/** The "Capture problems" section of This device. */
export function CaptureProblems() {
  const [events, setEvents] = createSignal<CaptureProblem[]>([]);
  const [loaded, setLoaded] = createSignal(false);
  const [error, setError] = createSignal<string | null>(null);
  const [status, setStatus] = createSignal<string | null>(null);
  const [busy, setBusy] = createSignal<number | null>(null);

  async function load() {
    try {
      const r = await api.captureProblems();
      setEvents(r.events);
      setFailedCount(r.failed);
      setLoaded(true);
    } catch (e) {
      setError(errorText(e));
    }
  }

  onMount(() => void load());

  async function act(p: CaptureProblem, retry: boolean) {
    setError(null);
    setStatus(null);
    setBusy(p.id);
    try {
      if (retry) {
        const r = await api.retryCaptureProblem(p.id);
        setStatus(
          r.status === "done"
            ? `${eventLabel(p.event)} saved.`
            : `${eventLabel(p.event)} failed again: ${r.message ?? "no message"}. It is tried again on the next pass.`,
        );
      } else {
        await api.dismissCaptureProblem(p.id);
      }
      await load();
    } catch (e) {
      setError(errorText(e));
    } finally {
      setBusy(null);
    }
  }

  return (
    <section class="capture-problems" aria-label="Capture problems">
      <h3>Capture problems</h3>
      <p class="muted">
        A message, call or contact change whose device script failed three times is listed here, with why, until you
        retry or dismiss it. Skipped ones were left out by the exclusion list or the skip pattern.
      </p>
      <Show when={loaded() && events().length === 0}>
        <p class="muted">No capture problems.</p>
      </Show>
      <Show when={events().length > 0}>
        <ul class="capture-problem-list">
          <For each={events()}>
            {(p) => (
              <li class="capture-problem" data-event={p.id} data-status={p.status}>
                <div class="capture-problem-head">
                  <strong>{eventLabel(p.event)}</strong>
                  <Show when={p.who}>
                    {(who) => <span> from {who()}</span>}
                  </Show>
                  <span class="muted"> · {when(p.at)} · {problemStatus(p)}</span>
                </div>
                <Show when={p.message}>
                  {(m) => <p class={p.status === "failed" ? "error capture-problem-message" : "muted capture-problem-message"}>{m()}</p>}
                </Show>
                <Show when={p.status === "failed"}>
                  <div class="actions">
                    <button type="button" disabled={busy() !== null} onClick={() => void act(p, true)}>
                      Retry
                    </button>
                    <button type="button" disabled={busy() !== null} onClick={() => void act(p, false)}>
                      Dismiss
                    </button>
                  </div>
                </Show>
              </li>
            )}
          </For>
        </ul>
      </Show>
      <Show when={status()}>
        <p role="status">{status()}</p>
      </Show>
      <Show when={error()}>
        <p class="error" role="alert">
          {error()}
        </p>
      </Show>
    </section>
  );
}
