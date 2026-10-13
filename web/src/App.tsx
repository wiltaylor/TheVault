import { Route, Router, useNavigate } from "@solidjs/router";
import { type ParentProps, onMount } from "solid-js";
import { setUnauthorizedHandler } from "./api";
import { Shell } from "./components/Shell";
import { Secrets, Tokens } from "./pages/Access";
import { Assets } from "./pages/Assets";
import { NoteHistory, Trash } from "./pages/History";
import { Home } from "./pages/Home";
import { Login } from "./pages/Login";
import { NewNote, NoteView } from "./pages/NoteView";
import { ScriptEditor, ScriptList } from "./pages/Scripts";
import { Search } from "./pages/Search";
import { Settings } from "./pages/Settings";
import { MovedToSettings, SettingsLayout } from "./pages/SettingsLayout";
import { TaskEditor, TaskList } from "./pages/Tasks";
import { TaxonomyEditor, TaxonomyList } from "./pages/TaxonomyEditor";
import { TemplateEditor } from "./pages/TemplateEditor";
import { TypeEditor } from "./pages/TypeEditor";
import { TypeGallery } from "./pages/TypeGallery";
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
        <Route path="/types" component={TypeGallery} />
        <Route path="/types/new" component={TypeEditor} />
        <Route path="/types/:type" component={TypeList} />
        <Route path="/types/:type/edit" component={TypeEditor} />
        <Route path="/types/:type/templates" component={TemplateEditor} />
        <Route path="/notes/new" component={NewNote} />
        <Route path="/notes/:id" component={NoteView} />
        <Route path="/notes/:id/history" component={NoteHistory} />
        <Route path="/assets" component={Assets} />
        <Route path={["/scripts", "/scripts/:name", "/tasks", "/tasks/new", "/tasks/:name", "/secrets", "/tokens", "/taxonomies", "/taxonomies/new", "/taxonomies/:name"]} component={MovedToSettings} />
        <Route path="/settings" component={SettingsLayout}>
          <Route path="/" component={Settings} />
          <Route path="/scripts" component={ScriptList} />
          <Route path="/scripts/:name" component={ScriptEditor} />
          <Route path="/tasks" component={TaskList} />
          <Route path="/tasks/new" component={TaskEditor} />
          <Route path="/tasks/:name" component={TaskEditor} />
          <Route path="/taxonomies" component={TaxonomyList} />
          <Route path="/taxonomies/new" component={TaxonomyEditor} />
          <Route path="/taxonomies/:name" component={TaxonomyEditor} />
          <Route path="/secrets" component={Secrets} />
          <Route path="/tokens" component={Tokens} />
        </Route>
        <Route path="/trash" component={Trash} />
        <Route path="/search" component={Search} />
        <Route path="*" component={() => <p class="error">There is no page here.</p>} />
      </Route>
    </Router>
  );
}
