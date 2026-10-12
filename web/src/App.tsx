import { Route, Router, useNavigate } from "@solidjs/router";
import { type ParentProps, onMount } from "solid-js";
import { setUnauthorizedHandler } from "./api";
import { Shell } from "./components/Shell";
import { Home } from "./pages/Home";
import { Login } from "./pages/Login";
import { NoteView } from "./pages/NoteView";
import { Search } from "./pages/Search";
import { TaxonomyEditor, TaxonomyList } from "./pages/TaxonomyEditor";
import { TemplateEditor } from "./pages/TemplateEditor";
import { TypeEditor } from "./pages/TypeEditor";
import { TypeList } from "./pages/TypeList";
import { refreshSession, signedOut } from "./session";

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

export function App() {
  return (
    <Router root={Root}>
      <Route path="/login" component={Login} />
      <Route path="/" component={Shell}>
        <Route path="/" component={Home} />
        <Route path="/types/new" component={TypeEditor} />
        <Route path="/types/:type" component={TypeList} />
        <Route path="/types/:type/edit" component={TypeEditor} />
        <Route path="/types/:type/templates" component={TemplateEditor} />
        <Route path="/taxonomies" component={TaxonomyList} />
        <Route path="/taxonomies/new" component={TaxonomyEditor} />
        <Route path="/taxonomies/:name" component={TaxonomyEditor} />
        <Route path="/notes/:id" component={NoteView} />
        <Route path="/search" component={Search} />
        <Route path="*" component={() => <p class="error">There is no page here.</p>} />
      </Route>
    </Router>
  );
}
