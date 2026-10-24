import { Route, useNavigate, useParams } from "@solidjs/router";
import { cleanup, render, screen } from "@solidjs/testing-library";
import { createResource } from "solid-js";
import { afterEach, expect, it } from "vitest";
import * as M from "./model";
import { PaneRouter, locationTitle } from "./pane";
import { goPane, navigatePane, resetWorkspace, workspace } from "./store";

afterEach(cleanup);

function Page() {
  const params = useParams();
  const navigate = useNavigate();
  const [label] = createResource(
    () => params.n,
    async (n) => {
      await new Promise((r) => setTimeout(r, 5));
      return `page ${n}`;
    },
  );
  return (
    <div>
      <button onClick={() => navigate(`/x/${Number(params.n) + 1}`)}>next {params.n}</button>
      <p>{label()}</p>
    </div>
  );
}

it("each pane routes over its own history", async () => {
  let ws = M.emptyWorkspace("/x/1");
  const a = M.activePane(ws);
  ws = M.splitPane(ws, a, "right", "/x/10");
  const b = M.activePane(ws);
  resetWorkspace(ws);
  render(() => (
    <>
      <PaneRouter pane={a}>
        <Route path="/x/:n" component={Page} />
      </PaneRouter>
      <PaneRouter pane={b}>
        <Route path="/x/:n" component={Page} />
      </PaneRouter>
    </>
  ));
  expect(await screen.findByText("page 1")).toBeTruthy();
  expect(await screen.findByText("page 10")).toBeTruthy();

  // A page navigating moves only its own pane, and pushes onto its history.
  screen.getByText("next 1").click();
  expect(await screen.findByText("page 2")).toBeTruthy();
  expect(screen.getByText("page 10")).toBeTruthy();
  expect(workspace().panes[a]).toEqual({ history: ["/x/1", "/x/2"], index: 1 });
  expect(workspace().panes[b]).toEqual({ history: ["/x/10"], index: 0 });

  // Back, and a navigation from outside the pane (a sidebar link), reach its router.
  goPane(a, -1);
  expect(await screen.findByText("page 1")).toBeTruthy();
  navigatePane(b, "/x/20");
  expect(await screen.findByText("page 20")).toBeTruthy();
  expect(window.location.pathname).toBe("/x/20");
});

it("names the Log pane Log, with the scroll icon (activity Y4)", () => {
  expect(locationTitle("/log")).toEqual({ title: "Log", icon: "scroll-text" });
  expect(locationTitle("/log?category=ai&actor=agent:triage")).toEqual({ title: "Log", icon: "scroll-text" });
});

it("names the Skills page's pane Skills and a skill screen's pane Skill (skills S4, S5)", () => {
  expect(locationTitle("/skills")).toEqual({ title: "Skills", icon: "book-open" });
  expect(locationTitle("/skills/pdf-forms")).toEqual({ title: "Skill", icon: "book-open" });
  expect(locationTitle("/skills/pdf-forms/files")).toEqual({ title: "Skill", icon: "book-open" });
});
