import { cleanup, fireEvent, render, waitFor } from "@solidjs/testing-library";
import { afterEach, expect, it, vi } from "vitest";
import { type AiStatus, type ClassifierSettings as Settings, api } from "../api";
import { ClassifierSettings } from "./ClassifierSettings";

vi.mock("./TypeEditor", () => ({ errorText: String }));
afterEach(() => { cleanup(); vi.restoreAllMocks(); });

it("explains ID storage and submits the selected ID rather than its name", async () => {
  const settings: Settings = {
    accounts: [{ id: "account-id", name: "Équipe" }],
    classifiers: [{ name: "incoming", valid: true, reason: "missing_account", last_outcome: "not_recorded" }],
  };
  vi.spyOn(api, "classifierSettings").mockResolvedValue(settings);
  const select = vi.spyOn(api, "classifierAccount").mockResolvedValue(settings);
  const view = render(() => <ClassifierSettings status={{} as AiStatus} />);
  await waitFor(() => expect(view.getByText(/Choosing an account stores its ID/)).toBeTruthy());
  const input = view.getByLabelText("Account for incoming");
  fireEvent.change(input, { target: { value: "account-id" } });
  await waitFor(() => expect(select).toHaveBeenCalledWith("incoming", "account-id"));
});
