import { cleanup, fireEvent, render, screen } from "@solidjs/testing-library";
import { afterEach, expect, it, vi } from "vitest";
import { ApprovalRules } from "./ApprovalRules";
import type { AgentPermissions } from "../api";

afterEach(cleanup);
const permissions = { types: ["email"], change_fields: { email: ["processed"] }, approval_rules: [{ operation: "delete", note_type: "email", fields: [], daily_cap: 5 }] } as unknown as AgentPermissions;

it("adds a narrow update without discarding stored held rules", () => {
  const set = vi.fn();
  render(() => <ApprovalRules permissions={permissions} editable set={set} />);
  fireEvent.input(screen.getByLabelText("Rule type"), { target: { value: "email" } });
  fireEvent.input(screen.getByLabelText("Rule fields"), { target: { value: "processed" } });
  fireEvent.input(screen.getByLabelText("Rule daily cap"), { target: { value: "200" } });
  fireEvent.click(screen.getByText("Add update rule"));
  expect(set).toHaveBeenCalledWith([...permissions.approval_rules!, { operation: "update", note_type: "email", fields: ["processed"], daily_cap: 200 }]);
  expect(screen.getByText(/held, never automatic/)).toBeTruthy();
  expect(screen.getByText(/Read types: email/)).toBeTruthy();
});

it.each(["-1", "1.5", "9007199254740992", "", "NaN"])("rejects invalid cap %s", (cap) => {
  const set = vi.fn();
  render(() => <ApprovalRules permissions={permissions} editable set={set} />);
  fireEvent.input(screen.getByLabelText("Rule type"), { target: { value: "email" } });
  fireEvent.input(screen.getByLabelText("Rule fields"), { target: { value: "processed" } });
  fireEvent.input(screen.getByLabelText("Rule daily cap"), { target: { value: cap } });
  fireEvent.click(screen.getByText("Add update rule"));
  expect(set).not.toHaveBeenCalled();
  expect(screen.getByRole("alert")).toBeTruthy();
});

it("shows rules read-only on the phone without edit controls", () => {
  render(() => <ApprovalRules permissions={permissions} editable={false} set={vi.fn()} />);
  expect(screen.queryByText("Add update rule")).toBeNull();
  expect(screen.queryByText("Remove rule 1")).toBeNull();
  expect(screen.getByText(/held, never automatic/)).toBeTruthy();
});

it.each([
  { change_types: ["task"] },
  { types: ["task"] },
  { change_fields: { email: ["subject"] } },
  { change_fields: { email: [] } },
])("shows rules outside change permissions as never applicable: %j", (limits) => {
  render(() => <ApprovalRules permissions={{ ...permissions, ...limits, approval_rules: [
    { operation: "update", note_type: "email", fields: ["processed"], daily_cap: 200 },
  ] }} editable={false} set={vi.fn()} />);
  expect(screen.getByText(/never applies \(outside this agent's change permissions\)/)).toBeTruthy();
  expect(screen.queryByText(/subject to server limits/)).toBeNull();
});

it("discloses attempted-update caps and unattended incoming-mail approvals", () => {
  render(() => <ApprovalRules permissions={permissions} editable={false} set={vi.fn()} />);
  expect(screen.getByText(/Caps count attempted updates/).textContent).toContain("released only when the write provably did not happen");
  const disclosure = screen.getByText(/A rule also approves its matching update/).textContent;
  expect(disclosure).toContain("unattended runs started by incoming mail");
  expect(disclosure).toContain("without a matching rule is refused or held");
  expect(disclosure).toContain("outside-text action guard is not yet in place");
});
