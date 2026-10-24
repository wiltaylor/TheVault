// The vault's definitions changed somewhere else: a type, taxonomy, agent or skill written on
// another device, by an agent, a script or the CLI. The server's live stream says so with
// `definition.changed`; in an app, which has no stream, the embedded server's sync status counts
// the changes a round pulled. Either way the lists the app holds are read again. A burst of
// changes reads each list at most twice: once now, and once more after that read if more came.

import { agentsChanged } from "./agents";
import { reloadDefinitions } from "./session";
import { refreshSkills } from "./skills";

type Part = "types" | "agents" | "skills";

const read: Record<Part, () => Promise<void>> = {
  types: reloadDefinitions,
  agents: agentsChanged,
  skills: refreshSkills,
};

const running = new Map<Part, Promise<void>>();
const again = new Set<Part>();

function reread(part: Part): Promise<void> {
  const now = running.get(part);
  if (now) {
    again.add(part);
    return now;
  }
  const p = read[part]()
    .catch(() => undefined)
    .finally(() => {
      running.delete(part);
      if (again.delete(part)) void reread(part);
    });
  running.set(part, p);
  return p;
}

/** The lists a change of feed kind `kind` makes stale; every list for `null`. */
export function partsOf(kind: string | null): Part[] {
  if (kind === null) return ["types", "agents", "skills"];
  if (kind === "type" || kind === "taxonomy") return ["types"];
  if (kind === "agent") return ["agents"];
  if (kind === "skill") return ["skills"];
  return [];
}

/** Read again what a change of `kind` makes stale, or everything when `kind` is `null`. */
export async function definitionsChanged(kind: string | null): Promise<void> {
  await Promise.all(partsOf(kind).map(reread));
}
