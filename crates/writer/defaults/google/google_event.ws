use vault
use http
use secrets
use json
use regex

// Writes changes to `event` notes back to Google Calendar (google-sync D9, D10, D20). The event
// type runs this after every create, update and delete of one of its notes.
//
// A handler cannot tell who wrote a note, so the note's hidden `snapshot`, what Google last sent,
// is the loop guard: a note whose synced fields equal its snapshot has no change of the vault's
// to send, and the handler sends nothing. That covers every write of the `google_calendar` task,
// and this script's own write-back.
//
// - after_update: PATCH the event (or the one occurrence) with only the fields that differ from
//   the snapshot and `If-Match: <etag>`. A changed attendee list is first merged with the
//   entries Google holds, read with a GET, so each kept attendee keeps its response. On 412, GET
//   the event, merge field by field as the task does (D10), and PATCH once more. Then write
//   Google's `etag` and the new `snapshot`.
// - after_create: a note with no `google_ref` is inserted into its `calendar`, or the settings'
//   `default_calendar`. A note with one that the task just made is left alone. Any other,
//   such as a note restored from the trash, is looked up: when Google no longer has the event it
//   is inserted as a new event under a new `google_ref` (D20).
// - after_delete: DELETE the event or occurrence (D9).
//
// The Outbox (outbox X5) writes an invite or change into the note with the hidden one-shot
// field `send_updates = "all"`. The insert or PATCH then carries `sendUpdates=all`, so Google
// emails the attendees, and the write-back clears the field and records the send on the Outbox
// items sending this event, which then leave the Outbox (outbox X14), or marks them `failed` with
// Google's message. A direct edit leaves the field unset,
// so Google emails no one.
//
// The settings are the JSON object in the `google_settings` secret: `default_calendar`, and the
// `endpoints` test seam (google-sync D21). The handler reads it and writes nothing but notes.
//
// The conversion from note to Google is the inverse of `google_calendar.ws`'s; see
// `docs/scripts.md`, "The Google Calendar sync".

struct Ctx {
    ok: bool,
    token: string,
    base: string,
    default_calendar: string,
    people: Map[string, string],
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

fn map_of(v: Value, key: string) -> Map[string, Value] {
    match v.get(key) {
        Some(Value::Map(m)) => m,
        _ => #{},
    }
}

fn strings(items: List[Value]) -> List[string] {
    items.map(|v| v.as_string().unwrap_or("")).filter(|s| s != "")
}

// A string literal for a query.
fn quoted(s: string) -> string {
    let escaped = s.replace("\\", "\\\\").replace("\"", "\\\"").replace("\n", "\\n")
    "\"" + escaped.replace("{{", "{{{{").replace("}}", "}}}}") + "\""
}

// `s` percent-encoded for a URL path segment or form value.
fn encode(s: string) -> string {
    let out = ""
    for c in s {
        let n = int(c)
        let plain = (n >= 48 && n <= 57) || (n >= 65 && n <= 90) || (n >= 97 && n <= 122)
        if plain || c == '-' || c == '.' || c == '_' || c == '~' {
            out = out + str(c)
        } else if n < 128 {
            out = out + fmt("%{:02X}", n)
        } else {
            out = out + str(c)
        }
    }
    out
}

// ---- dates and times ------------------------------------------------------------------------

fn num(s: string, from: int, to: int) -> int {
    if s.len() < to {
        return 0
    }
    s.slice(from, to).parse_int().unwrap_or(0)
}

// Days since 1970-01-01 of a Gregorian date.
fn days_from_civil(year: int, m: int, d: int) -> int {
    let y = if m <= 2 { year - 1 } else { year }
    let era = if y >= 0 { y / 400 } else { (y - 399) / 400 }
    let yoe = y - era * 400
    let mp = (m + 9) % 12
    let doy = (153 * mp + 2) / 5 + d - 1
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy
    era * 146097 + doe - 719468
}

fn date_of_days(days: int) -> string {
    let z = days + 719468
    let era = if z >= 0 { z / 146097 } else { (z - 146096) / 146097 }
    let doe = z - era * 146097
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100)
    let mp = (5 * doy + 2) / 153
    let d = doy - (153 * mp + 2) / 5 + 1
    let m = if mp < 10 { mp + 3 } else { mp - 9 }
    let y = if m <= 2 { yoe + era * 400 + 1 } else { yoe + era * 400 }
    fmt("{:04}-{:02}-{:02}", y, m, d)
}

fn days_of(date: string) -> int {
    days_from_civil(num(date, 0, 4), num(date, 5, 7), num(date, 8, 10))
}

fn add_days(date: string, n: int) -> string {
    date_of_days(days_of(date) + n)
}

// Seconds since the epoch of an RFC 3339 time, or of a date at midnight UTC.
fn secs_of(t: string) -> int {
    if t.len() < 10 {
        return 0
    }
    let day = days_of(t.slice(0, 10)) * 86400
    if t.len() < 19 {
        return day
    }
    let local = day + num(t, 11, 13) * 3600 + num(t, 14, 16) * 60 + num(t, 17, 19)
    match regex::captures("([+-])(\\d\\d):(\\d\\d)$", t) {
        Some(c) => {
            let offset = c[2].parse_int().unwrap_or(0) * 3600 + c[3].parse_int().unwrap_or(0) * 60
            if c[1] == "+" { local - offset } else { local + offset }
        }
        None => local,
    }
}

// ---- Google ---------------------------------------------------------------------------------

// An access token from the refresh token. `Err` starting `invalid_grant` when Google refused it.
fn access_token(url: string) -> Result[string, string] {
    let form = "grant_type=refresh_token&client_id=" + encode(secrets::get("google_client_id")?) +
        "&client_secret=" + encode(secrets::get("google_client_secret")?) +
        "&refresh_token=" + encode(secrets::get("google_refresh_token")?)
    let r = http::post(url, #{ "content-type": "application/x-www-form-urlencoded", "accept": "application/json" }, form)?
    let data = json::parse(r.body).unwrap_or(Value::Null)
    let token = text_of(data, "access_token")
    if r.status == 200 && token != "" {
        return Ok(token)
    }
    let error = text_of(data, "error")
    if error == "invalid_grant" {
        return Err("invalid_grant: " + text_of(data, "error_description"))
    }
    Err(fmt("Google's token endpoint answered {}: {}", r.status, if error != "" { error } else { r.body }))
}

fn not_connected() -> Ctx {
    Ctx { ok: false, token: "", base: "", default_calendar: "", people: #{} }
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

// The string at `key` of `m`, or `fallback` when there is none.
fn text_or(m: Map[string, Value], key: string, fallback: string) -> string {
    match m.get(key) {
        Some(Value::String(s)) => if s == "" { fallback } else { s },
        _ => fallback,
    }
}

// The Calendar API with an access token, from the `google_settings` secret. `ok` is false when
// Google is not connected (there is no refresh token), or Google refused the refresh token: the
// `google_calendar` task records that and notifies the user, and this run ends without failing.
fn connect() -> Result[Ctx, string] {
    match secrets::get("google_refresh_token") {
        Ok(_) => {},
        Err(_) => {
            vault::log("Google is not connected: there is no google_refresh_token secret, so nothing is sent.")
            return Ok(not_connected())
        },
    }
    let fields = secret_json("google_settings")
    let endpoints = map_of(fields, "endpoints")
    let token_url = text_or(endpoints, "token", "https://oauth2.googleapis.com/token")
    let base = text_or(endpoints, "calendar", "https://www.googleapis.com/calendar/v3")
    let default_calendar = text_of(fields, "default_calendar")
    match access_token(token_url) {
        Ok(t) => Ok(Ctx {
            ok: true,
            token: t,
            base: base,
            default_calendar: if default_calendar == "" { "primary" } else { default_calendar },
            people: #{},
        }),
        Err(e) => {
            if !e.starts_with("invalid_grant") {
                return Err(e)
            }
            vault::log("Google refused the refresh token, so nothing is sent: " + e)
            Ok(not_connected())
        }
    }
}

// A Calendar API call: `{ status, body }` with the body parsed.
fn call(ctx: Ctx, method: string, url: string, etag: string, body: Value) -> Result[Value, string] {
    let headers = #{ "authorization": "Bearer " + ctx.token, "accept": "application/json" }
    let text = ""
    if !body.is_null() {
        headers["content-type"] = "application/json"
        text = json::to_string(body)
    }
    if etag != "" {
        headers["if-match"] = etag
    }
    let r = http::request(method, url, headers, text)?
    Ok(Value::Map(#{ "status": Value::Int(r.status), "body": json::parse(r.body).unwrap_or(Value::Null) }))
}

fn status_of(reply: Value) -> int {
    reply.get("status").unwrap().as_int().unwrap_or(0)
}

fn body_of(reply: Value) -> Value {
    reply.get("body").unwrap_or(Value::Null)
}

fn failure(what: string, reply: Value) -> string {
    let message = match body_of(reply).get("error") {
        Some(e) => text_of(e, "message"),
        None => "",
    }
    fmt("Google Calendar {} answered {}: {}", what, status_of(reply), message)
}

fn events_url(ctx: Ctx, calendar: string) -> string {
    ctx.base + "/calendars/" + encode(calendar) + "/events"
}

fn event_url(ctx: Ctx, calendar: string, google_ref: string) -> string {
    events_url(ctx, calendar) + "/" + encode(google_ref)
}

// Whether a GET answer says Google no longer has the event.
fn gone(reply: Value) -> bool {
    let status = status_of(reply)
    status == 404 || status == 410 || (status == 200 && text_of(body_of(reply), "status") == "cancelled")
}

// ---- the synced fields ----------------------------------------------------------------------

// The snapshot keys, grouped as they merge: `start` and `end` settle together.
fn groups() -> List[List[string]] {
    [["title"], ["start", "end"], ["location"], ["description"], ["attendees"], ["organiser"], ["status"], ["meet_url"]]
}

// The groups Google lets a client set: `organizer` and `hangoutLink` are Google's own.
fn writable() -> List[List[string]] {
    [["title"], ["start", "end"], ["location"], ["description"], ["attendees"], ["status"]]
}

fn snapshot_keys() -> List[string] {
    ["title", "start", "end", "location", "description", "attendees", "organiser", "status", "meet_url"]
}

// Text as a description holds it: lines without their line-ending spaces, blank lines dropped.
fn clean(text: string) -> string {
    text.replace("\r", "").split("\n").map(|l| l.trim_end()).filter(|l| l.trim() != "").join("\n")
}

// The synced fields of a Google event, in the form the vault holds them.
fn synced(item: Value) -> Map[string, string] {
    let start = item.get("start").unwrap_or(Value::Null)
    let end = item.get("end").unwrap_or(Value::Null)
    let all_day = text_of(start, "date") != ""
    let from = if all_day { text_of(start, "date") } else { text_of(start, "dateTime") }
    let to = if all_day {
        // Google's all-day end is the day after; the vault's range ends on the last day.
        let after = text_of(end, "date")
        if after == "" || days_of(after) - 1 < days_of(from) { from } else { add_days(after, -1) }
    } else {
        let t = text_of(end, "dateTime")
        if t == "" { from } else { t }
    }
    let attendees = list_of(item, "attendees").map(|a| text_of(a, "email")).filter(|e| e != "")
    let organiser = match item.get("organizer") {
        Some(o) => text_of(o, "email"),
        None => "",
    }
    let status = text_of(item, "status")
    #{
        "title": text_of(item, "summary"),
        "start": from,
        "end": to,
        "location": text_of(item, "location"),
        "description": clean(text_of(item, "description")),
        "attendees": attendees.join("\n"),
        "organiser": organiser,
        "status": if status == "" { "confirmed" } else { status },
        "meet_url": text_of(item, "hangoutLink"),
    }
}

// The attendee list to send for `addresses`, merged with `held`, the attendees Google holds: an
// address Google already has keeps its whole entry, with its responseStatus, displayName and
// optional and organizer flags; a new address is sent as `{ email }`; an address the note no
// longer has is left out. Addresses compare ignoring case.
fn attendees_for(addresses: List[string], held: List[Value]) -> List[Value] {
    addresses.map(|a| {
        let wanted = a.to_lower()
        match held.find(|h| text_of(h, "email").to_lower() == wanted) {
            Some(entry) => entry,
            None => Value::Map(#{ "email": Value::String(a) }),
        }
    })
}

// The members of a Google event that set the groups `which` of `values`: the inverse of
// `synced`. `held` is the attendee list Google holds, which a new attendee list keeps the
// entries of.
fn to_google(values: Map[string, string], which: List[List[string]], held: List[Value]) -> Map[string, Value] {
    let out: Map[string, Value] = #{}
    for group in which {
        let k = group[0]
        if k == "title" {
            out["summary"] = Value::String(value(values, "title"))
        } else if k == "start" {
            let from = value(values, "start")
            let to = value(values, "end")
            if to == "" {
                to = from
            }
            if from.len() == 10 {
                // The vault's range ends on its last day; Google's all-day end is the day after.
                out["start"] = Value::Map(#{ "date": Value::String(from), "dateTime": Value::Null })
                out["end"] = Value::Map(#{ "date": Value::String(add_days(to, 1)), "dateTime": Value::Null })
            } else if from != "" {
                out["start"] = Value::Map(#{ "dateTime": Value::String(from), "date": Value::Null })
                out["end"] = Value::Map(#{ "dateTime": Value::String(to), "date": Value::Null })
            }
        } else if k == "location" {
            out["location"] = Value::String(value(values, "location"))
        } else if k == "description" {
            out["description"] = Value::String(value(values, "description"))
        } else if k == "attendees" {
            let list = value(values, "attendees").split("\n").filter(|a| a != "")
            out["attendees"] = Value::List(attendees_for(list, held))
        } else if k == "status" {
            let status = value(values, "status")
            if status != "" {
                out["status"] = Value::String(status)
            }
        }
    }
    out
}

fn label_of(block: Value) -> string {
    match list_of(block, "labels").get(0) {
        Some(l) => l.as_string().unwrap_or(""),
        None => "",
    }
}

fn is_field_block(block: Value, kind: string, name: string) -> bool {
    text_of(block, "kind") == kind && label_of(block) == name
}

// The text of content field `name`, a line per paragraph.
fn content_text(blocks: List[Value], name: string) -> string {
    for b in blocks {
        if is_field_block(b, "content", name) {
            let lines = list_of(b, "blocks").map(|p| text_of(p.get("fields").unwrap_or(Value::Null), "text"))
            return clean(lines.join("\n"))
        }
    }
    ""
}

// The members of dict field `name`.
fn dict_of(blocks: List[Value], name: string) -> Map[string, string] {
    let out: Map[string, string] = #{}
    for b in blocks {
        if is_field_block(b, "dict", name) {
            let fields = map_of(b, "fields")
            for k in fields.keys() {
                out[k] = fields[k].as_string().unwrap_or("")
            }
        }
    }
    out
}

// The synced fields of an event note.
fn note_values(fields: Value, blocks: List[Value]) -> Map[string, string] {
    let when = fields.get("when").unwrap_or(Value::Null)
    #{
        "title": text_of(fields, "title"),
        "start": text_of(when, "start"),
        "end": text_of(when, "end"),
        "location": text_of(fields, "location"),
        "description": content_text(blocks, "description"),
        "attendees": strings(list_of(fields, "attendees_raw")).join("\n"),
        "organiser": text_of(fields, "organiser"),
        "status": text_of(fields, "status"),
        "meet_url": text_of(fields, "meet_url"),
    }
}

fn value(m: Map[string, string], key: string) -> string {
    m.get(key).unwrap_or("")
}

// Two values of key `k` alike: times are the same instant whatever offset they were written in.
fn same(k: string, a: string, b: string) -> bool {
    if (k == "start" || k == "end") && a.len() > 10 && b.len() > 10 {
        return secs_of(a) == secs_of(b)
    }
    a == b
}

fn group_differs(a: Map[string, string], b: Map[string, string], group: List[string]) -> bool {
    group.any(|k| !same(k, value(a, k), value(b, k)))
}

// The groups of `which` in which `a` and `b` differ.
fn differing(a: Map[string, string], b: Map[string, string], which: List[List[string]]) -> List[List[string]] {
    which.filter(|g| group_differs(a, b, g))
}

// D10: Google's values merged into the note's against the snapshot of what Google last sent.
fn merge(google: Map[string, string], snapshot: Map[string, string], mine: Map[string, string], google_newer: bool) -> Map[string, string] {
    let out: Map[string, string] = #{}
    for group in groups() {
        let in_vault = group_differs(mine, snapshot, group)
        let in_google = group_differs(google, snapshot, group)
        let take_google = snapshot.is_empty() || !in_vault || (in_google && google_newer)
        for k in group {
            out[k] = if take_google { value(google, k) } else { value(mine, k) }
        }
    }
    out
}

// ---- the note -------------------------------------------------------------------------------

// The person whose `emails` holds `address`, ignoring case, or "".
fn person_for(ctx: Ctx, address: string) -> Result[string, string] {
    let wanted = address.to_lower()
    if let Some(id) = ctx.people.get(wanted) {
        return Ok(id)
    }
    let found = ""
    for row in vault::query("from person where has_ci(emails, " + quoted(wanted) + ") limit 1")? {
        found = text_of(row, "id")
    }
    ctx.people[wanted] = found
    Ok(found)
}

fn put(fields: Map[string, Value], key: string, text: string) {
    if text == "" {
        fields.remove(key);
    } else {
        fields[key] = Value::String(text);
    }
}

fn paragraphs(text: string) -> List[Value] {
    text.split("\n").filter(|l| l.trim() != "").map(|l| Value::Map(#{
        "kind": Value::String("paragraph"),
        "fields": Value::Map(#{ "text": Value::String(l) }),
    }))
}

// The note body for `values` on top of the note's `fields` and `blocks`, as the task writes it.
fn body_for(ctx: Ctx, fields: Map[string, Value], blocks: List[Value], values: Map[string, string], snapshot: Map[string, string], meta: Map[string, string], same_attendees: bool) -> Result[Value, string] {
    put(fields, "title", value(values, "title"))
    if value(values, "start") == "" {
        fields.remove("when");
    } else {
        fields["when"] = Value::Map(#{
            "start": Value::String(value(values, "start")),
            "end": Value::String(value(values, "end")),
        });
    }
    put(fields, "location", value(values, "location"))
    put(fields, "organiser", value(values, "organiser"))
    put(fields, "status", value(values, "status"))
    put(fields, "meet_url", value(values, "meet_url"))
    let addresses = value(values, "attendees").split("\n").filter(|a| a != "")
    if addresses.is_empty() {
        fields.remove("attendees_raw");
        fields.remove("attendees");
    } else {
        fields["attendees_raw"] = Value::List(addresses.map(|a| Value::String(a)));
        if !same_attendees || !fields.contains_key("attendees") {
            let ids: List[Value] = []
            for a in addresses {
                let id = person_for(ctx, a)?
                if id != "" && !ids.any(|i| i.as_string().unwrap_or("") == id) {
                    ids.push(Value::String(id))
                }
            }
            if ids.is_empty() {
                fields.remove("attendees");
            } else {
                fields["attendees"] = Value::List(ids);
            }
        }
    }
    put(fields, "calendar", meta["calendar"])
    put(fields, "series", meta["series"])
    put(fields, "google_ref", meta["google_ref"])
    put(fields, "etag", meta["etag"])
    // The Outbox's one-shot request to email the attendees, sent with this write-back.
    fields.remove("send_updates");
    fields.remove("created");
    fields.remove("updated");

    let kept = blocks.filter(|b| !is_field_block(b, "content", "description") && !is_field_block(b, "dict", "snapshot"))
    let description = value(values, "description")
    if description != "" {
        kept.push(Value::Map(#{
            "kind": Value::String("content"),
            "labels": Value::List([Value::String("description")]),
            "blocks": Value::List(paragraphs(description)),
        }))
    }
    let snap: Map[string, Value] = #{}
    for k in snapshot_keys() {
        snap[k] = Value::String(value(snapshot, k))
    }
    kept.push(Value::Map(#{
        "kind": Value::String("dict"),
        "labels": Value::List([Value::String("snapshot")]),
        "fields": Value::Map(snap),
    }))
    Ok(Value::Map(#{ "fields": Value::Map(fields), "blocks": Value::List(kept) }))
}

// Record Google's answer `event` on note `id`: its etag, its values as the snapshot, and its
// values in each field group the note still holds as `basis`, what was sent. A group the user
// changed again since keeps the user's value, for the next run to send. `google_ref` and
// `calendar` are set when not "", and `clear_series` removes `series`. The write clears the
// Outbox's one-shot `send_updates`; when it was set, the send is recorded on the Outbox items
// sending this event (outbox X5, X14).
fn write_back(ctx: Ctx, id: string, basis: Map[string, string], event: Value, google_ref: string, calendar: string, clear_series: bool) -> Result[(), string] {
    let google = synced(event)
    let tries = 0
    let last = ""
    while tries < 3 {
        tries += 1
        let note = match vault::get(id) {
            Ok(n) => n,
            Err(_) => return Ok(()),
        }
        let body = note.get("note").unwrap()
        let fields = map_of(body, "fields")
        let blocks = list_of(body, "blocks")
        let current = note_values(Value::Map(fields), blocks)
        let values: Map[string, string] = #{}
        for group in groups() {
            let take_google = !group_differs(current, basis, group)
            for k in group {
                values[k] = if take_google { value(google, k) } else { value(current, k) }
            }
        }
        let f = Value::Map(fields)
        let invited = text_of(f, "send_updates") != ""
        let meta = #{
            "calendar": if calendar != "" { calendar } else { text_of(f, "calendar") },
            "series": if clear_series { "" } else { text_of(f, "series") },
            "google_ref": if google_ref != "" { google_ref } else { text_of(f, "google_ref") },
            "etag": text_of(event, "etag"),
        }
        let same_attendees = value(values, "attendees") == value(current, "attendees")
        let next = body_for(ctx, fields, blocks, values, google, meta, same_attendees)?
        match vault::update(id, next, text_of(note, "version")) {
            Ok(_) => {
                if invited {
                    sent_items(id, text_of(event, "id"))
                }
                return Ok(())
            },
            Err(e) => {
                last = e
            }
        }
    }
    Err("writing Google's answer back to the note: " + last)
}

fn calendar_of(ctx: Ctx, fields: Value) -> string {
    let c = text_of(fields, "calendar")
    if c == "" { ctx.default_calendar } else { c }
}

// The query string that asks Google to email the attendees about an insert or patch: `?sendUpdates=`
// and the note's `send_updates`, which the Outbox sets for one write-back (outbox X5). "" when it is
// unset, as for every direct edit of the note, so Google emails no one.
fn updates_query(fields: Value) -> string {
    let send = text_of(fields, "send_updates")
    if send == "" { "" } else { "?sendUpdates=" + encode(send) }
}

// Insert note `id` as a new event, and record its new `google_ref`. Answers "", as every
// function below does once Google took the change, or why nothing was sent.
fn insert(ctx: Ctx, id: string, fields: Value, mine: Map[string, string]) -> Result[string, string] {
    let calendar = calendar_of(ctx, fields)
    let body = to_google(mine, writable(), [])
    for k in ["summary", "location", "description"] {
        if body.get(k).is_some() && body[k].as_string().unwrap_or("") == "" {
            body.remove(k);
        }
    }
    let reply = call(ctx, "POST", events_url(ctx, calendar) + updates_query(fields), "", Value::Map(body))?
    if status_of(reply) != 200 {
        return Err(failure("events.insert into " + calendar, reply))
    }
    let event = body_of(reply)
    vault::log("Inserted as " + text_of(event, "id") + " into " + calendar + ".")
    write_back(ctx, id, mine, event, text_of(event, "id"), calendar, true)?;
    Ok("")
}

fn gone_reason(google_ref: string) -> string {
    vault::log("Google no longer has " + google_ref + ", so nothing is sent.")
    "Google Calendar no longer has this event."
}

// PATCH the groups of the note that differ from its snapshot; on 412, merge and try once more.
fn send_changes(ctx: Ctx, id: string, fields: Value, mine: Map[string, string], snapshot: Map[string, string], changed: List[List[string]]) -> Result[string, string] {
    let google_ref = text_of(fields, "google_ref")
    let url = event_url(ctx, calendar_of(ctx, fields), google_ref)
    let updates = updates_query(fields)
    // A new attendee list replaces Google's whole, so read the entries Google holds to keep them.
    let held: List[Value] = []
    if changed.any(|g| g[0] == "attendees") {
        let current = call(ctx, "GET", url, "", Value::Null)?
        if gone(current) {
            return Ok(gone_reason(google_ref))
        }
        if status_of(current) != 200 {
            return Err(failure("events.get of " + google_ref, current))
        }
        held = list_of(body_of(current), "attendees")
    }
    let reply = call(ctx, "PATCH", url + updates, text_of(fields, "etag"), Value::Map(to_google(mine, changed, held)))?
    let status = status_of(reply)
    if status == 200 {
        write_back(ctx, id, mine, body_of(reply), "", "", false)?;
        return Ok("")
    }
    if status == 404 || status == 410 {
        return Ok(gone_reason(google_ref))
    }
    if status != 412 {
        return Err(failure("events.patch of " + google_ref, reply))
    }
    // Google changed the event since the snapshot: merge field by field (D10), then send again.
    let fresh = call(ctx, "GET", url, "", Value::Null)?
    if gone(fresh) {
        return Ok(gone_reason(google_ref))
    }
    if status_of(fresh) != 200 {
        return Err(failure("events.get of " + google_ref, fresh))
    }
    let event = body_of(fresh)
    let google = synced(event)
    let google_newer = secs_of(text_of(event, "updated")) >= secs_of(text_of(fields, "updated"))
    let merged = merge(google, snapshot, mine, google_newer)
    let send = differing(merged, google, writable())
    if !send.is_empty() {
        let again = call(ctx, "PATCH", url + updates, text_of(event, "etag"), Value::Map(to_google(merged, send, list_of(event, "attendees"))))?
        if status_of(again) != 200 {
            return Err(failure("events.patch of " + google_ref + " after a merge", again))
        }
        event = body_of(again)
    }
    write_back(ctx, id, mine, event, "", "", false)?;
    Ok("")
}

fn not_connected_reason() -> string {
    "Google is not connected, or its sign-in expired. Connect Google in Settings > Integrations and send again."
}

// Send the vault's change to note `id`. `created` for an after_create run. Answers "" when
// Google took a change, or why nothing was sent.
fn push(id: string, created: bool) -> Result[string, string] {
    let note = match vault::get(id) {
        Ok(n) => n,
        Err(_) => return Ok("The event is no longer in the vault."),
    }
    let body = note.get("note").unwrap()
    let fields = body.get("fields").unwrap_or(Value::Null)
    let blocks = list_of(body, "blocks")
    let mine = note_values(fields, blocks)
    let snapshot = dict_of(blocks, "snapshot")
    let google_ref = text_of(fields, "google_ref")

    if google_ref == "" {
        if value(mine, "start") == "" {
            vault::log("The note has no time yet, so it is not sent to Google.")
            return Ok("The event has no time, so it was not sent to Google.")
        }
        let ctx = connect()?
        if !ctx.ok {
            return Ok(not_connected_reason())
        }
        return insert(ctx, id, fields, mine)
    }

    let changed = differing(mine, snapshot, writable())
    let unchanged = "The event already is as the change asks, so Google had nothing to send."
    if created {
        // The task made this note from Google just now: it holds what Google sent.
        let fresh = text_of(fields, "created") == text_of(fields, "updated") && differing(mine, snapshot, groups()).is_empty()
        if fresh {
            return Ok(unchanged)
        }
        // Restored from the trash: recreate it when Google no longer has it (D20).
        let ctx = connect()?
        if !ctx.ok {
            return Ok(not_connected_reason())
        }
        let reply = call(ctx, "GET", event_url(ctx, calendar_of(ctx, fields), google_ref), "", Value::Null)?
        if gone(reply) {
            return insert(ctx, id, fields, mine)
        }
        if status_of(reply) != 200 {
            return Err(failure("events.get of " + google_ref, reply))
        }
        if changed.is_empty() {
            return Ok(unchanged)
        }
        return send_changes(ctx, id, fields, mine, snapshot, changed)
    }

    // The loop guard: nothing differs from what Google last sent.
    if changed.is_empty() {
        return Ok(unchanged)
    }
    let ctx = connect()?
    if !ctx.ok {
        return Ok(not_connected_reason())
    }
    send_changes(ctx, id, fields, mine, snapshot, changed)
}

// Send the change to note `id`, then settle the Outbox items that asked for it (outbox X5). When
// the note holds `send_updates`, the change came from the Outbox: a write-back records the send
// on its items, which then leave the Outbox (outbox X14); a Google error marks them failed with Google's message and leaves `send_updates` set, so
// that a retry sends it again; a run that sent nothing marks them failed with the reason and
// clears `send_updates`.
fn run(id: string, created: bool) -> Result[(), string] {
    let invited = match vault::get(id) {
        Ok(n) => text_of(n.get("note").unwrap_or(Value::Null).get("fields").unwrap_or(Value::Null), "send_updates") != "",
        Err(_) => false,
    }
    match push(id, created) {
        Ok(reason) => {
            if invited && reason != "" {
                failed_items(id, reason);
                match vault::patch(id, Value::Map(#{ "send_updates": Value::Null }), "") {
                    Ok(_) => {},
                    Err(e) => vault::log("send_updates was not cleared: " + e),
                }
            }
            Ok(())
        },
        Err(e) => {
            if invited && failed_items(id, e) > 0 {
                vault::log("Google refused the Outbox's change: " + e)
                return Ok(())
            }
            Err(e)
        },
    }
}

fn after_create(event: Value) -> Result[(), string] {
    run(text_of(event, "id"), true)
}

fn after_update(event: Value) -> Result[(), string] {
    run(text_of(event, "id"), false)
}

// ---- the Outbox (outbox X5) -----------------------------------------------------------------

// The Outbox items that are sending a change to event note `id`: an `event` item whose `event`
// or `result` is the note and whose status is `sending`.
fn sending_items(id: string) -> List[string] {
    let rows = match vault::query("from outbox where kind == \"event\" && status == \"sending\"") {
        Ok(rows) => rows,
        Err(e) => {
            vault::log("The Outbox was not read: " + e)
            return []
        },
    }
    let out: List[string] = []
    for row in rows {
        let f = row.get("fields").unwrap_or(Value::Null)
        if text_of(f, "event") == id || text_of(f, "result") == id {
            out.push(text_of(row, "id"))
        }
    }
    out
}

// Now as an RFC 3339 time in UTC.
fn now_text() -> string {
    let secs = vault::now() / 1000
    let rest = secs % 86400
    date_of_days(secs / 86400) + fmt("T{:02}:{:02}:{:02}Z", rest / 3600, (rest / 60) % 60, rest % 60)
}

// Record the send of the items sending event note `id`, as Google event `google_id`, with the
// note as their copy. The `outbox_send` handler then removes them (outbox X14).
fn sent_items(id: string, google_id: string) {
    for item in sending_items(id) {
        let f = #{
            "sent_ref": Value::String(google_id),
            "sent_at": Value::String(now_text()),
            "result": Value::String(id),
        }
        match vault::patch(item, Value::Map(f), "") {
            Ok(_) => vault::log("Outbox item " + item + " was sent as " + google_id + "."),
            Err(e) => vault::log("Outbox item " + item + " was not recorded as sent: " + e),
        }
    }
}

// Mark the items sending event note `id` failed with `message`, and answer how many there were.
fn failed_items(id: string, message: string) -> int {
    let n = 0
    for item in sending_items(id) {
        let f = #{ "status": Value::String("failed"), "error": Value::String(message) }
        match vault::patch(item, Value::Map(f), "") {
            Ok(_) => {
                n += 1
            },
            Err(e) => vault::log("Outbox item " + item + " was not marked failed: " + e),
        }
    }
    n
}

// Delete the note's event, or its one occurrence, in Google (D9).
fn after_delete(event: Value) -> Result[(), string] {
    let fields = event.get("note").unwrap_or(Value::Null).get("fields").unwrap_or(Value::Null)
    let google_ref = text_of(fields, "google_ref")
    if google_ref == "" {
        return Ok(())
    }
    let ctx = connect()?
    if !ctx.ok {
        return Ok(())
    }
    let reply = call(ctx, "DELETE", event_url(ctx, calendar_of(ctx, fields), google_ref), "", Value::Null)?
    let status = status_of(reply)
    if status == 200 || status == 204 || status == 404 || status == 410 {
        return Ok(())
    }
    Err(failure("events.delete of " + google_ref, reply))
}
