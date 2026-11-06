import { cleanup, fireEvent, render, waitFor } from "@solidjs/testing-library";
import { createSignal } from "solid-js";
import { afterEach, expect, it, vi } from "vitest";
import { EventResponse } from "./EventResponse";

afterEach(cleanup);

it("offers three answers and lets an answered invitation change", async () => {
  const answer = vi.fn();
  const view = render(() => {
    const [response, setResponse] = createSignal("needs_action");
    return <EventResponse response={response()} onAnswer={(value) => { answer(value); setResponse(value); }} />;
  });
  expect(view.getByRole("button", { name: "Maybe" })).toBeVisible();
  expect(view.getByRole("button", { name: "Decline" })).toBeVisible();
  fireEvent.click(view.getByRole("button", { name: "Accept" }));
  await waitFor(() => expect(answer).toHaveBeenCalledWith("accepted"));
  expect(view.getByText("Your response: Accepted")).toBeVisible();
  fireEvent.click(view.getByRole("button", { name: "Change response" }));
  fireEvent.click(view.getByRole("button", { name: "Maybe" }));
  await waitFor(() => expect(answer).toHaveBeenCalledWith("tentative"));
});

it("shows nothing for organisers and non-attendees", () => {
  const view = render(() => <EventResponse response={undefined} onAnswer={() => {}} />);
  expect(view.queryByRole("group")).toBeNull();
});

it("reports a failed save without hiding the choices", async () => {
  const view = render(() => <EventResponse response="needs_action" onAnswer={async () => { throw new Error("Save failed"); }} />);
  fireEvent.click(view.getByRole("button", { name: "Decline" }));
  await waitFor(() => expect(view.getByRole("alert")).toHaveTextContent("Save failed"));
  expect(view.getByRole("button", { name: "Accept" })).not.toBeDisabled();
});
