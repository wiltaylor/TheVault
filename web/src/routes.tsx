// The pages a pane can show. Every pane has a router of its own over these routes.

import { Navigate, Route, type RouteSectionProps } from "@solidjs/router";
import { type Component, lazy } from "solid-js";
import { DailyNotes } from "./pages/DailyNotes";
import { Inbox } from "./pages/Inbox";
import { Outbox } from "./pages/Outbox";
import { NoteHistoryRedirect } from "./history/redirect";
import { NewNote, NoteView } from "./pages/NoteView";
import { Search } from "./pages/Search";
import { MovedToScheduled, MovedToSettings, SettingsLayout } from "./pages/SettingsLayout";
import { TypeList } from "./pages/TypeList";

/**
 * A page that loads when a pane first shows it. Settings, admin and other less used pages load
 * this way, so the app starts without them; each shows nothing until its code has arrived. There
 * is no Suspense boundary: one would also hold back the page whenever a resource it reads loads.
 */
function page<M extends Record<string, unknown>>(load: () => Promise<M>, name: keyof M & string): Component<RouteSectionProps> {
  return lazy(() => load().then((m) => ({ default: m[name] as Component<RouteSectionProps> })));
}


const Secrets = page(() => import("./pages/Access"), "Secrets");
const Tokens = page(() => import("./pages/Access"), "Tokens");
const Archive = page(() => import("./pages/Archive"), "Archive");
const BrowserBox = page(() => import("./pages/BrowserBox"), "BrowserBox");
const Assets = page(() => import("./pages/Assets"), "Assets");
const AssetView = page(() => import("./pages/AssetView"), "AssetView");
const Devices = page(() => import("./pages/Devices"), "Devices");
const Log = page(() => import("./pages/Log"), "Log");
const Ai = page(() => import("./pages/Ai"), "Ai");
const AgentScreen = page(() => import("./pages/AgentScreen"), "AgentScreen");
const SkillScreen = page(() => import("./pages/SkillScreen"), "SkillScreen");
const Skills = page(() => import("./pages/Skills"), "Skills");
const GoogleCallback = page(() => import("./pages/Integrations"), "GoogleCallback");
const Integrations = page(() => import("./pages/Integrations"), "Integrations");
const Settings = page(() => import("./pages/Settings"), "Settings");
const ScheduledEditor = page(() => import("./pages/Scheduled"), "ScheduledEditor");
const ScheduledList = page(() => import("./pages/Scheduled"), "ScheduledList");
const Trash = page(() => import("./pages/Trash"), "Trash");
const TaxonomyEditor = page(() => import("./pages/TaxonomyEditor"), "TaxonomyEditor");
const TaxonomyList = page(() => import("./pages/TaxonomyEditor"), "TaxonomyList");
const TemplateEditor = page(() => import("./pages/TemplateEditor"), "TemplateEditor");
const TypeEditor = page(() => import("./pages/TypeEditor"), "TypeEditor");
const TypeGallery = page(() => import("./pages/TypeGallery"), "TypeGallery");

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
        <Route path="/integrations" component={Integrations} />
        <Route path="/ai" component={Ai} />
      </Route>
      <Route path="/integrations/google/callback" component={GoogleCallback} />
      <Route path={["/agents/:name", "/agents/:name/:section", "/agents/:name/:section/:tab"]} component={AgentScreen} />
      <Route path="/skills" component={Skills} />
      <Route path={["/skills/:name", "/skills/:name/:tab"]} component={SkillScreen} />
      <Route path="/trash" component={Trash} />
      <Route path="/archive" component={Archive} />
      <Route path="/search" component={Search} />
      <Route path="/inbox" component={Inbox} />
      <Route path="/outbox" component={Outbox} />
      <Route path="/log" component={Log} />
      <Route path="/browser-box" component={BrowserBox} />
      <Route path="*" component={() => <p class="error">There is no page here.</p>} />
    </>
  );
}
