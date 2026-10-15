// The actions the command palette offers: create a note of any type, capture, go to a page or a
// type's notes, open a settings section, and act on the workspace's panes, tabs and sidebars.

import type { TypeDef } from "../api";
import { openCapture } from "../capture/store";
import { toggleSidebar } from "../sidebars";
import { typeNavLabel } from "../sidebarTypes";
import { toggleTheme } from "../theme";
import * as M from "../workspace/model";
import { closePane, closeTab, openTab, splitPane, workspace } from "../workspace/store";
import type { ActionLike } from "./model";

export type Glyph =
  | "capture"
  | "inbox"
  | "calendar"
  | "search"
  | "trash"
  | "gear"
  | "files"
  | "tab"
  | "close"
  | "split-right"
  | "split-down"
  | "sidebar-left"
  | "sidebar-right"
  | "theme";

export type ActionIcon = { glyph: Glyph } | { type: string | null };

export interface Action extends ActionLike {
  id: string;
  section: "Create" | "Go to" | "Settings" | "Workspace";
  icon: ActionIcon;
  /** The key that does the same, to show beside it. */
  shortcut?: string;
  /** A page to open: in the active pane, or a background tab. */
  href?: string;
  /** Otherwise, what it does. */
  run?: () => void;
}

const SETTINGS: [string, string, string[]][] = [
  ["/settings", "General", ["preferences", "theme"]],
  ["/settings/types", "Types", ["schema", "fields", "templates"]],
  ["/settings/taxonomies", "Taxonomies", ["terms", "tags", "categories"]],
  ["/settings/scheduled", "Scheduled", ["tasks", "cron", "jobs", "scripts"]],
  ["/settings/secrets", "Secrets", ["keys", "passwords"]],
  ["/settings/tokens", "API tokens", ["access", "agents"]],
];

/** "New …" and "Go to …" for each type. The daily note type has no "New": each day has one. */
export function typeActions(types: readonly TypeDef[]): Action[] {
  const out: Action[] = [];
  for (const t of types) {
    const label = t.label || t.name;
    const plural = typeNavLabel(t);
    if (t.name !== "dailynote") {
      out.push({
        id: `new:${t.name}`,
        section: "Create",
        label: `New ${label}`,
        keywords: [`Create ${label}`, `Add ${label}`, `New ${plural}`, `New ${t.name}`],
        icon: { type: t.icon },
        href: `/notes/new?type=${encodeURIComponent(t.name)}`,
      });
    }
    out.push({
      id: `list:${t.name}`,
      section: "Go to",
      label: `Go to ${plural}`,
      keywords: [plural, label, t.name, `${label} list`],
      icon: { type: t.icon },
      href: `/types/${encodeURIComponent(t.name)}`,
    });
  }
  return out;
}

/** The actions that do not depend on the vault's types. `phone` drops what a phone cannot do. */
export function generalActions(opts: { phone: boolean; dark: boolean }): Action[] {
  const pane = () => M.activePane(workspace());
  const out: Action[] = [
    {
      id: "capture",
      section: "Create",
      label: "Capture",
      keywords: ["Fleeting note", "Quick note"],
      icon: { glyph: "capture" },
      shortcut: "Ctrl+Shift+K",
      run: openCapture,
    },
    { id: "inbox", section: "Go to", label: "Go to Inbox", keywords: ["Fleeting notes", "Captures"], icon: { glyph: "inbox" }, href: "/inbox" },
    { id: "daily", section: "Go to", label: "Go to Today", keywords: ["Daily notes", "Journal", "Daily note"], icon: { glyph: "calendar" }, href: "/daily" },
    { id: "search", section: "Go to", label: "Go to Search", keywords: ["Find", "Filter notes"], icon: { glyph: "search" }, href: "/search" },
    { id: "trash", section: "Go to", label: "Go to Trash", keywords: ["Deleted notes", "Restore"], icon: { glyph: "trash" }, href: "/trash" },
    { id: "assets", section: "Go to", label: "Go to Assets", keywords: ["Files", "Uploads", "Images"], icon: { glyph: "files" }, href: "/assets" },
  ];
  for (const [href, name, keywords] of SETTINGS) {
    out.push({
      id: `settings:${href}`,
      section: "Settings",
      label: name === "General" ? "Open settings" : `Settings: ${name}`,
      keywords: [`Settings ${name}`, `Open settings ${name}`, ...keywords],
      icon: { glyph: "gear" },
      href,
    });
  }
  out.push({ id: "tab:new", section: "Workspace", label: "New tab", keywords: ["Open tab"], icon: { glyph: "tab" }, run: () => openTab(M.HOME) });
  out.push({
    id: "tab:close",
    section: "Workspace",
    label: "Close tab",
    icon: { glyph: "close" },
    run: () => {
      const w = workspace();
      if (!M.isEmpty(w)) closeTab(M.activeTab(w).id);
    },
  });
  out.push({ id: "pane:close", section: "Workspace", label: "Close pane", icon: { glyph: "close" }, shortcut: "Alt+W", run: () => closePane(pane()) });
  if (!opts.phone) {
    out.push({ id: "split:right", section: "Workspace", label: "Split right", keywords: ["Split pane", "Side by side"], icon: { glyph: "split-right" }, run: () => splitPane(pane(), "right") });
    out.push({ id: "split:down", section: "Workspace", label: "Split down", keywords: ["Split pane", "Stack"], icon: { glyph: "split-down" }, run: () => splitPane(pane(), "bottom") });
  }
  out.push({
    id: "sidebar:left",
    section: "Workspace",
    label: "Toggle left sidebar",
    keywords: ["Show sidebar", "Hide sidebar", "Navigation"],
    icon: { glyph: "sidebar-left" },
    shortcut: "Ctrl+\\",
    run: () => toggleSidebar("left"),
  });
  out.push({
    id: "sidebar:right",
    section: "Workspace",
    label: "Toggle right sidebar",
    keywords: ["Properties", "History", "Show right sidebar", "Hide right sidebar"],
    icon: { glyph: "sidebar-right" },
    shortcut: "Ctrl+Shift+\\",
    run: () => toggleSidebar("right"),
  });
  out.push({
    id: "theme",
    section: "Workspace",
    label: opts.dark ? "Switch to light theme" : "Switch to dark theme",
    keywords: ["Toggle theme", "Dark mode", "Light mode"],
    icon: { glyph: "theme" },
    run: toggleTheme,
  });
  for (const a of out) a.general = true;
  return out;
}
