use vault
use http
use secrets
use json
use regex

// Sends the Outbox's approved items (outbox X3, X4). The `outbox` type runs this after every
// create and update of one of its notes, on the home server only: an approval made in an app
// syncs to the server, whose handler sends it.
//
// A run acts only on an item that is `approved` now. It claims the item by setting it `sending`
// against the version it read, so a second run for the same approval finds it changed and stops.
// Then it sends the item by its `kind`:
//
// - `email`: through Gmail (`send_email`). Gmail's answer is recorded at once: `sent_ref` (the
//   Gmail message id), `sent_at` and `status = sent`. The `google_gmail` script's `copy_sent`
//   handler then imports the sent copy as an `email` note and links it as `result`.
// - `event`: a calendar invite or change (`send_event`, outbox X5), written into the event note
//   with `send_updates = "all"`. The event's write-back sends it to Google, which emails the
//   attendees, and marks the item `sent` or `failed`.
// - `sms`: through the phone (`send_sms`, outbox X6).
//
// Each kind's function answers a `Sent`: `done` when the item was sent and its `sent_ref` is
// final, or not `done` when another part of the vault finishes it later (the calendar write-back
// for an event, the device command's result for an SMS), with any `sent_ref` known so far. An
// `Err` sets the item `failed` with the message in `error`; the user retries by approving it
// again.
//
// The run's own writes are authored `handler outbox_send`, which the writer allows to move an
// item from approved to sending and from sending to sent or failed, and to write `sent_ref`,
// `result`, `error` and `sent_at` (outbox X2).

// ---- limits ---------------------------------------------------------------------------------

// The scope Gmail needs to send.
fn send_scope() -> string {
    "https://www.googleapis.com/auth/gmail.send"
}

fn reconnect() -> string {
    "Reconnect Google in Settings > Integrations to send email."
}

// The largest message sent as `raw` in one JSON request, in bytes: 4.5 MB. A larger one goes to
// the media upload endpoint.
fn raw_limit() -> int {
    4718592
}

// The most attachments one email carries, in bytes: Gmail's 25 MB.
fn attachment_limit() -> int {
    26214400
}

// The largest asset a script can read, in bytes: 16 MiB.
fn read_limit() -> int {
    16777216
}

// ---- values ---------------------------------------------------------------------------------

struct Sent {
    done: bool,
    sent_ref: string,
    result: string,
}

fn text_of(v: Value, key: string) -> string {
    match v.get(key) {
        Some(Value::String(s)) => s,
        _ => "",
    }
}

fn int_of(v: Value, key: string) -> int {
    match v.get(key) {
        Some(Value::Int(n)) => n,
        Some(Value::String(s)) => s.parse_int().unwrap_or(0),
        _ => 0,
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

fn fields_of(note: Value) -> Value {
    note.get("note").unwrap_or(Value::Null).get("fields").unwrap_or(Value::Null)
}

// The blocks of content field `name` of `note`, `{ kind, fields, blocks }` each.
fn content_of(note: Value, name: string) -> List[Value] {
    let blocks = list_of(note.get("note").unwrap_or(Value::Null), "blocks")
    for b in blocks {
        if text_of(b, "kind") == "content" && strings_of(b, "labels").contains(name) {
            return list_of(b, "blocks")
        }
    }
    []
}

fn quoted(s: string) -> string {
    "\"" + s.replace("\\", "\\\\").replace("\"", "\\\"") + "\""
}

// The JSON object in secret `name`, or null when there is none or it is not JSON.
fn secret_json(name: string) -> Value {
    match secrets::get(name) {
        Ok(text) => match json::parse(text) {
            Ok(v) => v,
            Err(_) => Value::Null,
        },
        Err(_) => Value::Null,
    }
}

fn day_ms() -> int {
    86400000
}

fn pad2(n: int) -> string {
    fmt("{:02}", n)
}

// Milliseconds since the epoch as an RFC 3339 datetime in UTC.
fn rfc3339(ms: int) -> string {
    let days = ms / day_ms()
    let rest = (ms % day_ms()) / 1000
    let z = days + 719468
    let era = z / 146097
    let doe = z - era * 146097
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100)
    let mp = (5 * doy + 2) / 153
    let d = doy - (153 * mp + 2) / 5 + 1
    let m = if mp < 10 { mp + 3 } else { mp - 9 }
    let y = yoe + era * 400 + if m <= 2 { 1 } else { 0 }
    fmt("{:04}-{}-{}T{}:{}:{}Z", y, pad2(m), pad2(d), pad2(rest / 3600), pad2((rest / 60) % 60), pad2(rest % 60))
}

fn size_text(size: int) -> string {
    fmt("{:.1} MB", float(size) / 1048576.0)
}

// ---- the run --------------------------------------------------------------------------------

fn after_create(note: Value) -> Result[(), string] {
    handle(note)
}

fn after_update(note: Value) -> Result[(), string] {
    handle(note)
}

fn handle(note: Value) -> Result[(), string] {
    if text_of(fields_of(note), "status") != "approved" {
        return Ok(())
    }
    let id = text_of(note, "id")
    let item = match vault::get(id) {
        Ok(n) => n,
        Err(_) => return Ok(()),
    }
    let fields = fields_of(item)
    if text_of(fields, "status") != "approved" {
        return Ok(())
    }
    // Claim the item against the version read, so it is sent once.
    let claim = Value::Map(#{ "status": Value::String("sending"), "error": Value::Null })
    match vault::patch(id, claim, text_of(item, "version")) {
        Ok(_) => {},
        Err(e) => {
            vault::log("Item " + id + " changed before it was claimed, so this run leaves it: " + e)
            return Ok(())
        },
    }
    let kind = text_of(fields, "kind")
    let outcome = if kind == "email" {
        send_email(item)
    } else if kind == "event" {
        send_event(item)
    } else if kind == "sms" {
        send_sms(item)
    } else {
        Err("An Outbox item of kind " + quoted(kind) + " cannot be sent.")
    }
    match outcome {
        Ok(sent) => record_sent(id, sent),
        Err(e) => {
            vault::log("Could not send " + id + ": " + e)
            vault::patch(id, Value::Map(#{ "status": Value::String("failed"), "error": Value::String(e) }), "")?;
            Ok(())
        },
    }
}

// Record what the kind's function answered.
fn record_sent(id: string, sent: Sent) -> Result[(), string] {
    let f: Map[string, Value] = #{}
    if sent.sent_ref != "" {
        f["sent_ref"] = Value::String(sent.sent_ref)
    }
    if sent.done {
        f["status"] = Value::String("sent")
        f["sent_at"] = Value::String(rfc3339(vault::now()))
    }
    if sent.result != "" {
        f["result"] = Value::String(sent.result)
    }
    if f.len() > 0 {
        vault::patch(id, Value::Map(f), "")?;
    }
    vault::log("Item " + id + (if sent.done { " was sent as " + sent.sent_ref + "." } else { " is on its way." }))
    Ok(())
}

// ---- event (outbox X5) ----------------------------------------------------------------------

// A calendar invite or change: write the event note, authored by this handler, with the event's
// one-shot `send_updates = "all"`. The `event` type's write-back (`google_event.ws`) then sends
// it to Google with sendUpdates=all, so Google emails the attendees, and sets this item `sent`,
// with the Google event id as `sent_ref`, or `failed` with Google's message. So the answer is
// not `done`, and its `result` is the event note.
//
// A new event (no `event`) is created from the item's title, `when`, `location`,
// `attendees_raw`, `calendar` and `description`. A retry of one whose first send failed reuses
// the note that try made, the item's `result`. A change sets each of those the item holds on
// the `event` note and leaves the rest as they are; it cannot move an event to another calendar.
fn send_event(item: Value) -> Result[Sent, string] {
    let fields = fields_of(item)
    let target = text_of(fields, "event")
    if target == "" {
        target = text_of(fields, "result")
    }
    let when = fields.get("when").unwrap_or(Value::Null)
    let start = text_of(when, "start")
    let end = text_of(when, "end")
    let description = content_of(item, "description")
    let attendees = strings_of(fields, "attendees_raw")
    if target == "" {
        let title = text_of(fields, "title").trim()
        if title == "" {
            return Err("The new event has no title.")
        }
        if start == "" {
            return Err("The new event has no time: set When and send again.")
        }
        let f: Map[string, Value] = #{
            "title": Value::String(title),
            "when": Value::Map(#{ "start": Value::String(start), "end": Value::String(if end == "" { start } else { end }) }),
            "send_updates": Value::String("all"),
        }
        for k in ["location", "calendar"] {
            let v = text_of(fields, k).trim()
            if v != "" {
                f[k] = Value::String(v)
            }
        }
        if attendees.len() > 0 {
            f["attendees_raw"] = Value::List(attendees.map(|a| Value::String(a)))
        }
        let blocks: List[Value] = []
        if description.len() > 0 {
            blocks.push(description_block(description))
        }
        let made = vault::create("event", Value::Map(#{ "fields": Value::Map(f), "blocks": Value::List(blocks) }))?
        let id = text_of(made, "id")
        vault::log("Item " + text_of(item, "id") + " created event " + id + ".")
        return Ok(Sent { done: false, sent_ref: "", result: id })
    }

    let event = match vault::get(target) {
        Ok(n) => n,
        Err(_) => return Err("The event to change is no longer in the vault."),
    }
    if text_of(event, "type") != "event" {
        return Err("The note to change is not an event.")
    }
    let body = event.get("note").unwrap_or(Value::Null)
    let f: Map[string, Value] = match body.get("fields") {
        Some(Value::Map(m)) => m,
        _ => #{},
    }
    let calendar = text_of(fields, "calendar").trim()
    let current_calendar = text_of(Value::Map(f), "calendar")
    if calendar != "" && current_calendar != "" && calendar != current_calendar {
        return Err("The Outbox cannot move an event to another calendar. Clear Calendar and send again.")
    }
    if calendar != "" {
        f["calendar"] = Value::String(calendar)
    }
    let title = text_of(fields, "title").trim()
    if title != "" {
        f["title"] = Value::String(title)
    }
    if start != "" || end != "" {
        let from = if start == "" { end } else { start }
        f["when"] = Value::Map(#{ "start": Value::String(from), "end": Value::String(if end == "" { from } else { end }) })
    }
    let location = text_of(fields, "location").trim()
    if location != "" {
        f["location"] = Value::String(location)
    }
    if attendees.len() > 0 {
        f["attendees_raw"] = Value::List(attendees.map(|a| Value::String(a)))
    }
    let blocks = list_of(body, "blocks")
    if description.len() > 0 {
        blocks = blocks.filter(|b| !(text_of(b, "kind") == "content" && strings_of(b, "labels").contains("description")))
        blocks.push(description_block(description))
    }
    // A failed send leaves `send_updates` set. Clear it first, so that this write is a change the
    // write-back runs for even when the fields are as the last try left them.
    let version = text_of(event, "version")
    if text_of(Value::Map(f), "send_updates") != "" {
        let cleared = vault::patch(target, Value::Map(#{ "send_updates": Value::Null }), version)?
        version = text_of(cleared, "version")
    }
    f["send_updates"] = Value::String("all")
    f.remove("created");
    f.remove("updated");
    vault::update(target, Value::Map(#{ "fields": Value::Map(f), "blocks": Value::List(blocks) }), version)?;
    vault::log("Item " + text_of(item, "id") + " changed event " + target + ".")
    Ok(Sent { done: false, sent_ref: "", result: target })
}

// The event's `description` content field, holding `blocks`.
fn description_block(blocks: List[Value]) -> Value {
    Value::Map(#{
        "kind": Value::String("content"),
        "labels": Value::List([Value::String("description")]),
        "blocks": Value::List(blocks),
    })
}

// ---- sms (outbox X6) ------------------------------------------------------------------------

// An SMS: queue `send_sms` for the item's device (empty for the vault's one phone with SMS
// sending on) with `{ to, body, outbox }`, recording the command id as `sent_ref` with
// `done = false`. The phone's result for the command marks the item sent, with the phone's id of
// the sent message, or failed (the command result route); the phone's capture of the sent message
// links the copy as `result`. A device whose SMS sending switch is off refuses the command, and
// the item fails with that.
fn send_sms(item: Value) -> Result[Sent, string] {
    let fields = fields_of(item)
    let to = text_of(fields, "phone").trim()
    if to == "" {
        return Err("The SMS has no number: fill in Phone number and send again.")
    }
    let lines: List[string] = []
    let html: List[string] = []
    render_blocks(content_of(item, "body"), lines, html)
    let body = lines.join("\n").trim()
    if body == "" {
        return Err("The SMS has no text: write it in Body and send again.")
    }
    let payload = Value::Map(#{
        "to": Value::String(to),
        "body": Value::String(body),
        "outbox": Value::String(text_of(item, "id")),
    })
    let command = vault::device_command(text_of(fields, "device").trim(), "send_sms", payload)?
    let id = text_of(command, "id")
    vault::log("Item " + text_of(item, "id") + " waits for the phone as command " + id + ".")
    Ok(Sent { done: false, sent_ref: id, result: "" })
}

// ---- email (outbox X4) ----------------------------------------------------------------------

fn endpoint(endpoints: Value, key: string, fallback: string) -> string {
    let url = text_of(endpoints, key)
    let base = if url == "" { fallback } else { url }
    if base.ends_with("/") { base.slice(0, base.len() - 1) } else { base }
}

// `s` percent-encoded for a query string or a form body.
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

// Gmail's error message for a response that is not 200.
fn gmail_error(what: string, status: int, body: string) -> string {
    let message = match json::parse(body) {
        Ok(v) => text_of(v.get("error").unwrap_or(Value::Null), "message"),
        Err(_) => "",
    }
    if status == 403 && message.to_lower().contains("insufficient") {
        return reconnect()
    }
    let why = if message == "" { body.trim() } else { message }
    fmt("Gmail answered {} to {}: {}", status, what, why)
}

struct Gmail {
    base: string,
    upload: string,
    auth: Map[string, string],
}

fn connect() -> Result[Gmail, string] {
    let settings = secret_json("google_settings")
    let scopes = strings_of(settings, "scopes")
    if !scopes.contains(send_scope()) {
        return Err(reconnect())
    }
    match secrets::get("google_refresh_token") {
        Ok(_) => {},
        Err(_) => return Err("Google is not connected. Connect Google in Settings > Integrations to send email."),
    }
    let endpoints = settings.get("endpoints").unwrap_or(Value::Null)
    let token = access_token(endpoint(endpoints, "token", "https://oauth2.googleapis.com/token"))?
    let base = endpoint(endpoints, "gmail", "https://gmail.googleapis.com/gmail/v1")
    // The media upload endpoint sits beside the API: /upload/gmail/v1 on the same host.
    let upload = if base.ends_with("/gmail/v1") {
        base.slice(0, base.len() - 9) + "/upload/gmail/v1"
    } else {
        base + "/upload"
    }
    Ok(Gmail {
        base: base,
        upload: endpoint(endpoints, "gmail_upload", upload),
        auth: #{ "authorization": "Bearer " + token, "accept": "application/json" },
    })
}

fn send_email(item: Value) -> Result[Sent, string] {
    let gmail = connect()?
    let message = build_email(gmail, item)?
    let headers = gmail.auth.clone()
    let r = if message.text.len() < raw_limit() {
        headers["content-type"] = "application/json"
        let body = #{ "raw": Value::String(http::base64url(message.text)) }
        if message.thread != "" {
            body["threadId"] = Value::String(message.thread)
        }
        http::post(gmail.base + "/users/me/messages/send", headers, json::to_string(Value::Map(body)))?
    } else {
        headers["content-type"] = "message/rfc822"
        http::post(gmail.upload + "/users/me/messages/send?uploadType=media", headers, message.text)?
    }
    if r.status != 200 {
        return Err(gmail_error("messages.send", r.status, r.body))
    }
    let sent = json::parse(r.body)?
    let id = text_of(sent, "id")
    if id == "" {
        return Err("Gmail answered no message id for the sent email.")
    }
    Ok(Sent { done: true, sent_ref: id, result: "" })
}

struct Message {
    text: string,
    thread: string,
}

// Whether `s` is printable ASCII, so it goes into a header as it is.
fn plain_ascii(s: string) -> bool {
    for c in s {
        let n = int(c)
        if n < 32 || n > 126 {
            return false
        }
    }
    true
}

// A header value with line breaks taken out.
fn one_line(s: string) -> string {
    s.replace("\r", " ").replace("\n", " ").trim()
}

// Header text, as an RFC 2047 encoded word when it is not plain ASCII.
fn header_text(s: string) -> string {
    let t = one_line(s)
    if plain_ascii(t) { t } else { "=?UTF-8?B?" + http::base64(t) + "?=" }
}

// An address as `addr` or `Name <addr>`, the name encoded when it is not plain ASCII.
fn address(raw: string) -> string {
    let a = one_line(raw)
    if plain_ascii(a) {
        return a
    }
    match regex::captures("^(.*)<([^>]+)>\\s*$", a) {
        Some(groups) => {
            let name = groups[1].trim().replace("\"", "")
            if name == "" { "<" + groups[2].trim() + ">" } else { header_text(name) + " <" + groups[2].trim() + ">" }
        },
        None => a,
    }
}

// The message headers of the email `in_reply_to` names, for threading: `{ message_id,
// references, thread }`, each "" when unknown.
fn reply_headers(gmail: Gmail, email_id: string) -> Result[Map[string, string], string] {
    let out = #{ "message_id": "", "references": "", "thread": "" }
    if email_id == "" {
        return Ok(out)
    }
    let email = match vault::get(email_id) {
        Ok(n) => n,
        Err(_) => return Ok(out),
    }
    let gmail_ref = text_of(fields_of(email), "gmail_ref")
    if gmail_ref == "" {
        vault::log("The email replied to has no Gmail id, so the reply starts a thread of its own.")
        return Ok(out)
    }
    let url = gmail.base + "/users/me/messages/" + encode(gmail_ref) + "?format=metadata&metadataHeaders=Message-ID&metadataHeaders=References"
    let r = http::get(url, gmail.auth)?
    if r.status != 200 {
        vault::log(gmail_error("messages.get of the email replied to", r.status, r.body))
        return Ok(out)
    }
    let m = json::parse(r.body)?
    out["thread"] = text_of(m, "threadId")
    for h in list_of(m.get("payload").unwrap_or(Value::Null), "headers") {
        let name = text_of(h, "name").to_lower()
        if name == "message-id" {
            out["message_id"] = one_line(text_of(h, "value"))
        } else if name == "references" {
            out["references"] = one_line(text_of(h, "value"))
        }
    }
    Ok(out)
}

// The MIME type of a file, by its extension.
fn mime_of(filename: string) -> string {
    let dot = filename.split(".")
    let ext = if dot.len() > 1 { dot[dot.len() - 1].to_lower() } else { "" }
    let types = #{
        "pdf": "application/pdf", "png": "image/png", "jpg": "image/jpeg", "jpeg": "image/jpeg",
        "gif": "image/gif", "webp": "image/webp", "svg": "image/svg+xml", "avif": "image/avif",
        "txt": "text/plain", "md": "text/markdown", "csv": "text/csv", "html": "text/html",
        "htm": "text/html", "json": "application/json", "xml": "application/xml",
        "zip": "application/zip", "ics": "text/calendar", "mp3": "audio/mpeg", "mp4": "video/mp4",
        "doc": "application/msword", "xls": "application/vnd.ms-excel",
        "ppt": "application/vnd.ms-powerpoint",
        "docx": "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
        "xlsx": "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
        "pptx": "application/vnd.openxmlformats-officedocument.presentationml.presentation",
    }
    match types.get(ext) {
        Some(t) => t,
        None => "application/octet-stream",
    }
}

// One base64 MIME part.
fn part(headers: List[string], base64: string) -> string {
    headers.join("\r\n") + "\r\nContent-Transfer-Encoding: base64\r\n\r\n" + http::wrap(base64, 76) + "\r\n"
}

fn multipart(subtype: string, boundary: string, parts: List[string]) -> string {
    let out = "Content-Type: multipart/" + subtype + "; boundary=\"" + boundary + "\"\r\n\r\n"
    for p in parts {
        out = out + "--" + boundary + "\r\n" + p
    }
    out + "--" + boundary + "--\r\n"
}

// The attachment parts of asset ids `ids`, after checking Gmail's 25 MB cap and the 16 MiB a
// script can read.
fn attachment_parts(ids: List[string]) -> Result[List[string], string] {
    if ids.len() == 0 {
        return Ok([])
    }
    let sizes: Map[string, int] = #{}
    let names: Map[string, string] = #{}
    for a in vault::assets()? {
        sizes[text_of(a, "id")] = int_of(a, "size")
        names[text_of(a, "id")] = text_of(a, "filename")
    }
    let total = 0
    for id in ids {
        match sizes.get(id) {
            Some(size) => {
                total = total + size
                if size > read_limit() {
                    return Err(names[id] + " is " + size_text(size) + ", larger than the 16 MB an attachment may be. Remove it and send again.")
                }
            },
            None => return Err("Attachment " + id + " is not in the vault. Remove it and send again."),
        }
    }
    if total > attachment_limit() {
        return Err("The attachments come to " + size_text(total) + ", over the 25 MB Gmail sends in one email. Remove some and send again.")
    }
    let parts: List[string] = []
    for id in ids {
        let asset = vault::read_asset(id)?
        let name = one_line(text_of(asset, "filename")).replace("\"", "'")
        let shown = header_text(name)
        parts.push(part([
            "Content-Type: " + mime_of(name) + "; name=\"" + shown + "\"",
            "Content-Disposition: attachment; filename=\"" + shown + "\"",
        ], text_of(asset, "base64")))
    }
    Ok(parts)
}

fn build_email(gmail: Gmail, item: Value) -> Result[Message, string] {
    let fields = fields_of(item)
    let to = strings_of(fields, "to")
    let cc = strings_of(fields, "cc")
    let bcc = strings_of(fields, "bcc")
    if to.len() + cc.len() + bcc.len() == 0 {
        return Err("The email has no recipients: add an address to To, Cc or Bcc.")
    }
    let subject = text_of(fields, "subject").trim()
    if subject == "" {
        subject = text_of(fields, "title").trim()
    }
    let reply = reply_headers(gmail, text_of(fields, "in_reply_to"))?
    let attachments = attachment_parts(strings_of(fields, "attachments"))?

    let headers: List[string] = []
    match secrets::get("google_account") {
        Ok(account) => {
            if account.trim() != "" {
                headers.push("From: " + address(account))
            }
        },
        Err(_) => {},
    }
    if to.len() > 0 {
        headers.push("To: " + to.map(|a| address(a)).join(", "))
    }
    if cc.len() > 0 {
        headers.push("Cc: " + cc.map(|a| address(a)).join(", "))
    }
    if bcc.len() > 0 {
        headers.push("Bcc: " + bcc.map(|a| address(a)).join(", "))
    }
    headers.push("Subject: " + header_text(subject))
    let message_id = reply["message_id"]
    if message_id != "" {
        headers.push("In-Reply-To: " + message_id)
        let refs = reply["references"]
        headers.push("References: " + (if refs == "" { message_id } else { refs + " " + message_id }))
    }
    headers.push("MIME-Version: 1.0")

    let blocks = content_of(item, "body")
    let rendered = render(blocks)
    let boundary = "=_thevault_" + text_of(item, "id")
    let body = multipart("alternative", boundary + "_alt", [
        part(["Content-Type: text/plain; charset=\"UTF-8\""], http::base64(rendered.text)),
        part(["Content-Type: text/html; charset=\"UTF-8\""], http::base64(rendered.html)),
    ])
    if attachments.len() > 0 {
        body = multipart("mixed", boundary + "_mixed", [body].concat(attachments))
    }
    Ok(Message { text: headers.join("\r\n") + "\r\n" + body, thread: reply["thread"] })
}

// ---- the body -------------------------------------------------------------------------------

struct Rendered {
    text: string,
    html: string,
}

fn escape(s: string) -> string {
    s.replace("&", "&amp;").replace("<", "&lt;").replace(">", "&gt;").replace("\"", "&quot;")
}

// A note mention's title, or the mention as it was.
fn mention_titles(text: string) -> string {
    let t = text
    for found in regex::find_all("\\[\\[[0-9A-HJKMNP-TV-Z]{{26}}\\]\\]", t) {
        let id = found.slice(2, found.len() - 2)
        let title = match vault::get(id) {
            Ok(n) => text_of(fields_of(n), "title"),
            Err(_) => "",
        }
        if title != "" {
            t = t.replace(found, title)
        }
    }
    t
}

// Inline markup as plain text.
fn markup_text(text: string) -> string {
    let t = mention_titles(text)
    t = regex::replace("\\[\\[\\+?[^\\]|]+\\|([^\\]]+)\\]\\]", t, "$1")
    t = regex::replace("\\[([^\\]]*)\\]\\((https?://[^)\\s]+|mailto:[^)\\s]+)\\)", t, "$1 ($2)")
    t = regex::replace("\\[([^\\]]*)\\]\\([^)]*\\)", t, "$1")
    t = regex::replace("\\*\\*(.+?)\\*\\*", t, "$1")
    t = regex::replace("\\*(.+?)\\*", t, "$1")
    t = regex::replace("\\+\\+(.+?)\\+\\+", t, "$1")
    t = regex::replace("~~(.+?)~~", t, "$1")
    t = regex::replace("==(.+?)==", t, "$1")
    t = regex::replace("`([^`]+)`", t, "$1")
    t = regex::replace("\\{{[a-z]+\\}}(.*?)\\{{/\\}}", t, "$1")
    regex::replace("\\\\(.)", t, "$1")
}

// Inline markup as HTML.
fn markup_html(text: string) -> string {
    let t = escape(mention_titles(text))
    t = regex::replace("\\[\\[\\+?[^\\]|]+\\|([^\\]]+)\\]\\]", t, "$1")
    t = regex::replace("\\[([^\\]]*)\\]\\((https?://[^)\\s]+|mailto:[^)\\s]+)\\)", t, "<a href=\"$2\">$1</a>")
    t = regex::replace("\\[([^\\]]*)\\]\\([^)]*\\)", t, "$1")
    t = regex::replace("\\*\\*(.+?)\\*\\*", t, "<strong>$1</strong>")
    t = regex::replace("\\*(.+?)\\*", t, "<em>$1</em>")
    t = regex::replace("\\+\\+(.+?)\\+\\+", t, "<u>$1</u>")
    t = regex::replace("~~(.+?)~~", t, "<s>$1</s>")
    t = regex::replace("==(.+?)==", t, "<mark>$1</mark>")
    t = regex::replace("`([^`]+)`", t, "<code>$1</code>")
    t = regex::replace("\\{{([a-z]+)\\}}(.*?)\\{{/\\}}", t, "<span style=\"color:$1\">$2</span>")
    t = regex::replace("\\\\(.)", t, "$1")
    t.replace("\n", "<br>")
}

// The items of a list block, into `text` and `html`, indented by `depth`.
fn render_items(items: List[Value], style: string, depth: int, text: List[string], html: List[string]) {
    let tag = if style == "numbered" { "ol" } else { "ul" }
    html.push("<" + tag + ">")
    let n = 0
    for item in items.filter(|i| text_of(i, "kind") == "item") {
        n = n + 1
        let f = item.get("fields").unwrap_or(Value::Null)
        let words = text_of(f, "text")
        let checked = match f.get("checked") {
            Some(Value::Bool(b)) => b,
            _ => false,
        }
        let mark = if style == "numbered" {
            fmt("{}. ", n)
        } else if style == "todo" {
            if checked { "[x] " } else { "[ ] " }
        } else {
            "- "
        }
        text.push("  ".repeat(depth) + mark + markup_text(words))
        let box = if style == "todo" { if checked { "&#9745; " } else { "&#9744; " } } else { "" }
        let nested = list_of(item, "blocks")
        if nested.len() > 0 {
            html.push("<li>" + box + markup_html(words))
            render_items(nested, style, depth + 1, text, html)
            html.push("</li>")
        } else {
            html.push("<li>" + box + markup_html(words) + "</li>")
        }
    }
    html.push("</" + tag + ">")
}

// Content blocks into paragraphs of text and HTML.
fn render_blocks(blocks: List[Value], text: List[string], html: List[string]) {
    for b in blocks {
        let kind = text_of(b, "kind")
        let f = b.get("fields").unwrap_or(Value::Null)
        if kind == "paragraph" {
            text.push(markup_text(text_of(f, "text")))
            html.push("<p>" + markup_html(text_of(f, "text")) + "</p>")
        } else if kind == "heading" {
            let level = int_of(f, "level")
            let h = if level >= 1 && level <= 3 { level } else { 2 }
            text.push(markup_text(text_of(f, "text")))
            html.push(fmt("<h{}>", h) + markup_html(text_of(f, "text")) + fmt("</h{}>", h))
        } else if kind == "quote" {
            text.push("> " + markup_text(text_of(f, "text")).replace("\n", "\n> "))
            html.push("<blockquote>" + markup_html(text_of(f, "text")) + "</blockquote>")
        } else if kind == "code" {
            text.push(text_of(f, "source"))
            html.push("<pre><code>" + escape(text_of(f, "source")) + "</code></pre>")
        } else if kind == "list" {
            let lines: List[string] = []
            render_items(list_of(b, "blocks"), text_of(f, "style"), 0, lines, html)
            text.push(lines.join("\n"))
        } else if kind == "toggle" || kind == "admonition" {
            let head = if kind == "toggle" { text_of(f, "text") } else { text_of(f, "title") }
            if head != "" {
                text.push(markup_text(head))
                html.push("<p><strong>" + markup_html(head) + "</strong></p>")
            }
            render_blocks(list_of(b, "blocks"), text, html)
        } else if kind == "columns" || kind == "column" {
            render_blocks(list_of(b, "blocks"), text, html)
        }
    }
}

fn render(blocks: List[Value]) -> Rendered {
    let text: List[string] = []
    let html: List[string] = []
    render_blocks(blocks, text, html)
    Rendered {
        text: text.join("\n\n").replace("\r\n", "\n").replace("\n", "\r\n") + "\r\n",
        html: "<!DOCTYPE html><html><body>" + html.join("\n") + "</body></html>\r\n",
    }
}
