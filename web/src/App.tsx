import { Route, createRouter, useNavigate } from "@solidjs/router";
import { type ParentProps, onMount } from "solid-js";
import { setUnauthorizedHandler } from "./api";
import { Shell } from "./components/Shell";
import { Login } from "./pages/Login";
import { refreshSession, signedOut } from "./session";
import { resyncUrl } from "./workspace/store";

function Root(props: ParentProps) {
  const navigate = useNavigate();
  setUnauthorizedHandler(() => {
    signedOut();
    if (!location.pathname.startsWith("/login")) {
      navigate(`/login?next=${encodeURIComponent(location.pathname + location.search)}`);
    }
  });
  onMount(() => void refreshSession());
  return <>{props.children}</>;
}

const here = () => window.location.pathname + window.location.search + window.location.hash;

/**
 * The browser's own router only tells the login page from the app, and catches no links: inside
 * the app every pane has a router of its own (see workspace/), and the workspace owns the address
 * bar. When this router moves the browser into the app (after a login), the workspace puts the
 * active pane's address back.
 */
const BrowserRouter = createRouter({
  get: here,
  set: ({ value, replace }) => {
    if (replace) window.history.replaceState(null, "", value);
    else window.history.pushState(null, "", value);
    if (!value.startsWith("/login")) resyncUrl();
  },
  init: (notify) => {
    const onPop = () => notify(here());
    window.addEventListener("popstate", onPop);
    return () => window.removeEventListener("popstate", onPop);
  },
});

export function App() {
  return (
    <BrowserRouter root={Root}>
      <Route path="/login" component={Login} />
      <Route path="*all" component={Shell} />
    </BrowserRouter>
  );
}
