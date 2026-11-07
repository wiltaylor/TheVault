//! The chat's `ask_user` tool (TV-77 D3): in a run the user is not watching, a call from a note
//! or notes sent to the agent, the agent asks the user a question and the run ends.
//!
//! It is offered only in turns on a thread of origin `mention` or `dispatch` ([`asks_on`]), the
//! user's reply turns on it included, never in a plain chat (TV-77 Q13), where the agent asks in
//! its answer, and never to a sub agent. An agent's `permissions.tools` list filters it as it
//! filters any tool (TV-77 Q10): an agent with no list has it.
//!
//! The turn loop (`crate::ai_chat`) answers the call with [`RESULT`], keeps the question in the
//! thread's hidden `question` field, sends a `question` event, and ends the turn once the round's
//! other calls are answered, so the transcript stays whole. The thread then reads `needs_you`
//! (`crate::agent_threads::state_of`), and an unattended run that ends so leaves a notification
//! under the bell with the question (`crate::agent_runs`, TV-77 Q9). The user's next message on
//! the thread clears the field in the same write that adds the message, and runs as a normal
//! turn, which may ask again.
//!
//! An agent with its own system prompt never reads the shipped chat prompt, so the server adds
//! [`prompt_section`] to the system text of every turn that offers the tool (TV-77 F8).

use ai::ToolSpec;
use serde_json::{Value, json};

use crate::ai_chat::{ToolOut, tool};

/// The tool's name.
pub const TOOL: &str = "ask_user";

/// The longest question, so the bell's notification holds it whole.
pub const MAX_QUESTION: usize = state::MAX_BODY;

/// The tool result the model reads.
pub const RESULT: &str =
    "Asked the user. This run ends; their answer comes as the next message.";

/// Whether a turn on a thread of origin `origin` offers the tool: a call from a note or notes
/// sent to the agent, never a plain chat, a trigger's run or a sub agent's session.
pub(crate) fn asks_on(origin: &str) -> bool {
    [scheduler::MENTION_ORIGIN, scheduler::DISPATCH_ORIGIN].contains(&origin)
}

pub(crate) fn tool_spec() -> ToolSpec {
    tool(
        TOOL,
        &format!(
            "Ask the user a question you need answered before you can go on, and end this run. \
             Offered only when no one watches the run: a call from a note or notes sent to you. \
             Finish what you can first, and ask once, with everything you need in one question. \
             The thread then shows that it needs the user, and the bell tells them; their answer \
             comes as the next message in this thread. Any other calls in the same round still \
             run. At most {MAX_QUESTION} characters."
        ),
        json!({
            "question": { "type": "string", "description": "The question, as plain text the user can answer without reading the rest of the thread." },
        }),
    )
}

/// The question a call asks, trimmed, or the error result for a call without one or with one
/// too long.
pub(crate) fn question(input: &Value) -> Result<String, ToolOut> {
    let q = input["question"].as_str().map_or("", str::trim);
    if q.is_empty() {
        return Err(ToolOut::error("Send the question to ask."));
    }
    if q.chars().count() > MAX_QUESTION {
        return Err(ToolOut::error(format!(
            "The question is longer than {MAX_QUESTION} characters: ask it shorter."
        )));
    }
    Ok(q.to_owned())
}

/// The section of the system text of a turn that offers the tool.
pub(crate) fn prompt_section() -> &'static str {
    "\n## Asking the user\n\nNo one watches this run. When you cannot go on without the user's \
     answer, such as a choice only they can make or a fact the notes do not give, do what you \
     can first, then call ask_user with one question that holds all you need. The run ends after \
     that round, and their answer comes as the next message in this thread. Do not ask about \
     what you can find out with your tools, and do not ask only to confirm a change: propose it \
     instead.\n"
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_calls_from_notes_and_sent_notes_ask() {
        assert!(asks_on("mention"));
        assert!(asks_on("dispatch"));
        for origin in ["user", "trigger:nightly", "subagent:01JB", ""] {
            assert!(!asks_on(origin), "{origin}");
        }
    }

    #[test]
    fn a_question_must_be_given_and_fit_the_bell() {
        assert_eq!(
            question(&json!({ "question": "  Which roof? \n" })).ok(),
            Some("Which roof?".to_owned())
        );
        assert!(question(&json!({ "question": "  " })).is_err());
        assert!(question(&json!({})).is_err());
        let long = "a".repeat(MAX_QUESTION + 1);
        assert!(question(&json!({ "question": long })).is_err());
        let fits = "a".repeat(MAX_QUESTION);
        assert!(question(&json!({ "question": fits })).is_ok());
    }
}
