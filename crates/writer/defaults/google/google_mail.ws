use vault
use http
use secrets
use json

// Mail actions (TV-25): handling an email in the vault goes back to Gmail, one way only. The
// `email` type runs this after every update and delete of one of its notes, on the home server
// only: an action taken in an app reaches it when the app syncs.
//
// - after_update: an email whose `processed` turns from not set to true (Done in the inbox)
//   leaves Gmail's inbox: `messages.modify` removes the `INBOX` label. The message keeps its
//   other labels and stays in All Mail.
// - after_delete: an email moved to the vault's trash goes to Gmail's trash with
//   `messages.trash`, which Gmail empties after 30 days. Its `gmail_ref` is also added to the
//   `google_gmail_trashed` secret, so the Gmail sync never imports the message again, whether or
//   not Gmail was reached.
//
// Restoring an email untrashes it at Google. Unticking `processed` does not undo archiving.
//
// The loop guard: a write by a scheduled task, such as the `google_gmail` sync, or by a handler
// sends nothing. The handler sees who wrote the note in its argument's `author`. The inbox's
// "Mark older processed" (TV-30) writes its run's own id to `inbox_catch_up` with `processed`,
// and a write that changes `inbox_catch_up` sends nothing either. So no catch-up run archives
// mail in Gmail, also one that marks an email again after the user unticked it. The user's own
// tick leaves `inbox_catch_up` as it is, so it archives as usual.
//
// Without `gmail.modify` in the settings' `scopes`, as for a connection made before mail actions,
// nothing is sent: the run logs it and ends successfully, and the Google card asks the user to
// reconnect. Without Google connected the run also ends successfully. A message Gmail no longer
// has (404) counts as done. Any other answer, or a failed sign-in, fails the run, which records
// Gmail's message, and raises a notification on the email: nothing retries it, so the user does
// it in Gmail.
//
// The settings are the JSON object in the `google_settings` secret: `scopes` and the `endpoints`
// test seam (google-sync D21).

// The scope mail actions need.
fn modify_scope() -> string {
    "https://www.googleapis.com/auth/gmail.modify"
}

// The most refs `google_gmail_trashed` keeps, newest last: about 40 KB, under the 64 KiB a
// secret holds.
fn trashed_limit() -> int {
    2000
}

fn text_of(v: Value, key: string) -> string {
    match v.get(key) {
        Some(Value::String(s)) => s,
        _ => "",
    }
}

fn list_of(v: Value, key: string) -> List[Value] {
    match v.get(key) {
        Some(Value::List(items)) => items,
        _ => [],
    }
}

fn strings_of(v: Value, key: string) -> List[string] {
    list_of(v, key).map(|s| s.as_string().unwrap_or("").trim()).filter(|s| s != "")
}

fn is_true(fields: Value, key: string) -> bool {
    match fields.get(key) {
        Some(Value::Bool(b)) => b,
        _ => false,
    }
}

// The fields of `{ fields, blocks }` at `key` of the handler's argument.
fn fields_at(note: Value, key: string) -> Value {
    note.get(key).unwrap_or(Value::Null).get("fields").unwrap_or(Value::Null)
}

// The JSON value in secret `name`, or null when there is none or it is not JSON.
fn secret_json(name: string) -> Value {
    match secrets::get(name) {
        Ok(text) => match json::parse(text) {
            Ok(v) => v,
            Err(_) => Value::Null,
        },
        Err(_) => Value::Null,
    }
}

// Whether the write was the user's: anything but a scheduled task or a handler (the loop guard).
fn by_user(note: Value) -> bool {
    let author = note.get("author").unwrap_or(Value::Null)
    let kind = text_of(author, "kind")
    kind == "session" || kind == "token" || (kind == "agent" && is_true(author, "gmail_actions"))
}

// `s` percent-encoded for a URL path segment or a form value.
fn encode(s: string) -> string {
    let out: List[string] = []
    for c in s {
        let n = int(c)
        let plain = (n >= 48 && n <= 57) || (n >= 65 && n <= 90) || (n >= 97 && n <= 122)
        if plain || c == '-' || c == '.' || c == '_' || c == '~' {
            out.push(str(c))
        } else if n < 128 {
            out.push(fmt("%{:02X}", n))
        } else {
            out.push(str(c))
        }
    }
    out.join("")
}

fn endpoint(endpoints: Value, key: string, fallback: string) -> string {
    let url = text_of(endpoints, key)
    let base = if url == "" { fallback } else { url }
    if base.ends_with("/") { base.slice(0, base.len() - 1) } else { base }
}

// A new access token from the refresh token.
fn access_token(token_url: string) -> Result[string, string] {
    let form = "client_id=" + encode(secrets::get("google_client_id")?) +
        "&client_secret=" + encode(secrets::get("google_client_secret")?) +
        "&refresh_token=" + encode(secrets::get("google_refresh_token")?) +
        "&grant_type=refresh_token"
    let r = http::post(token_url, #{ "content-type": "application/x-www-form-urlencoded", "accept": "application/json" }, form)?
    let body = match json::parse(r.body) {
        Ok(v) => v,
        Err(_) => Value::Null,
    }
    if r.status == 200 {
        let token = text_of(body, "access_token")
        if token == "" {
            return Err("Google's token endpoint answered no access token.")
        }
        return Ok(token)
    }
    if text_of(body, "error") == "invalid_grant" {
        return Err("The Google sign-in expired. Connect Google again in Settings > Integrations.")
    }
    Err(fmt("Google's token endpoint answered {}: {} {}", r.status, text_of(body, "error"), text_of(body, "error_description")))
}

struct Gmail {
    ok: bool,
    base: string,
    auth: Map[string, string],
}

// The Gmail API to send a mail action to, or `ok: false` with a log line when none may be sent.
fn connect() -> Result[Gmail, string] {
    let none = Gmail { ok: false, base: "", auth: #{} }
    match secrets::get("google_refresh_token") {
        Ok(_) => {},
        Err(_) => {
            vault::log("Google is not connected: nothing sent to Gmail.")
            return Ok(none)
        },
    }
    let settings = secret_json("google_settings")
    if !strings_of(settings, "scopes").contains(modify_scope()) {
        vault::log("Reconnect Google to sync mail actions: the connection lacks gmail.modify, so nothing was sent to Gmail.")
        return Ok(none)
    }
    let endpoints = settings.get("endpoints").unwrap_or(Value::Null)
    let token = access_token(endpoint(endpoints, "token", "https://oauth2.googleapis.com/token"))?
    Ok(Gmail {
        ok: true,
        base: endpoint(endpoints, "gmail", "https://gmail.googleapis.com/gmail/v1"),
        auth: #{ "authorization": "Bearer " + token, "accept": "application/json", "content-type": "application/json" },
    })
}

// POST `body` to message `id`'s `action` (`modify` or `trash`). A message Gmail no longer has
// counts as done.
fn act(gmail: Gmail, id: string, action: string, body: string) -> Result[(), string] {
    let url = gmail.base + "/users/me/messages/" + encode(id) + "/" + action
    let r = http::post(url, gmail.auth, body)?
    if r.status == 200 {
        vault::log("Gmail message " + id + ": " + action + " done.")
        return Ok(())
    }
    if r.status == 404 {
        if action == "untrash" {
            return Err("Gmail no longer has this message. It may have been permanently deleted; the vault copy was restored but Google could not restore it.")
        }
        vault::log("Gmail has no message " + id + " any more: nothing to " + action + ".")
        return Ok(())
    }
    let message = match json::parse(r.body) {
        Ok(v) => text_of(v.get("error").unwrap_or(Value::Null), "message"),
        Err(_) => "",
    }
    let why = if message == "" { r.body.trim() } else { message }
    Err(fmt("Gmail answered {} to {} of message {}: {}", r.status, action, id, clip(why, 300)))
}

// ---- the handlers ---------------------------------------------------------------------------

fn after_update(note: Value) -> Result[(), string] {
    if !by_user(note) {
        return Ok(())
    }
    let now = fields_at(note, "note")
    let before = fields_at(note, "previous")
    if !is_true(now, "processed") || is_true(before, "processed") {
        return Ok(())
    }
    if text_of(now, "inbox_catch_up") != text_of(before, "inbox_catch_up") {
        return Ok(())
    }
    let id = text_of(now, "gmail_ref")
    if id == "" {
        return Ok(())
    }
    let body = Value::Map(#{ "removeLabelIds": Value::List([Value::String("INBOX")]) })
    reported(note, "archive", text_of(note, "id"), send(id, "modify", json::to_string(body)))
}

fn after_create(note: Value) -> Result[(), string] {
    if !is_true(note, "restored") || !by_user(note) {
        return Ok(())
    }
    let id = text_of(fields_at(note, "note"), "gmail_ref")
    if id == "" {
        return Ok(())
    }
    reported(note, "untrash", text_of(note, "id"), restore_mail(id))
}

fn restore_mail(id: string) -> Result[(), string] {
    match secrets::get("google_refresh_token") {
        Ok(_) => {},
        Err(_) => {
            vault::log("Google is not connected: the vault copy was restored, no Gmail restore was sent, and the suppressed sync reference was kept.")
            return Ok(())
        },
    }
    let gmail = connect()?
    if !gmail.ok {
        return Err("Reconnect Google with gmail.modify to restore this message if it reached Gmail trash. Gmail trash expires after 30 days.")
    }
    act(gmail, id, "untrash", "")?
    let refs = match secret_json("google_gmail_trashed") {
        Value::List(items) => items.filter(|v| v.as_string().unwrap_or("") != id),
        _ => [],
    }
    secrets::set("google_gmail_trashed", json::to_string(Value::List(refs)))
}

fn after_delete(note: Value) -> Result[(), string] {
    // Local suppression is independent of permission to send an HTTP request to Gmail.
    let kind = text_of(note.get("author").unwrap_or(Value::Null), "kind")
    if kind == "" || kind == "task" || kind == "handler" {
        return Ok(())
    }
    let id = text_of(fields_at(note, "note"), "gmail_ref")
    if id == "" {
        return Ok(())
    }
    // The trash is sent also when the ref cannot be remembered, and each failure raises its own
    // notification, not tied to the email, which is in the vault's trash.
    let remembered = remember_trashed(id)
    let sent = if by_user(note) {
        reported(note, "trash", "", send(id, "trash", ""))
    } else {
        vault::log("Gmail trash skipped: the local delete is remembered so sync does not import it again. Gmail actions need user Apply or an opted-in foreground chat.")
        Ok(())
    }
    match remembered {
        Ok(_) => sent,
        Err(e) => {
            let why = "the vault could not remember it as trashed: " + clip(e, 300)
            let body = sentence(why) + about(note) + " The Gmail sync can import it again."
            notice("A trashed email can come back", body, "")
            Err(why)
        },
    }
}

// `s` cut to at most `n` characters, with an ellipsis when it was cut.
fn clip(s: string, n: int) -> string {
    if s.len() > n { s.slice(0, n - 1) + "…" } else { s }
}

// `s` ending in one full stop.
fn sentence(s: string) -> string {
    if s.ends_with(".") { s } else { s + "." }
}

// " Email: <subject>." for the email in the handler's argument, or "" when it has no subject.
fn about(note: Value) -> string {
    let subject = text_of(fields_at(note, "note"), "title")
    if subject == "" { "" } else { " Email: " + clip(subject, 200) + "." }
}

// Raise a notification on note `on` unless that is "", and log it when it cannot be raised.
fn notice(title: string, body: string, on: string) {
    match vault::notify(title, body, on) {
        Ok(_) => {},
        Err(n) => vault::log("The notification was not raised: " + n),
    }
}

// Connect and POST `body` to message `id`'s `action`. Not connected, or without gmail.modify,
// sends nothing and succeeds.
fn send(id: string, action: string, body: string) -> Result[(), string] {
    let gmail = connect()?
    if !gmail.ok {
        return Ok(())
    }
    act(gmail, id, action, body)
}

// `result` as it is, after a notification when it failed, on note `on` unless that is "":
// nothing retries a mail action, so the user must know to `verb` the message in Gmail. A
// notification that cannot be raised is logged, and the run still fails with Gmail's error.
fn reported(note: Value, verb: string, on: string, result: Result[(), string]) -> Result[(), string] {
    match result {
        Ok(_) => Ok(()),
        Err(e) => {
            let title = "Gmail did not " + verb + " an email"
            let condition = if verb == "untrash" { " if it reached Gmail trash" } else { "" }
            let body = sentence(clip(e, 600)) + about(note) + " Nothing retries it: " + verb + " the message in Gmail" + condition + "."
            notice(title, body, on)
            Err(e)
        },
    }
}

// Add `id` to the `google_gmail_trashed` secret, the refs the Gmail sync skips.
fn remember_trashed(id: string) -> Result[(), string] {
    let refs = match secret_json("google_gmail_trashed") {
        Value::List(items) => items.map(|v| v.as_string().unwrap_or("")).filter(|s| s != "" && s != id),
        _ => [],
    }
    refs.push(id)
    let keep = if refs.len() > trashed_limit() { refs.slice(refs.len() - trashed_limit(), refs.len()) } else { refs }
    secrets::set("google_gmail_trashed", json::to_string(Value::List(keep.map(|s| Value::String(s)))))
}
