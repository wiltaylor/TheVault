import { A, Navigate, useNavigate, useSearchParams } from "@solidjs/router";
import { Show, createSignal } from "solid-js";
import { ApiError } from "../api";
import { logIn, sessionState } from "../session";

/** Only a path inside the app, so a crafted link cannot send the browser elsewhere. */
function safeNext(next: unknown): string {
  return typeof next === "string" && next.startsWith("/") && !next.startsWith("//") ? next : "/";
}

export function Login() {
  const navigate = useNavigate();
  const [params] = useSearchParams();
  const [password, setPassword] = createSignal("");
  const [error, setError] = createSignal<string | null>(null);
  const [busy, setBusy] = createSignal(false);

  async function onSubmit(ev: SubmitEvent) {
    ev.preventDefault();
    setBusy(true);
    setError(null);
    try {
      await logIn(password());
      navigate(safeNext(params.next), { replace: true });
    } catch (e) {
      setError(
        e instanceof ApiError && e.status === 401
          ? "That password is not right."
          : `Could not log in: ${e instanceof Error ? e.message : String(e)}`,
      );
      setBusy(false);
    }
  }

  return (
    <main class="login">
      <Show when={sessionState() === "in" && !busy()}>
        <Navigate href={safeNext(params.next)} />
      </Show>
      <form class="login-card" onSubmit={onSubmit}>
        <h1>TheVault</h1>
        <label for="password">Password</label>
        <input
          id="password"
          type="password"
          autocomplete="current-password"
          required
          value={password()}
          onInput={(e) => setPassword(e.currentTarget.value)}
        />
        <Show when={error()}>
          <p class="error" role="alert">
            {error()}
          </p>
        </Show>
        <button type="submit" class="primary" disabled={busy()}>
          Log in
        </button>
        <A class="login-apps" href="/apps">
          Get the apps
        </A>
      </form>
    </main>
  );
}
