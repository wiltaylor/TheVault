import { activePane, expect, logIn, test } from "./fixtures";

test("a failed invitation save shows an error on the event page", async ({ page, admin }) => {
  const made = await admin.post("/api/notes?type=event", {
    data: { note: { fields: { title: "Failed RSVP", my_response: "needs_action" }, blocks: [] } },
  });
  expect(made.status()).toBe(201);
  const { id } = await made.json();
  await logIn(page, `/notes/${id}`);
  await page.route(`**/api/notes/${id}`, (route) => route.request().method() === "PUT"
    ? route.fulfill({ status: 500, contentType: "application/json", body: JSON.stringify({ error: "Save failed" }) })
    : route.continue());
  const invitation = activePane(page).getByRole("group", { name: "Invitation response" });
  await invitation.getByRole("button", { name: "Accept", exact: true }).click();
  await expect(invitation.getByRole("alert")).toContainText("could not be saved");
});

test("accept an invitation on its event page, then change the answer on its card", async ({ page, admin }) => {
  const made = await admin.post("/api/notes?type=event", {
    data: { note: { fields: { title: "Invitation RSVP", my_response: "needs_action" }, blocks: [] } },
  });
  expect(made.status(), await made.text()).toBe(201);
  const { id } = await made.json();
  await logIn(page, `/notes/${id}`);
  const invitation = activePane(page).getByRole("group", { name: "Invitation response" });
  await expect(invitation.getByRole("button", { name: "Maybe" })).toBeVisible();
  await expect(invitation.getByRole("button", { name: "Decline" })).toBeVisible();
  await invitation.getByRole("button", { name: "Accept", exact: true }).click();
  await expect(page.getByTestId("save-status")).toHaveText("Saved");
  await expect.poll(async () => (await (await admin.get(`/api/notes/${id}`)).json()).note.fields.my_response).toBe("accepted");

  const holder = await admin.post("/api/notes?type=event", {
    data: { note: { fields: { title: "Invitation card" }, blocks: [
      { kind: "content", labels: ["description"], fields: {}, blocks: [
        { kind: "card", fields: { note: id }, blocks: [] },
      ] },
    ] } },
  });
  expect(holder.status(), await holder.text()).toBe(201);
  const source = await holder.json();
  await page.goto(`/notes/${source.id}`);
  const card = activePane(page).getByTestId("note-card");
  await expect(card.getByText("Your response: Accepted")).toBeVisible();
  await card.getByRole("button", { name: "Change response" }).click();
  await card.getByRole("button", { name: "Decline" }).click();
  await expect(card.getByText("Your response: Declined")).toBeVisible();
  await expect.poll(async () => (await (await admin.get(`/api/notes/${id}`)).json()).note.fields.my_response).toBe("declined");
  await expect(page).toHaveURL(new RegExp(`/notes/${source.id}$`));
});
