// The pages a pane can show. Every pane has a router of its own over these routes.

import { Navigate, Route } from "@solidjs/router";
import { Secrets, Tokens } from "./pages/Access";
import { Archive } from "./pages/Archive";
import { Assets } from "./pages/Assets";
import { AssetView } from "./pages/AssetView";
import { DailyNotes } from "./pages/DailyNotes";
import { Devices } from "./pages/Devices";
import { Inbox } from "./pages/Inbox";
import { NoteHistoryRedirect } from "./history/redirect";
import { NewNote, NoteView } from "./pages/NoteView";
import { Search } from "./pages/Search";
import { Settings } from "./pages/Settings";
import { MovedToScheduled, MovedToSettings, SettingsLayout } from "./pages/SettingsLayout";
import { ScheduledEditor, ScheduledList } from "./pages/Scheduled";
import { Trash } from "./pages/Trash";
import { TaxonomyEditor, TaxonomyList } from "./pages/TaxonomyEditor";
import { TemplateEditor } from "./pages/TemplateEditor";
import { TypeEditor } from "./pages/TypeEditor";
import { TypeGallery } from "./pages/TypeGallery";
import { TypeList } from "./pages/TypeList";

export function paneRoutes() {
  return (
    <>
      <Route path="/" component={() => <Navigate href="/daily" />} />
      <Route path={["/daily", "/daily/:date"]} component={DailyNotes} />
      <Route path={["/types", "/types/new", "/types/:type/edit", "/types/:type/templates"]} component={MovedToSettings} />
      <Route path="/types/:type" component={TypeList} />
      <Route path="/notes/new" component={NewNote} />
      <Route path="/notes/:id" component={NoteView} />
      <Route path="/notes/:id/history" component={NoteHistoryRedirect} />
      <Route path="/assets" component={Assets} />
      <Route path="/assets/:id" component={AssetView} />
      <Route
        path={["/tasks", "/tasks/new", "/tasks/:name", "/secrets", "/tokens", "/taxonomies", "/taxonomies/new", "/taxonomies/:name"]}
        component={MovedToSettings}
      />
      <Route
        path={["/scripts", "/scripts/:name", "/settings/scripts", "/settings/scripts/:name"]}
        component={() => <Navigate href="/settings/types" />}
      />
      <Route path="/settings" component={SettingsLayout}>
        <Route path="/" component={Settings} />
        <Route path="/types" component={TypeGallery} />
        <Route path="/types/new" component={TypeEditor} />
        <Route path="/types/:type/edit" component={TypeEditor} />
        <Route path="/types/:type/templates" component={TemplateEditor} />
        <Route path="/scheduled" component={ScheduledList} />
        <Route path="/scheduled/new" component={ScheduledEditor} />
        <Route path="/scheduled/:name" component={ScheduledEditor} />
        <Route path={["/tasks", "/tasks/new", "/tasks/:name"]} component={MovedToScheduled} />
        <Route path="/taxonomies" component={TaxonomyList} />
        <Route path="/taxonomies/new" component={TaxonomyEditor} />
        <Route path="/taxonomies/:name" component={TaxonomyEditor} />
        <Route path="/secrets" component={Secrets} />
        <Route path="/tokens" component={Tokens} />
        <Route path="/devices" component={Devices} />
      </Route>
      <Route path="/trash" component={Trash} />
      <Route path="/archive" component={Archive} />
      <Route path="/search" component={Search} />
      <Route path="/inbox" component={Inbox} />
      <Route path="*" component={() => <p class="error">There is no page here.</p>} />
    </>
  );
}
