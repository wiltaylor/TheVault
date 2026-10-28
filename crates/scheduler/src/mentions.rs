//! Calls of agents committed in notes (agents G19): where in a note a call may stand and what it
//! asks.
//!
//! A call is a link `[@<Label> <request>](agent:<name>?call=<id>)` in the inline markup of a
//! note's content blocks: each paragraph, heading, list item, quote, toggle, admonition title and
//! table cell at any depth. [`schema::agent_calls`] finds the calls in one text; inline code,
//! note mentions and other links hold none.

use schema::{Agent, AgentCall, Doc, DocBlock};

/// One call in a note, with the text around it: the markup item that holds it.
pub(crate) struct Found {
    pub call: AgentCall,
    pub context: String,
}

/// The calls of agents in `doc`'s content blocks, in note order.
pub(crate) fn calls(doc: &Doc) -> Vec<Found> {
    let mut out = Vec::new();
    for b in &doc.blocks {
        if b.kind == "content" {
            each_markup(&b.body.blocks, &mut out);
        }
    }
    out
}

fn each_markup(blocks: &[DocBlock], out: &mut Vec<Found>) {
    for b in blocks {
        for (_, text) in schema::markup_texts(b) {
            let found = schema::agent_calls(text);
            let context = with_labels(text, &found);
            out.extend(found.into_iter().map(|call| Found {
                call,
                context: context.clone(),
            }));
        }
        each_markup(&b.body.blocks, out);
    }
}

/// `text` with each call in it standing as its link text.
fn with_labels(text: &str, calls: &[AgentCall]) -> String {
    let mut out = String::with_capacity(text.len());
    let mut at = 0;
    for c in calls {
        out.push_str(&text[at..c.start]);
        out.push_str(&c.label);
        at = c.end;
    }
    out.push_str(&text[at..]);
    out.trim().to_owned()
}

/// What a call's link text asks of `agent`: the text after `@<Label>` or `@<name>`, or all of
/// it when it starts with neither.
pub(crate) fn request(label: &str, agent: &Agent) -> String {
    let text = label.trim();
    for word in [agent.label.trim(), agent.name.as_str()] {
        if word.is_empty() {
            continue;
        }
        let call = format!("@{word}");
        if let Some(head) = text.get(..call.len())
            && head.eq_ignore_ascii_case(&call)
        {
            let rest = &text[call.len()..];
            if rest.is_empty() || rest.starts_with(char::is_whitespace) {
                return rest.trim().to_owned();
            }
        }
    }
    text.to_owned()
}

/// The first `n` characters of `s`, with its runs of whitespace as single spaces.
pub(crate) fn first_chars(s: &str, n: usize) -> String {
    let flat = s.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut out: String = flat.chars().take(n).collect();
    if flat.chars().count() > n {
        out = out.trim_end().to_owned();
        out.push('\u{2026}');
    }
    out
}
