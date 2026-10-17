import { For, Show, createSignal, onCleanup, onMount } from "solid-js";
import { type Device } from "../api";
import { when } from "../admin/format";
import { type CaptureFix, type CaptureHealthStatus, type CaptureState, thisPhone } from "../deviceImport";
import { errorText } from "./TypeEditor";

// Settings > Devices > This device, "Capture health" (ticket 91): whether each kind of capture can
// work on this phone, battery optimisation, and whether the background work runs. Each problem has a
// button that opens the exact system screen that fixes it. Coming back to the app checks again.

/** A capture kind's state, in words. */
export function stateLabel(state: CaptureState): string {
  switch (state) {
    case "working":
      return "Working";
    case "permission_missing":
      return "Permission missing";
    case "restricted":
      return "Blocked by restricted settings";
    case "not_set_up":
      return "Not set up";
  }
}

/** What a fix's button says. */
export function fixLabel(fix: CaptureFix): string {
  switch (fix) {
    case "app_details":
      return "Open App info";
    case "notification_access":
      return "Open notification access";
    case "battery":
      return "Stop optimising battery";
    case "capture_setup":
      return "Set it up";
  }
}

/** The warning line a device card shows when the phone reported a capture problem, or null. */
export function healthWarning(device: Pick<Device, "health">): string | null {
  const problems = device.health?.problems ?? [];
  if (problems.length === 0) return null;
  return `The phone reports a capture problem: ${problems.join("; ")}.`;
}

/** The "Capture health" section of This device, inside the Android app. */
export function CaptureHealth() {
  const [health, setHealth] = createSignal<CaptureHealthStatus | null>(null);
  const [error, setError] = createSignal<string | null>(null);
  let section!: HTMLElement;

  async function load() {
    try {
      setHealth(await thisPhone.captureHealth());
      setError(null);
    } catch (e) {
      setError(errorText(e));
    }
  }

  // Back from the system screen: the user may have fixed it.
  const onVisible = () => {
    if (document.visibilityState === "visible") void load();
  };

  onMount(() => {
    if (window.location.hash === "#capture-health") section.scrollIntoView();
    document.addEventListener("visibilitychange", onVisible);
    void load();
  });
  onCleanup(() => document.removeEventListener("visibilitychange", onVisible));

  async function fix(f: CaptureFix) {
    setError(null);
    try {
      await thisPhone.openCaptureFix(f);
    } catch (e) {
      setError(errorText(e));
    }
  }

  function FixButton(props: { fix: CaptureFix | null | undefined }) {
    return (
      <Show when={props.fix}>
        {(f) => (
          <button type="button" data-fix={f()} onClick={() => void fix(f())}>
            {fixLabel(f())}
          </button>
        )}
      </Show>
    );
  }

  return (
    <section class="capture-health" id="capture-health" ref={section} aria-label="Capture health">
      <h3>Capture health</h3>
      <Show when={health()}>
        {(h) => (
          <>
            <Show
              when={h().problems.length > 0}
              fallback={<p class="muted">Every kind of capture can work.</p>}
            >
              <p class="error" role="alert">
                Something stops capture on this phone. Fix it below.
              </p>
            </Show>
            <ul class="capture-health-list">
              <For each={h().kinds}>
                {(k) => (
                  <li class="capture-health-item" data-kind={k.kind} data-state={k.state}>
                    <strong>{k.label}</strong>
                    <span class={k.state === "working" ? "muted" : "error"}> · {stateLabel(k.state)}</span>
                    <Show when={k.state !== "working"}>
                      <p class="muted capture-health-message">{k.message}</p>
                      <div class="actions">
                        <FixButton fix={k.fix} />
                        <FixButton fix={k.then} />
                      </div>
                    </Show>
                  </li>
                )}
              </For>
              <li
                class="capture-health-item"
                data-kind="battery"
                data-state={h().battery.optimised ? "optimised" : "working"}
              >
                <strong>Battery</strong>
                <span class={h().battery.optimised ? "error" : "muted"}>
                  {" "}
                  · {h().battery.optimised ? "Optimised" : "Not optimised"}
                </span>
                <p class="muted capture-health-message">{h().battery.message}</p>
                <Show when={h().battery.samsung_line}>
                  {(line) => <p class="capture-health-message">{line()}</p>}
                </Show>
                <div class="actions">
                  <FixButton fix={h().battery.fix} />
                </div>
              </li>
              <For each={h().background.filter((b) => b.expected)}>
                {(b) => (
                  <li class="capture-health-item" data-kind={b.work} data-state={b.stale ? "stale" : "working"}>
                    <strong>{b.label}</strong>
                    <span class={b.stale ? "error" : "muted"}>
                      {" "}
                      · {b.stale ? "Stalled" : "Running"} · last ran {b.last === null ? "never" : when(b.last)}
                    </span>
                    <Show when={b.stale}>
                      <p class="muted capture-health-message">
                        Android has not run it for over two hours. Stop battery optimisation for TheVault, and on a
                        Samsung phone keep it out of Sleeping apps.
                      </p>
                    </Show>
                  </li>
                )}
              </For>
            </ul>
          </>
        )}
      </Show>
      <Show when={error()}>
        <p class="error" role="alert">
          {error()}
        </p>
      </Show>
    </section>
  );
}
