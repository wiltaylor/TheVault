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
