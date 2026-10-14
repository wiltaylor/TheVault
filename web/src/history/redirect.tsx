// The old history page's address, /notes/:id/history: the note in its pane, with the History
// panel showing its versions, so links to it keep working.

import { Navigate, useParams } from "@solidjs/router";
import { focusPanel } from "../sidebars";
import { activePane } from "../workspace/model";
import { usePane } from "../workspace/pane";
import { workspace } from "../workspace/store";

export function NoteHistoryRedirect() {
  const params = useParams();
  const pane = usePane();
  // A pane restored in the background keeps the sidebar as it is.
  if (!pane || activePane(workspace()) === pane.id) focusPanel("history");
  return <Navigate href={`/notes/${params.id}`} />;
}
