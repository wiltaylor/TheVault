// Icons for types, by Lucide name (docs/vault-format.md: an unknown name shows a default icon).
// The Lucide set is loaded once, on first use, in its own chunk.
import { For, createResource } from "solid-js";
import { Dynamic } from "solid-js/web";

type IconNode = [string, Record<string, string>][];

interface IconSet {
  /** Lucide's icons by export name, such as `FileText`. */
  icons: Record<string, IconNode>;
  /** Every icon's vault name, such as `file-text`, sorted. */
  names: string[];
  /** Export name by vault name. */
  byName: Map<string, string>;
}

let loading: Promise<IconSet> | undefined;

function iconSet(): Promise<IconSet> {
  loading ??= import("lucide").then((m) => {
    const icons = m.icons as unknown as Record<string, IconNode>;
    const byName = new Map(Object.keys(icons).map((key) => [kebabName(key), key] as const));
    return { icons, byName, names: [...byName.keys()].sort() };
  });
  return loading;
}

/** Every icon name the picker offers, loading the icon set on first use. */
export async function iconNames(): Promise<string[]> {
  return (await iconSet()).names;
}

/** Lucide's export name `FileText` as the vault's icon name, `file-text`. */
export function kebabName(key: string): string {
  return key
    .replace(/([a-z0-9])([A-Z])/g, "$1-$2")
    .replace(/([A-Z])([A-Z][a-z])/g, "$1-$2")
    .replace(/([a-zA-Z])([0-9])/g, "$1-$2")
    .toLowerCase();
}

/** `file-text` as Lucide's export name, `FileText`. */
export function pascalName(name: string): string {
  return name
    .split(/[-_\s]+/)
    .filter(Boolean)
    .map((part) => part[0]!.toUpperCase() + part.slice(1).toLowerCase())
    .join("");
}

const FALLBACK = "File";

/** The Lucide icon called `name`, or a plain file icon when there is no such icon. */
export function TypeIcon(props: { name: string | null | undefined; size?: number }) {
  const [node] = createResource(
    () => props.name ?? "",
    async (name) => {
      const { icons, byName } = await iconSet();
      return icons[byName.get(name) ?? pascalName(name)] ?? icons[FALLBACK] ?? [];
    },
  );
  return (
    <svg
      class="icon"
      width={props.size ?? 20}
      height={props.size ?? 20}
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      stroke-width="2"
      stroke-linecap="round"
      stroke-linejoin="round"
      aria-hidden="true"
    >
      <For each={node() ?? []}>{([tag, attrs]) => <Dynamic component={tag} {...attrs} />}</For>
    </svg>
  );
}

/** A gear, drawn inline so the settings button never waits on the icon set. */
export function GearIcon() {
  return (
    <svg
      class="icon"
      width="20"
      height="20"
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      stroke-width="2"
      stroke-linecap="round"
      stroke-linejoin="round"
      aria-hidden="true"
    >
      <path d="M12.22 2h-.44a2 2 0 0 0-2 2v.18a2 2 0 0 1-1 1.73l-.43.25a2 2 0 0 1-2 0l-.15-.08a2 2 0 0 0-2.73.73l-.22.38a2 2 0 0 0 .73 2.73l.15.1a2 2 0 0 1 1 1.72v.51a2 2 0 0 1-1 1.74l-.15.09a2 2 0 0 0-.73 2.73l.22.38a2 2 0 0 0 2.73.73l.15-.08a2 2 0 0 1 2 0l.43.25a2 2 0 0 1 1 1.73V20a2 2 0 0 0 2 2h.44a2 2 0 0 0 2-2v-.18a2 2 0 0 1 1-1.73l.43-.25a2 2 0 0 1 2 0l.15.08a2 2 0 0 0 2.73-.73l.22-.39a2 2 0 0 0-.73-2.73l-.15-.08a2 2 0 0 1-1-1.74v-.5a2 2 0 0 1 1-1.74l.15-.09a2 2 0 0 0 .73-2.73l-.22-.38a2 2 0 0 0-2.73-.73l-.15.08a2 2 0 0 1-2 0l-.43-.25a2 2 0 0 1-1-1.73V4a2 2 0 0 0-2-2z" />
      <circle cx="12" cy="12" r="3" />
    </svg>
  );
}

function Glyph(props: { children: unknown }) {
  return (
    <svg
      class="icon"
      width="18"
      height="18"
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      stroke-width="2"
      stroke-linecap="round"
      stroke-linejoin="round"
      aria-hidden="true"
    >
      {props.children as never}
    </svg>
  );
}

/** A calendar, for the Daily notes link. */
export function CalendarIcon() {
  return (
    <Glyph>
      <rect x="3" y="4" width="18" height="18" rx="2" />
      <path d="M16 2v4M8 2v4M3 10h18" />
    </Glyph>
  );
}

/** A magnifying glass, for the Search link and box. */
export function SearchIcon() {
  return (
    <Glyph>
      <circle cx="11" cy="11" r="7" />
      <path d="m20 20-3.5-3.5" />
    </Glyph>
  );
}

/** A bell, for the notifications button. */
export function BellIcon() {
  return (
    <Glyph>
      <path d="M6 8a6 6 0 0 1 12 0c0 7 3 9 3 9H3s3-2 3-9" />
      <path d="M10.3 21a1.94 1.94 0 0 0 3.4 0" />
    </Glyph>
  );
}

/** An inbox tray, for the Inbox link. */
export function InboxIcon() {
  return (
    <Glyph>
      <path d="M22 12h-6l-2 3h-4l-2-3H2" />
      <path d="M5.45 5.11 2 12v6a2 2 0 0 0 2 2h16a2 2 0 0 0 2-2v-6l-3.45-6.89A2 2 0 0 0 16.76 4H7.24a2 2 0 0 0-1.79 1.11z" />
    </Glyph>
  );
}

/** A plus in a circle, for the Capture button. */
export function CaptureIcon() {
  return (
    <Glyph>
      <circle cx="12" cy="12" r="9" />
      <path d="M12 8v8M8 12h8" />
    </Glyph>
  );
}

/** A page layout, for the link to a type's templates. */
export function TemplateIcon() {
  return (
    <Glyph>
      <rect x="3" y="3" width="18" height="7" rx="1" />
      <rect x="3" y="14" width="9" height="7" rx="1" />
      <rect x="16" y="14" width="5" height="7" rx="1" />
    </Glyph>
  );
}

/** A bin, for the trash button beside the theme switch. */
export function TrashIcon() {
  return (
    <svg
      class="icon"
      width="20"
      height="20"
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      stroke-width="2"
      stroke-linecap="round"
      stroke-linejoin="round"
      aria-hidden="true"
    >
      <path d="M3 6h18M19 6v14a2 2 0 0 1-2 2H7a2 2 0 0 1-2-2V6M8 6V4a2 2 0 0 1 2-2h4a2 2 0 0 1 2 2v2M10 11v6M14 11v6" />
    </svg>
  );
}

/** An archive box, for archiving a note and the Archive page. */
export function ArchiveIcon() {
  return (
    <Glyph>
      <rect x="2" y="3" width="20" height="5" rx="1" />
      <path d="M4 8v11a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V8" />
      <path d="M10 12h4" />
    </Glyph>
  );
}

/** An archive box with an arrow out of it, for taking a note out of the archive. */
export function UnarchiveIcon() {
  return (
    <Glyph>
      <rect x="2" y="3" width="20" height="5" rx="1" />
      <path d="M4 8v11a2 2 0 0 0 2 2h2" />
      <path d="M20 8v11a2 2 0 0 1-2 2h-2" />
      <path d="m9 15 3-3 3 3" />
      <path d="M12 12v9" />
    </Glyph>
  );
}

/** A sun or a moon, for the theme switch's thumb. */
export function ThemeGlyph(props: { dark: boolean }) {
  return (
    <svg
      class="icon"
      width="12"
      height="12"
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      stroke-width="2.5"
      stroke-linecap="round"
      stroke-linejoin="round"
      aria-hidden="true"
    >
      {props.dark ? (
        <path d="M12 3a6 6 0 0 0 9 9 9 9 0 1 1-9-9Z" />
      ) : (
        <>
          <circle cx="12" cy="12" r="4" />
          <path d="M12 2v2M12 20v2M4.93 4.93l1.41 1.41M17.66 17.66l1.41 1.41M2 12h2M20 12h2M6.34 17.66l-1.41 1.41M19.07 4.93l-1.41 1.41" />
        </>
      )}
    </svg>
  );
}
