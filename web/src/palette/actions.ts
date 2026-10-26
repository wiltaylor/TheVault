// The actions the command palette offers: create a note of any type, capture, go to a page or a
// type's notes, open a settings section, and act on the workspace's panes, tabs and sidebars.

import { periodOfType } from "../planner/period";
import { AGENT_ICON, agentLabel, setAgentsOpen, setCreatingAgent } from "../agents";
import type { AgentSummary, SkillSummary, TypeDef } from "../api";
import type { ActiveNote } from "../archive";
import { openCapture } from "../capture/store";
import { focusPanel, setLeftDrawer, toggleSidebar } from "../sidebars";
import { SKILLS_HREF, SKILL_ICON, setCreatingSkill, skillHref } from "../skills";
import { typeNavLabel } from "../sidebarTypes";
import { toggleTheme } from "../theme";
import * as M from "../workspace/model";
import { closePane, closeTab, navigatePane, openTab, splitPane, workspace } from "../workspace/store";
import type { ActionLike } from "./model";

export type Glyph =
  | "capture"
  | "inbox"
  | "calendar"
  | "search"
  | "trash"
  | "archive"
  | "unarchive"
  | "gear"
  | "files"
  | "tab"
  | "close"
  | "split-right"
  | "split-down"
  | "sidebar-left"
  | "sidebar-right"
  | "theme"
  | "ai";

export type ActionIcon = { glyph: Glyph } | { type: string | null };

export interface Action extends ActionLike {
  id: string;
  section: "Create" | "Go to" | "Note" | "Settings" | "Workspace";
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
  ["/settings/tools", "Tools", ["script tools", "wscript", "agent tools"]],
  ["/settings/secrets", "Secrets", ["keys", "passwords"]],
  ["/settings/tokens", "API tokens", ["access", "agents"]],
  ["/settings/ai", "AI", ["AI accounts", "API keys", "models", "Sign in with ChatGPT", "usage"]],
];

/** "New …" and "Go to …" for each type. The planner's types have no "New": each period has one note. */
export function typeActions(types: readonly TypeDef[]): Action[] {
  const out: Action[] = [];
  for (const t of types) {
    const label = t.label || t.name;
    const plural = typeNavLabel(t);
    if (!periodOfType(t.name)) {
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

/** What can be done to the note the active pane shows: archive it, or take it out of the archive. */
export function noteActions(note: ActiveNote | null): Action[] {
  if (!note) return [];
  const action: Action = note.archived
    ? {
        id: "note:unarchive",
        section: "Note",
        label: "Unarchive this note",
        keywords: ["Restore from archive", "Take out of the archive"],
        icon: { glyph: "unarchive" },
        run: () => void note.toggle(),
      }
    : {
        id: "note:archive",
        section: "Note",
        label: "Archive this note",
        keywords: ["Archive note", "Hide note", "Put away"],
        icon: { glyph: "archive" },
        run: () => void note.toggle(),
      };
  action.general = true;
  return [action];
}

/**
 * The actions that do not depend on the vault's types. `phone` drops what a phone cannot do;
 * `androidApp` adds what only the Android app can do, on the phone it runs on. Ask AI is in every
 * client (ai D23).
 */
export function generalActions(opts: { phone: boolean; dark: boolean; androidApp?: boolean }): Action[] {
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
    { id: "daily", section: "Go to", label: "Go to Today", keywords: ["Daily notes", "Journal", "Daily note"], icon: { glyph: "calendar" }, href: "/planner/day" },
    {
      id: "planner",
      section: "Go to",
      label: "Go to Planner",
      keywords: ["Weekly note", "Monthly note", "Quarterly note", "Yearly note", "Calendar"],
      icon: { glyph: "calendar" },
      href: "/planner",
    },
    { id: "search", section: "Go to", label: "Go to Search", keywords: ["Find", "Filter notes"], icon: { glyph: "search" }, href: "/search" },
    { id: "trash", section: "Go to", label: "Go to Trash", keywords: ["Deleted notes", "Restore"], icon: { glyph: "trash" }, href: "/trash" },
    { id: "archive", section: "Go to", label: "Go to Archive", keywords: ["Archived notes", "Unarchive"], icon: { glyph: "archive" }, href: "/archive" },
    { id: "assets", section: "Go to", label: "Go to Assets", keywords: ["Files", "Uploads", "Images"], icon: { glyph: "files" }, href: "/assets" },
    {
      id: "ai:ask",
      section: "Go to",
      label: "Ask AI",
      keywords: ["AI chat", "Assistant", "Chat about notes"],
      icon: { glyph: "ai" },
      shortcut: "Ctrl+Shift+L",
      run: () => focusPanel("ai"),
    },
  ];
  if (opts.androidApp) {
    out.push({
      id: "device:import",
      section: "Settings",
      label: "Import messages and calls",
      keywords: ["Import from phone", "Import SMS", "Call log", "Re-import", "Check for new messages"],
      icon: { glyph: "inbox" },
      href: "/settings/devices#this-device",
    });
  }
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

/** Show the left sidebar with its Agents section open, in the drawer on a narrow screen. */
function showAgentsSection() {
  if (window.matchMedia?.("(max-width: 1024px)").matches) setLeftDrawer(true);
  else toggleSidebar("left", true);
  setAgentsOpen(true);
}

/**
 * "New agent", in the web app only, and "Open agent …" for each agent (agents G3). New agent
 * opens the left sidebar's name box for it, in the drawer on a narrow screen. With `skills`,
 * "New skill" (the web app only), which opens the Skills page with its name box shown, and
 * "Open skill …" for each skill (skills S4).
 */
export function agentActions(list: readonly AgentSummary[], readOnly: boolean, skills: readonly SkillSummary[] = []): Action[] {
  const out: Action[] = [];
  if (!readOnly) {
    out.push({
      id: "agent:new",
      section: "Create",
      label: "New agent",
      keywords: ["Create agent", "Add agent", "AI agent"],
      icon: { type: AGENT_ICON },
      run: () => {
        showAgentsSection();
        setCreatingAgent(true);
      },
    });
    // The browser box's desktop in the active pane (browser B4); the web app only.
    out.push({
      id: "browser-box",
      section: "Go to",
      label: "Open browser box",
      keywords: ["Browser box", "Desktop", "VNC", "Watch agent", "playwright"],
      icon: { type: "monitor" },
      href: "/browser-box",
    });
  }
  for (const a of list) {
    const label = agentLabel(a);
    out.push({
      id: `agent:${a.name}`,
      section: "Go to",
      label: `Open agent ${label}`,
      keywords: [label, a.name, `${label} agent`, "Agent"],
      icon: { type: a.icon || AGENT_ICON },
      href: `/agents/${encodeURIComponent(a.name)}`,
    });
  }
  if (!readOnly) {
    out.push({
      id: "skill:new",
      section: "Create",
      label: "New skill",
      keywords: ["Create skill", "Add skill", "Agent skill", "SKILL.md"],
      icon: { type: SKILL_ICON },
      run: () => {
        setCreatingSkill(true);
        navigatePane(M.activePane(workspace()), SKILLS_HREF);
      },
    });
  }
  for (const s of skills) {
    out.push({
      id: `skill:${s.name}`,
      section: "Go to",
      label: `Open skill ${s.name}`,
      keywords: [s.name, `${s.name} skill`, "Skill", ...(s.description ? [s.description.slice(0, 80)] : [])],
      icon: { type: s.valid ? SKILL_ICON : "triangle-alert" },
      href: skillHref(s.name),
    });
  }
  return out;
}
