// The typed client of TheVault's HTTP API. Every UI action goes through one of these calls.
// The browser authenticates with the session cookie that POST /api/login sets (D11).

import { noteWritten } from "./changes";
import type { SchemaType } from "./defs/typeModel";

export type Json = null | boolean | number | string | Json[] | { [key: string]: Json };

/** A content block in its JSON form: the block's kind, labels, items and nested blocks. */
export interface Block {
  kind: string;
  labels?: string[];
  fields?: Record<string, Json>;
  blocks?: Block[];
}

/** A note's JSON form: its fields by name and its compound fields as labelled blocks. */
export interface NoteDoc {
  fields: Record<string, Json>;
  blocks: Block[];
}

export interface Note {
  id: string;
  type: string;
  version: string;
  wcl: string;
  note: NoteDoc;
}

export interface FieldDef {
  name: string;
  label: string;
  description: string | null;
  kind: string;
  required: boolean;
  computed: boolean;
  /** Kept off the note page and the properties sidebar; still stored, queried and scripted. */
  hidden?: boolean;
  multiple: boolean;
  compound: boolean;
  default?: Json;
  options?: string[];
  /** `select`: one label per option, index-aligned; an empty label shows the value. */
  option_labels?: string[];
  /** `date_range`: whether the ends are dates or datetimes. */
  of?: "date" | "datetime" | "date_or_datetime";
  taxonomy?: string;
  target?: string;
  relation?: "one_way" | "two_way" | "peer";
  inverse?: string | null;
  /** `list`: the item kind. On a multiple `taxonomy` field, `"taxonomy"` says it was declared as a list of terms. */
  items?: string;
  values?: string;
  /** `location`: the image an image map's pins sit on, null on a real-world map. */
  image?: string | null;
  /** A list of strings: the field holding one label per value, in the same order. */
  labels?: string | null;
  /** `asset`: `"image"` when the field takes only images. */
  accept?: "any" | "image";
  /** Attributes this version does not know, by name, as WCL source text; the editors write them back as they were. */
  extra?: Record<string, string>;
}

export type TemplateItem = { slot: string; label: string | null; extra?: Record<string, string> } | { block: Block };

export interface Tab {
  title: string;
  /** The condition, in a query's `where` syntax, under which the tab shows (src/when.ts). */
  when?: string | null;
  items: TemplateItem[];
  /** Attributes this version does not know, by name, as WCL source text; the editors write them back as they were. */
  extra?: Record<string, string>;
}

export interface Template {
  name: string;
  label: string | null;
  tabs: Tab[];
  /** Attributes this version does not know, by name, as WCL source text; the editors write them back as they were. */
  extra?: Record<string, string>;
}

export interface TypeDef {
  name: string;
  label: string;
  plural: string;
  description: string | null;
  icon: string | null;
  colour: string | null;
  /** A built-in type (`dailynote`, `fleeting`): it can be customised but not deleted. */
  builtin?: boolean;
  fields: FieldDef[];
  templates: Template[];
  /** What this version ignores in the type file, such as an attribute only a newer release knows. */
  warnings?: SchemaIssue[];
  /** The type file's `list_page` block, each item only when the block sets it (lists L1). */
  list_page?: ListPage;
}

/** How a type's list page shows its notes: `view`, `sort` as `"<field> asc|desc"`, `page_size`. */
export interface ListPage {
  view?: string;
  sort?: string;
  page_size?: number;
}

/** One schema issue: where in the file, and what. A warning leaves the file valid. */
export interface SchemaIssue {
  at: string;
  message: string;
  severity: "error" | "warning";
}

export interface Term {
  name: string;
  path: string;
  colour: string | null;
  description: string | null;
  children: Term[];
}

export interface Taxonomy {
  name: string;
  label: string;
  description: string | null;
  /** Open: a note saved with a term the taxonomy lacks adds it. */
  allow_new?: boolean;
  terms: Term[];
}

/** A planned type change (D6): what it adds, removes and renames, and every note it edits. */
export interface TypePlan {
  type: string;
  applied: boolean;
  plan: string;
  wcl: string;
  added: string[];
  removed: string[];
  renamed: { from: string; to: string }[];
  changes: { id: string; title: string; version: string; edits: { op: string; [key: string]: Json }[] }[];
  flagged: string[];
}

/** Which notes a listing reads by archive state. Listings leave archived notes out by default. */
export type ArchivedFilter = "exclude" | "include" | "only";

/** Whether a note's fields mark it archived (`archived = true`). */
export function isArchived(fields: Record<string, Json> | undefined): boolean {
  return fields?.archived === true;
}

/** The id of the note a sync conflict copy was made from, from its `conflict_of` field (D20). */
export function conflictOf(fields: Record<string, Json> | undefined): string | undefined {
  const id = fields?.conflict_of;
  return typeof id === "string" && id !== "" ? id : undefined;
}

export interface Row {
  id: string;
  type: string;
  title: string;
  fields: Record<string, Json>;
}

/** An uploaded file, stored at `assets/<id>/<filename>` (D22). */
export interface Asset {
  id: string;
  filename: string;
  size: number;
  path: string;
}

/** What an asset is, by its file name. */
export type AssetKind = "image" | "pdf" | "audio" | "video" | "other";

/** An asset in the library listing: with its kind, upload time and how much uses it. */
export interface AssetSummary extends Asset {
  kind: AssetKind;
  content_type: string;
  /** When it was uploaded, from its ULID; null for an id that does not read as one. */
  uploaded_ms: number | null;
  /** Notes, trashed notes and types using it, all told. */
  used_by: number;
  notes: number;
  trashed: number;
  types: number;
}

/** The library filters: by kind, or only what nothing uses. */
export type AssetFilter = "all" | "image" | "pdf" | "other" | "unused";

export interface AssetQuery {
  filter?: AssetFilter;
  q?: string;
  /** `uploaded`, `name`, `size`, `kind` or `used`. */
  sort?: string;
  dir?: "asc" | "desc";
  cursor?: string | null;
  limit?: number;
}

export interface AssetPage {
  assets: AssetSummary[];
  /** Assets matching, over every page. */
  total: number;
  next: string | null;
  /** The assets nothing uses, in the whole library. */
  unused: { count: number; size: number };
  counts: Record<"all" | "image" | "pdf" | "other" | "unused", number>;
}

/** A way a note names an asset: `field` is the asset field, or the content field holding the block. */
export interface AssetHow {
  field: string;
  /** `field` for an asset field, else the block kind: `image`, `asset` or `node`. */
  via: string;
}

/** An asset with everything that uses it. */
export interface AssetUsage extends Omit<AssetSummary, "notes" | "trashed" | "types"> {
  notes: { id: string; type: string; title: string; archived: boolean; how: AssetHow[] }[];
  trashed: { id: string; type: string; title: string }[];
  types: string[];
}

/** What a cleanup deleted, or a dry run would. */
export interface AssetCleanup {
  dry_run: boolean;
  count: number;
  size: number;
  assets: Asset[];
  purge_days: number;
}

/** A deleted asset that can still be restored. */
export interface TrashedAsset extends Asset {
  deleted_ms: number;
  purge_ms: number;
}

/** A note pinned to the sidebar. */
export interface Pin {
  id: string;
  type: string;
  title: string;
}

/** A pinned note in the sidebar's tree, with the entries nested under it. */
export interface PinNoteItem extends Pin {
  kind: "note";
  children: PinItem[];
}

/** A folder in the sidebar's pinned tree. */
export interface PinFolderItem {
  kind: "folder";
  id: string;
  name: string;
  children: PinItem[];
}

export type PinItem = PinNoteItem | PinFolderItem;

/** A message for the user from a script, an agent or the server, shown under the bell. */
export interface AppNotification {
  id: string;
  /** Milliseconds since the Unix epoch. */
  created: number;
  title: string;
  body: string | null;
  /** The note it opens, with that note's type and title as they are now (null once it is gone). */
  note: string | null;
  note_type: string | null;
  note_title: string | null;
  /** The page of the app it opens when it names no note. */
  url: string | null;
  /** `task:<name>`, `handler:<type>.<event>`, `token:<name>`, `api` or `system`. */
  source: string;
  /** When it was read, or null while unread. */
  read: number | null;
}

export interface NotificationPage {
  items: AppNotification[];
  next: string | null;
  unread: number;
}

/** A stored pinned entry, as the folder and move endpoints return it. */
export interface PinEntry {
  kind: "note" | "folder";
  id: string;
  name?: string;
  parent: string | null;
  index: number;
}

export interface Page {
  rows: Row[];
  next: string | null;
  total: number;
}

/** Who wrote a version or deleted a note: the owner's session, a token, a handler's script, a task or an agent. */
export interface Author {
  kind: "session" | "token" | "handler" | "task" | "agent";
  name: string;
}

/** One stored version of a note, newest first in a history (D13). */
export interface Version {
  version: string;
  type: string;
  author: Author;
  at: number;
  current: boolean;
}

/** A deleted note waiting in the trash, with the WCL it had (D13). */
export interface TrashEntry {
  id: string;
  type: string;
  version: string;
  wcl: string;
  deleted_by: Author;
  deleted_at: number;
}

/** What emptying the trash, or purging one note from it, removed (ai D21). */
export interface TrashPurged {
  /** The notes purged, with their history. */
  purged: string[];
  /** The assets only those notes used, now in the asset trash. */
  assets_trashed: string[];
}

/** Why a script would not compile when the vault runs it in one role. */
export interface ScriptProblem {
  role: "before_handler" | "after_handler" | "task" | "device";
  message: string;
}

export interface Script {
  name: string;
  source: string;
  problems: ScriptProblem[];
}

/** One run of a scheduled task. Times are milliseconds since the epoch (D10). */
export interface TaskRun {
  id: number;
  task: string;
  attempt: number;
  started: number;
  ended: number | null;
  outcome: "running" | "succeeded" | "failed";
  message: string | null;
  log: string;
}

/** A scheduled task: its file's items, when it runs next, and how its latest run went. */
export interface Task {
  name: string;
  next_run: number | null;
  retry: { attempt: number; at: number } | null;
  running: boolean;
  failing: boolean;
  latest_run: TaskRun | null;
  /** Why the task file does not check, when it does not. The items below are then absent. */
  error?: string;
  label?: string;
  description?: string | null;
  schedule?: string;
  /** The schedule in words, with its time zone: "Weekdays at 07:00 (Australia/Melbourne)". */
  schedule_text?: string;
  timezone?: string;
  script?: string;
  function?: string;
  allow_domains?: string[];
  secrets?: string[];
  read_paths?: string[];
  write_paths?: string[];
  retries?: number;
  retry_delay_ms?: number;
  timeout_ms?: number;
  paused?: boolean;
  wcl?: string;
}

/** One line of `GET /api/agents/{name}/threads` (agents G7). */
export interface AgentThreadSummary {
  id: string;
  title: string;
  /** `user`, or `trigger:<name>` for a thread a trigger started. */
  origin?: string | null;
  last_activity?: string | null;
  /** How many proposals wait for the user. */
  pending: number;
  /** Whether the thread note is archived (agents G15). */
  archived?: boolean;
  /** Whether a turn runs on the thread now (agents G17). */
  running?: boolean;
  /** The running turn's id in the turns form; null for a trigger's run. */
  turn?: string | null;
  /** When the running turn started, in milliseconds since the epoch. */
  started_ms?: number | null;
}

/** A proposal of a thread and where it stands (agents G8). */
export interface StoredProposal {
  /** `proposal`, `type_proposal`, `taxonomy_proposal`, `agent_proposal`, or `applied` for a change the agent made. */
  event: string;
  data: Record<string, unknown>;
  status: "pending" | "applied" | "discarded" | "failed" | "stale" | "made";
  result?: Record<string, unknown> | null;
  error?: string | null;
}

/** A thread with its transcript (agents G7, G8). */
export interface AgentThread extends AgentThreadSummary {
  agent: string;
  version: string;
  /** The neutral transcript (ai D10). */
  messages?: { role: "user" | "assistant"; content: Json[]; provider_state?: Json }[];
  /** Each tool step's summary, by call id. */
  steps?: Record<string, { summary?: Record<string, unknown>; is_error?: boolean }>;
  proposals?: StoredProposal[];
}

/** One line of `GET /api/agents` (agents G2). An agent whose file does not check has `error`. */
export interface AgentSummary {
  name: string;
  label: string;
  description?: string | null;
  icon?: string | null;
  provider?: AiProvider | null;
  model?: string | null;
  enabled?: boolean;
  triggers?: number;
  /**
   * What a note may call the agent by (agents G16): `null` when it is disabled or has no mention
   * trigger, else the types whose notes may call it, `types: null` for every type.
   */
  mention?: { types: string[] | null } | null;
  built_in: boolean;
  error?: string;
  /** How many turns run on the agent's threads now (agents G17). */
  running?: number;
}

export interface AgentList {
  agents: AgentSummary[];
  /** Every tool `permissions.tools` may name. */
  tools: string[];
}

/** An agent's own permissions (agents G4). `null` tools or types means all of them. */
export interface AgentPermissions {
  tools: string[] | null;
  types: string[] | null;
  confirm: AiConfirm;
  web: AiWeb;
}

/**
 * An agent's trigger, as the routes answer it and take it back (agents G9, G10, G16). A mention
 * trigger's prompt may be empty: the instruction from the note is then the whole first message.
 */
export type AgentTrigger =
  | { name: string; kind: "schedule"; schedule: string; timezone: string; prompt: string }
  | { name: string; kind: "event"; on: string[]; type: string[]; where: string | null; prompt: string }
  | { name: string; kind: "mention"; type: string[]; prompt: string };

export type AgentEffort = "low" | "medium" | "high" | "xhigh" | "max";

/** An agent as `PUT /api/agents/{name}` takes it in its JSON form. */
export interface AgentForm {
  label: string;
  description: string | null;
  icon: string | null;
  enabled: boolean;
  provider: AiProvider | null;
  model: string | null;
  effort: AgentEffort | null;
  system_prompt: string | null;
  permissions: AgentPermissions;
  triggers: AgentTrigger[];
}

/** An agent file, parsed, with its WCL (agents G1). */
export interface Agent extends AgentForm {
  name: string;
  built_in: boolean;
  wcl: string;
}

/** One run of an agent trigger (agents G12), times in milliseconds since the epoch. */
export interface AgentRun {
  id: number;
  /** The thread the run wrote, once it had one. */
  thread: string | null;
  started: number;
  ended: number | null;
  outcome: "running" | "succeeded" | "failed";
  message: string | null;
}

/** An agent's trigger runs: the tools a trigger run is offered, and each trigger's last runs. */
/** `GET /api/agents/calls/{call}` (agents G19). */
export interface AgentCallInfo {
  agent: string;
  note: string;
  thread: string | null;
  running: boolean;
}

export interface AgentRuns {
  tools: string[];
  triggers: { name: string; kind: AgentTrigger["kind"]; runs: AgentRun[] }[];
}

/** A cron schedule in words and the next times it fires, as the scheduler reads it. */
export interface CronPreview {
  schedule: string;
  timezone: string;
  text: string;
  next: number[];
}

/** One side of the Google sync, Gmail or Calendar, as Settings > Integrations shows it. */
export interface GoogleSide {
  /** The scheduled task that runs it: `google_gmail` or `google_calendar`. */
  task: string;
  /** Whether the task file exists. */
  installed: boolean;
  /** Whether the task is paused, or null when it does not exist or does not check. */
  paused: boolean | null;
  /** Whether a run of the task is going now, from the scheduler. */
  running: boolean;
  /** When the sync last ran, as its state secret records it: RFC 3339 in UTC, ending Z. */
  last_run: string | null;
  last_error: string | null;
  /** The last line the latest finished run logged, such as "Gmail: 0 new, 0 relabelled, 0 archived, 0 not saved." */
  summary: string | null;
}

/** What the Google sync pulls, kept in the `google_settings` secret (google-sync D21). */
export interface GoogleSettings {
  /** `labels` pulls the labels in `gmail_labels`; `all` pulls all mail. */
  gmail_mode: "labels" | "all";
  /** Gmail label names. */
  gmail_labels: string[];
  /** How many days back the first pull reaches, 1 to 3650. */
  gmail_window_days: number;
  /** A date, `YYYY-MM-DD`, older mail is paged back to, or null. */
  gmail_backfill_to: string | null;
  /** The ids of the Google calendars to pull; never empty. */
  calendars: string[];
  /** The calendar a new event note goes to when it names none. */
  default_calendar: string;
}

/** The Google sync's connection (google-sync D1, D18, D21). */
export interface GoogleStatus {
  /** Whether the client id, client secret and refresh token are all stored. */
  connected: boolean;
  /** Whether the OAuth client's id and secret are stored. */
  client_stored: boolean;
  /** Whether the connection granted gmail.send, so the Outbox can send email (outbox X4). */
  can_send?: boolean;
  /** The connected address, from the `google_account` secret. */
  account: string | null;
  /** The settings, the defaults where none are saved. */
  settings: GoogleSettings;
  gmail: GoogleSide;
  calendar: GoogleSide;
}

/** How Google's consent page sends the browser back: to a loopback address to paste, or straight to this server. */
export type GoogleRedirect = "loopback" | "direct";

/** An AI provider (ai D1): an Anthropic key, an OpenAI key, or a ChatGPT plan. */
export type AiProvider = "anthropic" | "openai" | "chatgpt";

/** The API an OpenAI-compatible service speaks (ai D25). */
export type OpenAiApi = "responses" | "chat_completions";

/** One provider in Settings > AI. Never a key or token (ai D7). */
export interface AiProviderStatus {
  /** Whether it has a key or a custom API URL, or for ChatGPT a sign-in. */
  configured: boolean;
  /** The default model chosen for it. */
  model: string | null;
  /** The signed-in ChatGPT account, for `chatgpt`. */
  account?: string | null;
  /** The custom API URL, for `anthropic` and `openai`; null means the official API (ai D24). */
  url?: string | null;
  /** Whether an API key is stored, for `anthropic` and `openai`: with a custom URL it is optional. */
  has_key?: boolean;
  /** For `openai`: the API its calls speak, always `responses` without a custom URL (ai D25). */
  api?: OpenAiApi;
}

/** Settings > AI, from `GET /api/integrations/ai`. */
export interface AiStatus {
  /** Whether the server can store secrets at all. */
  secrets_key: boolean;
  default_provider: AiProvider | null;
  providers: Record<AiProvider, AiProviderStatus>;
  /** The chat panel's system prompt in use (ai D17). */
  system_prompt: string;
  /** The shipped system prompt, used when none is set. */
  default_system_prompt: string;
  /** Whether `system_prompt` is the user's own. */
  system_prompt_custom: boolean;
  /** Which kinds of chat change wait for the user; false lets the AI make them (ai D22). */
  confirm: AiConfirm;
  /** Which internet tools the chat gets (ai D28). */
  web: AiWeb;
}

/** Settings > AI's internet tool switches (ai D28), every key filled; true gives the chat the tool. */
export interface AiWeb {
  /** `web_search`, which also needs a Brave Search key. */
  search: boolean;
  /** `fetch_url`, reading web pages. */
  read: boolean;
  /** `download_asset`, downloading files into the vault. */
  download: boolean;
}

/** Settings > AI's confirmation switches (ai D22), every key filled. */
export interface AiConfirm {
  /** Creating and editing notes. */
  notes: boolean;
  /** Archiving and unarchiving notes. */
  archive: boolean;
  /** Moving notes to the trash; nothing empties it. */
  delete: boolean;
  /** Changing and creating types. */
  types: boolean;
  /** Downloading files from the web into the vault as assets (ai D27). */
  downloads: boolean;
  /** Changing, creating and deleting taxonomies (ai D31). */
  taxonomies: boolean;
  /** Changing, creating and deleting agents (agents G20). */
  agents: boolean;
}

/** A model a provider offers now (ai D6). */
export interface AiModel {
  id: string;
  display_name: string | null;
}

/** A provider's live model list, with its default model or the preselected one. */
export interface AiModels {
  provider: AiProvider;
  model: string | null;
  models: AiModel[];
}

/** The chat panel's picker, from `GET /api/ai/providers` (ai D23): no account, never a key. */
export interface AiProviders {
  default_provider: AiProvider | null;
  providers: Record<AiProvider, { configured: boolean; model: string | null }>;
  confirm: AiConfirm;
  /** The built-in Assistant's internet switches (agents G6). */
  web?: AiWeb;
}

/** What `POST /api/ai/apply` did (ai D23): a note's change with its new version, or a type's. */
export type AiApplied =
  | { op: "update" | "create" | "archive" | "unarchive" | "delete"; note_id: string; version: string }
  | { op: "type_change" | "type_create"; type_name: string }
  | { op: "taxonomy_change" | "taxonomy_create" | "taxonomy_delete"; taxonomy_name: string; notes: number }
  | { op: "agent_change" | "agent_create" | "agent_delete"; agent_name: string }
  | { op: "download"; asset_id: string; filename: string; media_type?: string; size?: number; url?: string };

/** A poll of a chat turn's events through an app's embedded server (ai D23). */
/** A turn started in the turns form (ai D23, agents G17). */
export interface AiTurnStarted {
  turn: string;
  /** When it started, in milliseconds since the epoch. */
  started_ms?: number;
}

export interface AiTurnPoll {
  events: StreamEvent[];
  /** How many events the turn has sent so far: the next poll's `after`. */
  next: number;
  /** Whether the turn's stream ended. */
  done: boolean;
}

/** What the Test button got back. */
export interface AiTestResult {
  ok: boolean;
  provider: AiProvider;
  model: string | null;
  ms: number;
  text: string | null;
  /** The OpenAI API the Test went over, which Test detects and saves (ai D25); null otherwise. */
  api?: OpenAiApi | null;
  error: { kind: string; message: string } | null;
}

/** The Brave Search card's state (ai D26): whether the API key is stored, never the key. */
export interface BraveStatus {
  configured: boolean;
}

/** How the Brave Search card's Test went: the number of results for "TheVault", or Brave's error. */
export interface BraveTestResult {
  ok: boolean;
  count: number | null;
  ms: number;
  error: { kind: string; message: string } | null;
}

/** A secret's name and times. No response ever carries its value (D10). */
export interface Secret {
  name: string;
  created: number;
  updated: number;
}

export type Scope = "read" | "write" | "admin";

/** A one-time install code for an AI agent: redeeming it makes the token `name` with `scope`. */
export interface AgentCode {
  code: string;
  expires: number;
  /** The token redeeming it makes, `agent-<name>`. */
  name: string;
  scope: Scope;
  server_url: string;
  /** `curl -fsSL <server>/agent/install.sh | sh -s -- <code>`. */
  command: string;
}

/** An API token. Only the response that creates it carries `secret` (D11). */
export interface Token {
  name: string;
  scope: Scope;
  created: number;
  last_used: number | null;
  secret?: string;
}

/** A contact, number or thread a device never captures (D25). */
export interface DeviceExclusion {
  kind: "contact" | "number" | "thread";
  value: string;
}

/** A paired device's settings, synced to the device (D22, D23, D25). */
export interface DeviceSettings {
  /** The commands server scripts may send the device. */
  commands: string[];
  contacts_write: boolean;
  sms_send: boolean;
  exclusions: DeviceExclusion[];
  /** A regular expression; an SMS whose body matches it is not captured. */
  skip_pattern: string;
}

/** A paired desktop or Android app (D2, D3). */
export interface Device {
  id: string;
  name: string;
  platform: "linux" | "android";
  capabilities: string[];
  /** The name of the device's token. */
  token: string;
  /** Whether the device registered a push endpoint. */
  push: boolean;
  last_seen: number | null;
  created: number;
  settings: DeviceSettings;
  /** The capture health the phone last reported with its settings sync, or null before its first report (ticket 91). */
  health?: DeviceHealth | null;
}

/** What a phone reports of its capture health (ticket 91). */
export interface DeviceHealth {
  /** What stops capture, in words; empty when all is well. */
  problems: string[];
  kinds?: { kind: string; label?: string; state: string }[];
  battery_optimised?: boolean;
  background?: { work: string; stale: boolean }[];
  /** When the home server heard the report, ms since the epoch. */
  reported?: number | null;
}

/** A command queued for a device, and what became of it (D22). */
export interface DeviceCommand {
  id: string;
  device: string;
  name: string;
  payload: unknown;
  status: "pending" | "delivered" | "done" | "failed" | "expired";
  /** What the device reported: its progress while `delivered`, its result once finished. */
  result: unknown;
  source: string;
}

/** A `devices/<platform>.wcl` as `GET /api/device-files` lists it. */
export interface DeviceFileInfo {
  platform: Device["platform"];
  commands?: { name: string; script: string; function: string }[];
  /** Why the file does not parse. */
  error?: string;
}

/** An app build the server hands out at `/api/clients`, from its clients folder's manifest. */
export interface ClientBuild {
  platform: "android" | "linux";
  kind: "apk" | "appimage" | "deb" | "rpm";
  arch: string;
  version: string;
  build: "release" | "debug";
  size: number;
  sha256: string;
  filename: string;
  published: number;
  /** Where to download it, relative to the server. */
  path: string;
  /** The absolute download address, as this request reached the server. */
  url?: string;
  /** An APK's QR code of `url`, as an SVG document. */
  qr_svg?: string;
}

/** A single-use pairing code, with the QR code an app scans to pair (D2). */
export interface PairCode {
  code: string;
  expires: number;
  server_url: string;
  payload: { server_url: string; code: string };
  /** The QR code of `payload` as an SVG document. */
  qr_svg: string;
}

/** A response other than 2xx, with the API's JSON error body when it sent one. */
/** How a neighbour in a note's graph links to the note: it is linked to, links to it, or both. */
export type GraphDirection = "center" | "out" | "in" | "both";

export interface GraphNode {
  id: string;
  type: string;
  title: string;
  updated: string | null;
  direction: GraphDirection;
}

/** A link between two notes of a graph: through a reference `field`, or a `mention` in it. */
export interface GraphEdge {
  from: string;
  to: string;
  via: "field" | "mention";
  field: string;
}

/** A note's link neighbourhood. `nodes[0]` is the note; `truncated` counts neighbours left out. */
export interface NoteGraph {
  id: string;
  nodes: GraphNode[];
  edges: GraphEdge[];
  truncated: number;
}

export class ApiError extends Error {
  constructor(
    readonly status: number,
    readonly body: { error?: string; message?: string; [key: string]: unknown },
  ) {
    super(body.message ?? `request failed with ${status}`);
  }

  /** A save whose base version is no longer the note's current version (D12). */
  get isConflict(): boolean {
    return this.status === 409 && this.body.error === "conflict";
  }

  /**
   * In an app: what was asked is the home server's, and the app cannot reach it (`offline`) or is
   * not paired with one (`not_paired`). The message says which.
   */
  get needsServer(): boolean {
    return this.status === 503 && (this.body.error === "offline" || this.body.error === "not_paired");
  }

  /** In an app: the app's device token may not do this; the web app can (D2). */
  get needsWebApp(): boolean {
    return this.status === 403 && this.body.error === "needs_web_app";
  }
}

/**
 * The app a page runs in, from its embedded server's `GET /api/app` (D18): whether it is paired,
 * and which device it is. The home server has no such route.
 */
export interface AppInfo {
  app: true;
  paired: boolean;
  device_id: string | null;
  device_name: string | null;
  server_url: string | null;
  /** The app's version, from its `tauri.conf.json`; null from an app too old to say. */
  version?: string | null;
  /** `linux-appimage`, `linux-package` (deb or rpm), `linux` or `android`. */
  platform?: string | null;
  /** `release` or `debug`. */
  build?: string | null;
  /** The app's last fresh start: it joined another vault and moved its old local vault aside (ticket 143). */
  fresh_start?: FreshStart | null;
}

/** A fresh start of an app's local vault, after it joined a different vault (ticket 143). */
export interface FreshStart {
  /** When, in milliseconds since the epoch. */
  at_ms: number;
  /** The folder on the device the old local vault was moved to. */
  previous: string;
  /** The vault the old local vault was a copy of, if it was known. */
  from_vault: string | null;
  /** The vault the device joined. */
  to_vault: string;
}

/** Where an app's sync stands, from its embedded server's `GET /api/app/sync` (D19, D20, D27). */
export interface AppSync {
  /** What the sidebar's sync icon shows: green, yellow or red. */
  state: "synced" | "syncing" | "error";
  /** Where the app's sync loop stands, in more detail. */
  phase:
    | "unpaired"
    | "starting"
    | "syncing"
    | "synced"
    | "offline"
    | "update_required"
    | "updating"
    | "revoked"
    | "error";
  /** Local changes waiting for the home server. */
  queued: number;
  /** Of those, the notes held back until an asset they name reaches the server (ticket 141). */
  held?: number;
  /** Of those, the notes the server refused in the last round, each with why; tried again later (ticket 143). */
  refused_notes?: RefusedNote[];
  /** Assets that failed to reach the server and wait for a later round. */
  assets_waiting?: number;
  /** Those assets, each with why its last try failed. */
  stuck_assets?: StuckAsset[];
  /** When a round last succeeded, in milliseconds since the epoch. */
  last_ok_ms: number | null;
  /** When the last round ended, succeeded or not. */
  last_attempt_ms?: number | null;
  /** Why the last round failed, or what a round that finished could not do, such as an asset that would not upload. */
  last_error: string | null;
  /** The home server's address, once paired. */
  server: string | null;
  /** The version gate's warning while sync goes on: the server is newer than the app and the app cannot update to it (D27). */
  warning?: string | null;
  /** How far the current or the last round got; null before any round ran (ticket 144). */
  progress?: SyncProgress | null;
}

/** A sync round's progress, from `GET /api/app/sync` (ticket 144). */
export interface SyncProgress {
  stage: "handshake" | "pushing" | "pulling" | "fetching" | "done" | "failed";
  /** Whether the round is still running. */
  running: boolean;
  /** Feed entries, notes and definition files, pulled out of those the round pulls. */
  notes_done: number;
  notes_total: number;
  /** Asset files fetched out of those the round fetches. */
  files_done: number;
  files_total: number;
  /** Asset bytes received, out of the sizes of the files the round fetches. */
  bytes_done: number;
  bytes_total: number;
  /** About how many seconds are left; null until there is a rate to go by. */
  eta_secs: number | null;
  elapsed_ms: number;
  /** Whether the round pulls more than 200 changes or fetches more than 20 MB: the popover shows its progress. */
  large: boolean;
}

/** A note an app could not send: the home server refused it (ticket 143). */
export interface RefusedNote {
  note_id: string;
  reason: string;
}

/** An asset an app stored that has not reached the home server: its last upload failed (ticket 141). */
export interface StuckAsset {
  id: string;
  filename: string;
  size: number;
  /** Why the last try failed. */
  error: string;
  /** The tries that failed in a row. */
  attempts: number;
  /** When the last try failed, in milliseconds since the epoch. */
  last_ms: number;
  /** When a round tries it again. */
  next_ms: number;
}

/** A device event that failed for good or was skipped, from the app's embedded server (ticket 88). */
export interface CaptureProblem {
  id: number;
  /** The device event, such as `sms_received` or `call_ended`. */
  event: string;
  /** When it happened on the phone, in milliseconds since the epoch. */
  at: number;
  status: "failed" | "skipped" | "queued" | "done" | "dismissed";
  attempts: number;
  /** The handler's error, or why it was skipped. */
  message: string | null;
  /** The contact, sender or number the event names, if any. */
  who: string | null;
}

/** `POST /api/app/sync/adopt`: the local notes queued for the server, oldest first, and whether a round was asked for. */
export interface SentDeviceNotes {
  queued: number;
  notes: string[];
  /** Local assets the server lacked, marked to be sent with them. */
  assets: number;
  syncing: boolean;
}

/** `GET /api/app/device-events`: whether a device runtime runs here, the failed count, and the list newest first. */
export interface CaptureProblems {
  device: boolean;
  failed: number;
  events: CaptureProblem[];
}

/** Where a note's history came from: the home server's, or in an app offline, the app's own. */
export type HistorySource = "server" | "device";

let onUnauthorized: () => void = () => {};

/** Called when a request other than login answers 401, so the app can return to the login page. */
export function setUnauthorizedHandler(handler: () => void) {
  onUnauthorized = handler;
}

/** Requests other than GET to these paths change notes. */
const NOTE_WRITES = /^\/api\/(notes|batch|upsert|trash)(\/|\?|$)/;

/** The Tauri IPC entry point the app gives its pages. */
interface AppIpc {
  invoke<T>(cmd: string, args?: Record<string, unknown>): Promise<T>;
}

/** The embedded server's answer to a request sent over IPC. */
interface IpcAnswer {
  status: number;
  headers: [string, string][];
  body: string;
}

/**
 * The IPC to send request bodies through, inside the Android app only. Its webview drops the body
 * of every request on the app's custom scheme, so the app's page script sets
 * `__thevaultApiOverIpc` and its `api_request` command answers instead
 * (apps/android/src-tauri/src/bridge.rs). In a browser and the desktop app this is null.
 */
function appIpc(): AppIpc | null {
  const w = globalThis as { __thevaultApiOverIpc?: boolean; __TAURI_INTERNALS__?: AppIpc };
  return w.__thevaultApiOverIpc && w.__TAURI_INTERNALS__ ? w.__TAURI_INTERNALS__ : null;
}

/** Bytes per IPC chunk of a body of bytes, before base64. */
const IPC_CHUNK = 1024 * 1024;

function base64(blob: Blob): Promise<string> {
  return new Promise((resolve, reject) => {
    const reader = new FileReader();
    reader.onload = () => {
      const url = String(reader.result);
      resolve(url.slice(url.indexOf(",") + 1));
    };
    reader.onerror = () => reject(reader.error);
    reader.readAsDataURL(blob);
  });
}

/** `fetch`, except that in the Android app a request with a body goes through IPC. */
async function send(path: string, init: RequestInit): Promise<Response> {
  const ipc = appIpc();
  if (!ipc || init.body == null) return fetch(path, init);
  init.signal?.throwIfAborted();
  const headers = Object.entries((init.headers ?? {}) as Record<string, string>);
  let text: string | undefined;
  let upload: number | undefined;
  if (typeof init.body === "string") {
    text = init.body;
  } else if (init.body instanceof Blob) {
    const blob = init.body;
    upload = await ipc.invoke<number>("api_upload_start");
    for (let at = 0; at < blob.size; at += IPC_CHUNK) {
      init.signal?.throwIfAborted();
      const chunk = await base64(blob.slice(at, at + IPC_CHUNK));
      await ipc.invoke("api_upload_append", { id: upload, chunk });
    }
  } else {
    throw new TypeError("only a text or Blob body can be sent over IPC");
  }
  const answer = await ipc.invoke<IpcAnswer>("api_request", {
    request: { method: init.method ?? "GET", path, headers, text, upload },
  });
  const empty = answer.status === 204 || answer.status === 205 || answer.status === 304;
  return new Response(empty ? null : answer.body, { status: answer.status, headers: answer.headers });
}

async function request<T>(method: string, path: string, body?: unknown, signal?: AbortSignal): Promise<T> {
  const init: RequestInit = {
    method,
    credentials: "same-origin",
    headers: { Accept: "application/json" },
    ...(signal ? { signal } : {}),
  };
  if (body instanceof Blob) {
    init.headers = { ...init.headers, "Content-Type": "application/octet-stream" };
    init.body = body;
  } else if (body !== undefined) {
    init.headers = { ...init.headers, "Content-Type": "application/json" };
    init.body = JSON.stringify(body);
  }
  const res = await send(path, init);
  if (res.status === 204) {
    if (method !== "GET" && NOTE_WRITES.test(path)) noteWritten();
    return undefined as T;
  }
  const text = await res.text();
  let parsed: unknown = null;
  try {
    parsed = text ? JSON.parse(text) : null;
  } catch {
    parsed = { message: text };
  }
  if (!res.ok) {
    if (res.status === 401 && path !== "/api/login") {
      onUnauthorized();
    }
    const errBody = parsed && typeof parsed === "object" ? (parsed as ApiError["body"]) : {};
    throw new ApiError(res.status, errBody);
  }
  if (method !== "GET" && NOTE_WRITES.test(path)) noteWritten();
  return parsed as T;
}

/** One server-sent event: its `event` name and its `data`, parsed as JSON. */
export interface StreamEvent {
  event: string;
  data: unknown;
}

/**
 * A reader of server-sent event frames from text arriving in chunks: `feed` takes the next chunk
 * and returns the events it completed. Comments (heartbeats) are skipped.
 */
export function sseParser(): { feed: (chunk: string) => StreamEvent[] } {
  let buffer = "";
  return {
    feed(chunk: string) {
      buffer += chunk.replace(/\r\n?/g, "\n");
      const out: StreamEvent[] = [];
      let end: number;
      while ((end = buffer.indexOf("\n\n")) >= 0) {
        const frame = buffer.slice(0, end);
        buffer = buffer.slice(end + 2);
        let event = "message";
        const data: string[] = [];
        for (const line of frame.split("\n")) {
          if (line.startsWith(":")) continue;
          const colon = line.indexOf(":");
          const field = colon < 0 ? line : line.slice(0, colon);
          const value = colon < 0 ? "" : line.slice(colon + 1).replace(/^ /, "");
          if (field === "event") event = value;
          else if (field === "data") data.push(value);
        }
        if (data.length === 0) continue;
        const text = data.join("\n");
        let parsed: unknown = text;
        try {
          parsed = JSON.parse(text);
        } catch {
          // Not JSON: hand over the text.
        }
        out.push({ event, data: parsed });
      }
      return out;
    },
  };
}

/** Throw the ApiError a failed streaming answer carries. */
async function streamFailed(res: Response): Promise<never> {
  const text = await res.text().catch(() => "");
  let parsed: unknown = null;
  try {
    parsed = text ? JSON.parse(text) : null;
  } catch {
    parsed = { message: text };
  }
  if (res.status === 401) onUnauthorized();
  throw new ApiError(res.status, parsed && typeof parsed === "object" ? (parsed as ApiError["body"]) : {});
}

/** Hand each server-sent event of `res` to `onEvent` as it arrives, until the stream ends. */
async function readStream(res: Response, onEvent: (e: StreamEvent) => void): Promise<void> {
  if (!res.ok || !res.body) await streamFailed(res);
  const reader = res.body!.getReader();
  const decoder = new TextDecoder();
  const parser = sseParser();
  try {
    for (;;) {
      const { value, done } = await reader.read();
      if (done) break;
      for (const e of parser.feed(decoder.decode(value, { stream: true }))) onEvent(e);
    }
  } finally {
    reader.releaseLock();
  }
}

/**
 * POST `body` to `path` and hand each server-sent event of the answer to `onEvent` as it
 * arrives, reading the body with a stream reader (EventSource cannot POST). Resolves when the
 * stream ends; aborting `signal` stops it. An answer that is not a stream throws an ApiError.
 */
export async function streamEvents(
  path: string,
  body: unknown,
  onEvent: (e: StreamEvent) => void,
  signal?: AbortSignal,
): Promise<void> {
  const res = await fetch(path, {
    method: "POST",
    credentials: "same-origin",
    headers: { Accept: "text/event-stream", "Content-Type": "application/json" },
    body: JSON.stringify(body),
    ...(signal ? { signal } : {}),
  });
  await readStream(res, onEvent);
}

/**
 * Read chat turn `turn`'s events after the first `after` (agents G17), handing each to `onEvent`
 * as it arrives: those it sent so far, then the rest as they come. Resolves when the stream ends;
 * aborting `signal` stops reading, and the turn runs on. A turn whose events are no longer kept
 * throws an ApiError, 410 or 404.
 */
export async function streamTurn(turn: string, after: number, onEvent: (e: StreamEvent) => void, signal?: AbortSignal): Promise<void> {
  const res = await fetch(`/api/ai/chat/turns/${encodeURIComponent(turn)}/events?after=${after}`, {
    method: "GET",
    credentials: "same-origin",
    headers: { Accept: "text/event-stream" },
    ...(signal ? { signal } : {}),
  });
  await readStream(res, onEvent);
}

const enc = encodeURIComponent;

export const api = {
  login: (password: string) => request<{ expires_in: number }>("POST", "/api/login", { password }),
  logout: () => request<void>("POST", "/api/logout"),
  types: async () => (await request<{ types: TypeDef[] }>("GET", "/api/types")).types,
  note: (id: string) => request<Note>("GET", `/api/notes/${enc(id)}`),
  /** The daily note for a `YYYY-MM-DD` day; a day with no note is a 404 `ApiError`. */
  daily: (date: string) => request<Note>("GET", `/api/daily/${enc(date)}`),
  /** The daily note for a day, created when the day has none. Opening it twice gives one note. */
  openDaily: (date: string) => request<Note>("POST", `/api/daily/${enc(date)}`),
  createNote: (type: string, note: NoteDoc) =>
    request<Note>("POST", `/api/notes?type=${enc(type)}`, { note }),
  /** Save `note` over version `base`. A stale base is an ApiError whose `isConflict` is true. */
  updateNote: (id: string, note: NoteDoc, base: string) =>
    request<Note>("PUT", `/api/notes/${enc(id)}`, { note, base_version: base }),
  /** Every type as schema inspection describes it, with handlers and reference rules. */
  schema: () => request<{ types: SchemaType[]; taxonomies: Taxonomy[] }>("GET", "/api/schema"),
  /** Create a type, or replace one that has no notes. */
  putType: (name: string, wcl: string) => request<{ name: string; wcl: string }>("PUT", `/api/types/${enc(name)}`, { wcl }),
  /** A type file as stored. A type that does not exist is an `ApiError` with `unknown_type`. */
  readType: (name: string) => request<{ name: string; wcl: string }>("GET", `/api/types/${enc(name)}`),
  /** Plan a change to a type, or apply the plan whose token is `plan`. */
  changeType: (
    name: string,
    wcl: string,
    renames: Record<string, string>,
    plan?: string,
    migration?: { script: string; function?: string },
  ) =>
    request<TypePlan>("POST", `/api/types/${enc(name)}/change`, {
      wcl,
      renames,
      ...(plan ? { plan } : {}),
      ...(migration ? { migration } : {}),
    }),
  /** Delete a type. A type that still has notes answers 409 `type_in_use` with their `count`. */
  deleteType: (name: string) => request<unknown>("DELETE", `/api/types/${enc(name)}`),
  putTemplate: (type: string, name: string, wcl: string) =>
    request<{ wcl: string }>("PUT", `/api/types/${enc(type)}/templates/${enc(name)}`, { wcl }),
  deleteTemplate: (type: string, name: string) =>
    request<unknown>("DELETE", `/api/types/${enc(type)}/templates/${enc(name)}`),
  taxonomies: async () => (await request<{ taxonomies: Taxonomy[] }>("GET", "/api/taxonomies")).taxonomies,
  /** Create or replace a taxonomy. `renames` maps old term paths to new ones. */
  putTaxonomy: (name: string, wcl: string, renames: Record<string, string>) =>
    request<Taxonomy & { notes: { id: string }[] }>("PUT", `/api/taxonomies/${enc(name)}`, { wcl, renames }),
  /** Delete a taxonomy. One a field uses answers 409 `taxonomy_in_use`, listing the notes. */
  deleteTaxonomy: (name: string) => request<unknown>("DELETE", `/api/taxonomies/${enc(name)}`),
  /** One page of `query`. `thisId` is the note that holds the query, bound to `this`; `signal` cancels it. */
  query: (query: string, cursor: string | null, pageSize: number, thisId?: string, signal?: AbortSignal) =>
    request<Page>(
      "POST",
      "/api/query",
      {
        query,
        ...(thisId ? { this: thisId } : {}),
        ...(cursor ? { cursor } : {}),
        page_size: pageSize,
      },
      signal,
    ),
  /** Check `query`. A query that does not parse is a 400 naming the line and column. */
  parseQuery: (query: string) =>
    request<{ text: string }>("POST", "/api/query/parse", {
      query,
    }),
  /** Full-text search. Archived notes are left out unless `archived` is `include` or `only`. */
  search: (q: string, cursor: string | null, pageSize: number, archived?: ArchivedFilter) => {
    const params = new URLSearchParams({ q, page_size: String(pageSize) });
    if (cursor) params.set("cursor", cursor);
    if (archived && archived !== "exclude") params.set("archived", archived);
    return request<Page>("GET", `/api/search?${params}`);
  },
  /** Move a note to the trash, over version `base`. A stale base is a conflict. */
  deleteNote: (id: string, base: string) =>
    request<unknown>("DELETE", `/api/notes/${enc(id)}?base_version=${enc(base)}`),
  /** Archive a note over version `base`: a new version holding `archived = true`. */
  archiveNote: (id: string, base: string) =>
    request<Note>("POST", `/api/notes/${enc(id)}/archive?base_version=${enc(base)}`),
  /** Take a note out of the archive over version `base`. */
  unarchiveNote: (id: string, base: string) =>
    request<Note>("POST", `/api/notes/${enc(id)}/unarchive?base_version=${enc(base)}`),
  /** The notes linked to or from note `id`, with every link among them. */
  graph: (id: string, limit?: number) =>
    request<NoteGraph>("GET", `/api/notes/${enc(id)}/graph${limit ? `?limit=${limit}` : ""}`),
  history: async (id: string) =>
    (await request<{ versions: Version[] }>("GET", `/api/notes/${enc(id)}/history`)).versions,
  /**
   * A note's history and where it came from. In an app the embedded server answers with the home
   * server's history, or offline with the versions the app itself recorded (`source: "device"`).
   */
  historyWithSource: async (id: string) => {
    const page = await request<{ versions: Version[]; source?: HistorySource }>("GET", `/api/notes/${enc(id)}/history`);
    return { versions: page.versions, source: page.source ?? "server" };
  },
  /** A stored version of a note, in the same form as the note itself. */
  version: (id: string, version: string) =>
    request<Note & { author: Author; at: number }>("GET", `/api/notes/${enc(id)}/history/${enc(version)}`),
  /** Make `version` the note's current content again, over current version `base`. */
  restoreVersion: (id: string, version: string, base: string) =>
    request<Note>("POST", `/api/notes/${enc(id)}/restore`, { version, base_version: base }),
  /** The sidebar's pinned tree. Pins live on the server, shared by every device. */
  /** A page of notifications, newest first, with the unread count. */
  notifications: (opts: { limit?: number; cursor?: string | null; unread?: boolean } = {}) => {
    const q = new URLSearchParams();
    if (opts.limit) q.set("limit", String(opts.limit));
    if (opts.cursor) q.set("cursor", opts.cursor);
    if (opts.unread) q.set("unread", "true");
    const qs = q.toString();
    return request<NotificationPage>("GET", `/api/notifications${qs ? `?${qs}` : ""}`);
  },
  readNotification: (id: string) =>
    request<AppNotification & { unread: number }>("POST", `/api/notifications/${enc(id)}/read`),
  readAllNotifications: () => request<{ marked: number; unread: number }>("POST", "/api/notifications/read-all"),
  dismissNotification: (id: string) =>
    request<{ id: string; deleted: boolean; unread: number }>("DELETE", `/api/notifications/${enc(id)}`),
  /** Remove every read notification. */
  clearReadNotifications: () => request<{ cleared: number; unread: number }>("DELETE", "/api/notifications"),
  pins: async () => (await request<{ pins: PinItem[] }>("GET", "/api/pins")).pins,
  pin: (id: string) => request<Pin & { added: boolean }>("PUT", `/api/pins/${enc(id)}`),
  /** Unpin a note; the entries nested under it move up into its place. */
  unpin: (id: string) => request<unknown>("DELETE", `/api/pins/${enc(id)}`),
  createPinFolder: (name: string, parent: string | null = null) =>
    request<PinEntry>("POST", "/api/pins/folders", { name, parent }),
  renamePinFolder: (id: string, name: string) => request<PinEntry>("PUT", `/api/pins/folders/${enc(id)}`, { name }),
  /** Delete a folder; its entries move up into its place. */
  deletePinFolder: (id: string) => request<unknown>("DELETE", `/api/pins/folders/${enc(id)}`),
  /** Move a pinned entry under `parent` (null for the top level) to `index` among its children. */
  movePin: (id: string, parent: string | null, index: number) =>
    request<PinEntry>("POST", "/api/pins/move", { id, parent, index }),
  trash: async () => (await request<{ notes: TrashEntry[] }>("GET", "/api/trash")).notes,
  restoreFromTrash: (id: string) => request<Note>("POST", `/api/trash/${enc(id)}/restore`),
  /** Purge every note in the trash with its history, for good. A signed-in session only (ai D21). */
  emptyTrash: () => request<TrashPurged>("POST", "/api/trash/empty"),
  /** Purge note `id` from the trash with its history, for good. A signed-in session only (ai D21). */
  purgeFromTrash: (id: string) => request<TrashPurged>("DELETE", `/api/trash/${enc(id)}`),
  scripts: async () => (await request<{ scripts: string[] }>("GET", "/api/scripts")).scripts,
  script: (name: string) => request<Script>("GET", `/api/scripts/${enc(name)}`),
  /** Store a script as sent. The answer says why it would not compile, if it would not. */
  putScript: (name: string, source: string) => request<Script>("PUT", `/api/scripts/${enc(name)}`, { source }),
  deleteScript: (name: string) => request<unknown>("DELETE", `/api/scripts/${enc(name)}`),
  tasks: async () => (await request<{ tasks: Task[] }>("GET", "/api/tasks")).tasks,
  task: (name: string) => request<Task>("GET", `/api/tasks/${enc(name)}`),
  putTask: (name: string, wcl: string) => request<Task>("PUT", `/api/tasks/${enc(name)}`, { wcl }),
  deleteTask: (name: string) => request<unknown>("DELETE", `/api/tasks/${enc(name)}`),
  /** Every agent's summary, Assistant first, and every tool an agent may be allowed (agents G2). */
  agents: () => request<AgentList>("GET", "/api/agents"),
  /** One agent: its parsed definition and its WCL. */
  agent: (name: string) => request<Agent>("GET", `/api/agents/${enc(name)}`),
  /**
   * Create or replace an agent from the form (`Agent` without `wcl`) or from its WCL. A file that
   * does not check is a 400 with its issues; disabling Assistant is a 400 `built_in`. Needs
   * `admin`, so the apps answer 403 `needs_web_app`.
   */
  putAgent: (name: string, body: Partial<AgentForm> | { wcl: string }) => request<Agent>("PUT", `/api/agents/${enc(name)}`, body),
  /** Delete an agent; its threads stay. Assistant is a 400 `built_in`. */
  deleteAgent: (name: string) => request<unknown>("DELETE", `/api/agents/${enc(name)}`),
  /** The last runs of each of the agent's triggers, and the tools a trigger run is offered (agents G12). */
  agentRuns: (name: string) => request<AgentRuns>("GET", `/api/agents/${enc(name)}/runs`),
  /**
   * What became of a call of an agent committed in a note (agents G19): its thread, null for a
   * call the server did not run, and whether the thread runs. 404 while the server has not seen it.
   */
  agentCall: (call: string) => request<AgentCallInfo>("GET", `/api/agents/calls/${enc(call)}`),
  /** The agent's threads, newest activity first (agents G7); archived ones only with `archived` (agents G15). */
  agentThreads: (name: string, archived = false) =>
    request<{ threads: AgentThreadSummary[] }>("GET", `/api/agents/${enc(name)}/threads${archived ? "?archived=true" : ""}`),
  /** Start a thread with the agent. */
  agentThreadStart: (name: string, title?: string) =>
    request<AgentThread>("POST", `/api/agents/${enc(name)}/threads`, title ? { title } : {}),
  /** One thread with its transcript, its tool steps and its proposals (agents G8). */
  agentThread: (name: string, id: string) => request<AgentThread>("GET", `/api/agents/${enc(name)}/threads/${enc(id)}`),
  /** Rename a thread. */
  renameAgentThread: (name: string, id: string, title: string) =>
    request<AgentThread>("PATCH", `/api/agents/${enc(name)}/threads/${enc(id)}`, { title }),
  /** Move a thread to the trash. */
  deleteAgentThread: (name: string, id: string) => request<unknown>("DELETE", `/api/agents/${enc(name)}/threads/${enc(id)}`),
  taskRuns: async (name: string, limit = 50) =>
    (await request<{ runs: TaskRun[] }>("GET", `/api/tasks/${enc(name)}/runs?limit=${limit}`)).runs,
  /**
   * Start a task now, paused or not. Answers as soon as the run starts, with the run still
   * `running`; [`waitForRun`] reads it until it ends.
   */
  runTask: (name: string) => request<TaskRun>("POST", `/api/tasks/${enc(name)}/run`),
  /** One run of a task, with its whole log. */
  taskRun: (name: string, id: number) => request<TaskRun>("GET", `/api/tasks/${enc(name)}/runs/${id}`),
  /** Describe a cron schedule and list the next `count` times it fires after now. */
  cron: (schedule: string, timezone: string, count = 5) =>
    request<CronPreview>(
      "GET",
      `/api/cron?schedule=${enc(schedule)}&timezone=${enc(timezone)}&count=${count}`,
    ),
  secrets: async () => (await request<{ secrets: Secret[] }>("GET", "/api/secrets")).secrets,
  /** Store or replace a secret's value. It is never read back. */
  putSecret: (name: string, value: string) => request<Secret>("PUT", `/api/secrets/${enc(name)}`, { value }),
  deleteSecret: (name: string) => request<unknown>("DELETE", `/api/secrets/${enc(name)}`),
  /** The Google sync's connection and each side's last run. */
  googleStatus: () => request<GoogleStatus>("GET", "/api/integrations/google"),
  /** Store the OAuth client and get Google's consent page address. */
  googleStart: (client_id: string, client_secret: string, redirect: GoogleRedirect) =>
    request<{ auth_url: string }>("POST", "/api/integrations/google/start", { client_id, client_secret, redirect }),
  /** Finish signing in with the address Google sent the browser to. */
  googleFinish: (url: string) => request<GoogleStatus>("POST", "/api/integrations/google/finish", { url }),
  /** Delete the Google secrets and pause the sync. */
  googleDisconnect: () => request<GoogleStatus>("POST", "/api/integrations/google/disconnect"),
  /** Save what the Google sync pulls. */
  googleSaveSettings: (settings: GoogleSettings) =>
    request<GoogleStatus>("PUT", "/api/integrations/google/settings", settings),
  /** Whether the Brave Search API key is stored (ai D26). */
  braveStatus: () => request<BraveStatus>("GET", "/api/integrations/brave"),
  /** Store the Brave Search API key; an empty key deletes it. */
  bravePutKey: (key: string) => request<BraveStatus>("PUT", "/api/integrations/brave/key", { key }),
  /** Search Brave once for "TheVault" with the stored key. */
  braveTest: () => request<BraveTestResult>("POST", "/api/integrations/brave/test"),
  /** Settings > AI: each provider's sign-in and default model (ai D7). */
  aiStatus: () => request<AiStatus>("GET", "/api/integrations/ai"),
  /**
   * Save the default provider, default models, chat system prompt, confirmation switches and
   * API URLs, and the internet tool switches (`web`, partial like `confirm`, ai D28); an empty string clears one, which for the system prompt means the default and for
   * a URL the official API (ai D24), and for `openai_api` Responses (ai D25). `confirm` may be
   * partial. A URL that is not http or https, or holds a user name or password, is a 400
   * `invalid_url`.
   */
  aiSaveSettings: (settings: {
    default_provider?: AiProvider | "";
    models?: Partial<Record<AiProvider, string>>;
    system_prompt?: string;
    confirm?: Partial<AiConfirm>;
    web?: Partial<AiWeb>;
    urls?: Partial<Record<"anthropic" | "openai", string>>;
    openai_api?: OpenAiApi | "";
  }) =>
    request<AiStatus>("PUT", "/api/integrations/ai/settings", settings),
  /** Store an Anthropic or OpenAI API key; an empty key deletes it. */
  aiPutKey: (provider: "anthropic" | "openai", key: string) =>
    request<AiStatus>("PUT", `/api/integrations/ai/${provider}/key`, { key }),
  /** Send a short test request to the provider's default model, or `model`. */
  aiTest: (provider: AiProvider, model?: string) =>
    request<AiTestResult>("POST", `/api/integrations/ai/${provider}/test`, model ? { model } : {}),
  /** The providers set up, for the chat panel's picker in every client (ai D23). */
  aiProviders: () => request<AiProviders>("GET", "/api/ai/providers"),
  /**
   * Apply a proposal or type proposal as the chat sent it, on the home server, at its base
   * version (ai D23). A stale one is a 409; a type change without admin a 403 `needs_admin`.
   */
  aiApply: (proposal: unknown) => request<AiApplied>("POST", "/api/ai/apply", proposal),
  /** Record that the user discarded a thread's proposal (agents G8). */
  aiDiscard: (id: string, thread: string) => request<{ thread: string; pending: number }>("POST", "/api/ai/discard", { id, thread }),
  /** Start a chat turn in the turns form: its id (ai D23). */
  aiTurnStart: (body: unknown) => request<AiTurnStarted>("POST", "/api/ai/chat/turns", body),
  /** In an app: the turn's events after the first `after`, waiting briefly for one. */
  aiTurnPoll: (turn: string, after: number, signal?: AbortSignal) =>
    request<AiTurnPoll>("GET", `/api/app/ai/turns/${enc(turn)}?after=${after}`, undefined, signal),
  /** Stop a chat turn. */
  aiTurnStop: (turn: string) => request<void>("DELETE", `/api/ai/chat/turns/${enc(turn)}`),
  /** The models a provider offers now. */
  aiModels: (provider: AiProvider) => request<AiModels>("GET", `/api/ai/models?provider=${enc(provider)}`),
  /** Start Sign in with ChatGPT: the authorize page to open (ai D8). */
  chatgptStart: () => request<{ auth_url: string }>("POST", "/api/integrations/ai/chatgpt/start"),
  /** Finish Sign in with ChatGPT with the address the browser failed to load. */
  chatgptFinish: (url: string) => request<AiStatus>("POST", "/api/integrations/ai/chatgpt/finish", { url }),
  /** Revoke and forget the ChatGPT sign-in. */
  chatgptDisconnect: () => request<AiStatus>("POST", "/api/integrations/ai/chatgpt/disconnect"),
  devices: async () => (await request<{ devices: Device[] }>("GET", "/api/devices")).devices,
  /** The app builds the server hands out. Public: it needs no session. */
  clients: () => request<{ clients: ClientBuild[]; enabled: boolean }>("GET", "/api/clients"),
  /** The home server's version. In an app the embedded server asks the home server. */
  serverVersion: async () => (await request<{ server_version: string }>("GET", "/api/version")).server_version,
  /** The app this page runs in, or null in a browser: the home server answers 404. */
  app: async (): Promise<AppInfo | null> => {
    try {
      const info = await request<AppInfo>("GET", "/api/app");
      return info?.app === true ? info : null;
    } catch (e) {
      if (e instanceof ApiError && e.status === 404) return null;
      throw e;
    }
  },
  /** In an app: where its sync stands. */
  appSync: () => request<AppSync>("GET", "/api/app/sync"),
  /** In an app: run a sync round now. */
  appSyncNow: () => request<AppSync>("POST", "/api/app/sync"),
  /**
   * In an app: queue every note this device holds that the server never had, as a create, and ask
   * for a round to send them ("Send this device's notes to the server", ticket 150).
   */
  sendDeviceNotes: () => request<SentDeviceNotes>("POST", "/api/app/sync/adopt"),
  /** In the Android app: the device events that failed or were skipped. */
  captureProblems: () => request<CaptureProblems>("GET", "/api/app/device-events"),
  /** In the Android app: give a failed device event three more tries, the first now. Answers it as it then stands. */
  retryCaptureProblem: (id: number) =>
    request<CaptureProblem & { failed: number }>("POST", `/api/app/device-events/${id}/retry`),
  /** In the Android app: hide a failed device event. */
  dismissCaptureProblem: (id: number) =>
    request<{ id: number; status: "dismissed"; failed: number }>("POST", `/api/app/device-events/${id}/dismiss`),
  /** A pairing code whose QR payload names `serverUrl`, the address this page was loaded from. */
  createPairCode: (serverUrl: string) =>
    request<PairCode>("POST", "/api/devices/pair-codes", { server_url: serverUrl }),
  /** Rename a device or change some of its settings. */
  updateDevice: (id: string, change: { name?: string; settings?: Partial<DeviceSettings> }) =>
    request<Device>("PATCH", `/api/devices/${enc(id)}`, change),
  /** Revoke a device: its token stops working at once. */
  revokeDevice: (id: string) => request<void>("DELETE", `/api/devices/${enc(id)}`),
  /** The commands queued for a device, oldest first. */
  deviceCommands: async (id: string) =>
    (await request<{ commands: DeviceCommand[] }>("GET", `/api/devices/${enc(id)}/commands`)).commands,
  /** Queue a command the device's platform file declares and the device has enabled. */
  queueDeviceCommand: (id: string, name: string, payload: unknown) =>
    request<DeviceCommand>("POST", `/api/devices/${enc(id)}/commands`, { name, payload }),
  /** Every device file, with the commands each declares. */
  deviceFiles: async () => (await request<{ files: DeviceFileInfo[] }>("GET", "/api/device-files")).files,
  tokens: async () => (await request<{ tokens: Token[] }>("GET", "/api/tokens")).tokens,
  /** Create a token. The answer is the only time its secret is shown. */
  createToken: (name: string, scope: Scope) => request<Token>("POST", "/api/tokens", { name, scope }),
  revokeToken: (name: string) => request<void>("DELETE", `/api/tokens/${enc(name)}`),
  /** Issue a one-time install code for an AI agent, valid for ten minutes. */
  createAgentCode: (name: string, scope: Scope, serverUrl: string) =>
    request<AgentCode>("POST", "/api/agent-codes", { name, scope, server_url: serverUrl }),
  assets: async () => (await request<{ assets: Asset[] }>("GET", "/api/assets")).assets,
  /** One page of the asset library, filtered, searched and sorted by the server. */
  assetLibrary: (q: AssetQuery) => {
    const p = new URLSearchParams();
    const f = q.filter ?? "all";
    if (f === "unused") p.set("unused", "true");
    else if (f !== "all") p.set("kind", f);
    if (q.q?.trim()) p.set("q", q.q.trim());
    p.set("sort", q.sort ?? "uploaded");
    if (q.dir) p.set("dir", q.dir);
    if (q.cursor) p.set("cursor", q.cursor);
    if (q.limit) p.set("limit", String(q.limit));
    return request<AssetPage>("GET", `/api/assets?${p}`);
  },
  /** Asset `id`, with every note, trashed note and type that uses it. */
  assetUsage: (id: string) => request<AssetUsage>("GET", `/api/assets/${enc(id)}/usage`),
  /** Give asset `id` a new file name. Notes name it by id, so none changes. */
  renameAsset: (id: string, filename: string) => request<Asset>("PATCH", `/api/assets/${enc(id)}`, { filename }),
  /** Delete the assets nothing uses (only those of `ids`, when given) into the asset trash. */
  cleanupAssets: (dryRun: boolean, ids?: string[]) =>
    request<AssetCleanup>("POST", "/api/assets/cleanup", { dry_run: dryRun, ...(ids ? { ids } : {}) }),
  assetTrash: async () => (await request<{ assets: TrashedAsset[] }>("GET", "/api/assets/trash")).assets,
  restoreAssets: (ids: string[]) =>
    request<{ restored: Asset[]; failed: { id: string; error: string }[] }>("POST", "/api/assets/trash/restore", { ids }),
  /** Upload `file` as a new asset. The body is the file's bytes, unchanged. */
  uploadAsset: (file: File) =>
    request<Asset>("POST", `/api/assets?filename=${enc(file.name)}`, file),
  /**
   * Delete an asset. Notes naming it answer 409 `asset_in_use`, listing them, unless `unlink` is
   * set: then the asset is taken out of those notes in the same commit.
   */
  deleteAsset: (id: string, unlink = false) =>
    request<Asset & { notes: { id: string; type: string; version: string }[] }>(
      "DELETE",
      `/api/assets/${enc(id)}${unlink ? "?unlink=true" : ""}`,
    ),
};

/** A note named in a refusal because it uses what was to be deleted. */
export interface UsingNote {
  id: string;
  type: string;
  title: string;
  /** The note is in the trash: restoring it would bring the reference back. */
  trashed?: boolean;
}

/** The notes a 409 "in use" refusal lists, or none for any other error. */
export function usingNotes(e: unknown): UsingNote[] {
  if (!(e instanceof ApiError) || e.status !== 409) return [];
  const notes = e.body.notes;
  if (!Array.isArray(notes)) return [];
  return notes.filter((n): n is UsingNote => typeof n === "object" && n !== null && typeof n.id === "string");
}

/** Where the browser fetches an asset's bytes. */
export function assetUrl(id: string): string {
  return `/api/assets/${enc(id)}`;
}


/** How often [`waitForRun`] reads a run that is still going. */
export const RUN_POLL_MS = 1000;

/**
 * Run `run` once it has ended, reading it again with `read` every `everyMs` while it is
 * `running`. A run that already ended comes back as it is. When `stop` says so, the run comes
 * back as last read, which may still be running.
 */
export async function waitForRun(
  run: TaskRun,
  read: (id: number) => Promise<TaskRun>,
  everyMs = RUN_POLL_MS,
  stop?: () => boolean,
): Promise<TaskRun> {
  while (run.outcome === "running" && !stop?.()) {
    await new Promise((r) => setTimeout(r, everyMs));
    if (stop?.()) break;
    run = await read(run.id);
  }
  return run;
}
