import type { AiAccountChoice, AiAccounts, AiUsage } from "../api";

// The AI accounts as the chat's picker and the agent screen read them (accounts A5, A6, A13).

/** The account called `name`: names are unique without regard to case (accounts A2). */
export function accountNamed<A extends { name: string }>(accounts: readonly A[], name: string | null | undefined): A | undefined {
  const n = name?.trim().toLowerCase();
  return n ? accounts.find((a) => a.name.toLowerCase() === n) : undefined;
}

/**
 * The account an agent that names `name` runs on (accounts A5): that one when it exists, else the
 * default account, else the first in the table; null when there is none.
 */
export function accountFor(list: AiAccounts | null | undefined, name: string | null | undefined): AiAccountChoice | null {
  if (!list) return null;
  return accountNamed(list.accounts, name) ?? list.accounts.find((a) => a.id === list.default) ?? list.accounts[0] ?? null;
}

/** The most used of a subscription's limits, in percent, or null with no reading (accounts A13). */
export function usedPercent(usage: AiUsage | null | undefined): number | null {
  if (!usage || usage.windows.length === 0) return null;
  return Math.round(Math.max(...usage.windows.map((w) => w.used_percent)));
}

/** An account as the picker lists it: its name, and how much of its plan is used when known. */
export function accountChoiceLabel(a: AiAccountChoice): string {
  const used = usedPercent(a.usage);
  return used === null ? a.name : `${a.name} (${used}% used)`;
}
