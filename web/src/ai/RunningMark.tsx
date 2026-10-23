// The marks a thread and an agent carry while a turn runs on them, and a thread whose turn
// answered while nobody had it open (agents G17): a small spinner, a still dot when motion is
// reduced, read as "Running"; and a dot read as "New reply".

import { Spinner } from "../components/SyncStatus";
import "./chat.css";

export function RunningMark() {
  return (
    <span class="ai-running" role="img" aria-label="Running" title="Running">
      <Spinner />
      <span class="ai-running-dot" aria-hidden="true" />
    </span>
  );
}

export function NewReplyMark() {
  return <span class="ai-new-reply" role="img" aria-label="New reply" title="New reply" />;
}
