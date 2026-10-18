use vault
use http
use secrets
use json
use regex

// Google Calendar events become `event` notes (google-sync D7, D8, D9, D10, D12, D16). The task
// `google_calendar` runs this every five minutes once Google is connected.
//
// The settings are the JSON object in the `google_settings` secret: `calendars`, and the
// `endpoints` test seam. The run's state is the JSON object in the `google_calendar_state` secret,
// written with `secrets::set` and by nothing else, so no note history is written for it:
// `sync_tokens` (by calendar id), `window_end`, `last_run`, `last_error` and
// `notified_invalid_grant` (google-sync D21). Without a refresh token Google is not connected,
// and the run ends successfully with a log line.
//
// For each calendar the settings list, events.list runs with the calendar's sync
// token, so only what changed since the last run arrives; the first run, and any run after Google
// answers 410 for an expired token, lists everything. A single event becomes one note, found
// again by `google_ref`. A recurring series becomes one note per occurrence from 30 days ago to
// the window end, a year ahead, fetched with events.instances, each with the series rule as
// text in `series`. A changed occurrence updates its note; a cancelled event, series or
// occurrence moves its notes to the trash. Once a day the window end moves forward and every
// series with a note gets the new days' occurrences.
//
// Each note keeps the synced fields as Google last sent them in its hidden `snapshot`, and
// Google's `etag`. A note the user changed since then merges field by field: a field changed only
// in Google takes Google's value, one changed only in the vault keeps the vault's, and one changed
// on both takes the newer change, the note's `updated` against Google's `updated` (D10).
// `docs/google.md` describes the snapshot.
//
// When Google refuses the refresh token (`invalid_grant`) the run records the error, notifies the
// user once, and ends without failing.

struct Known {
    id: string,
    series: string,
    calendar: string,
    start: string,
}

struct Sync {
    token: string,
    base: string,
    known: Map[string, Known],
    people: Map[string, string],
    rules: Map[string, string],
    created: int,
    changed: int,
    trashed: int,
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

// The string at `key` of `m`, or `fallback` when there is none.
fn text_or(m: Map[string, Value], key: string, fallback: string) -> string {
    match m.get(key) {
        Some(Value::String(s)) => if s == "" { fallback } else { s },
        _ => fallback,
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

// `s` percent-encoded for a URL path segment, query value or form value.
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

// Milliseconds since the epoch as an RFC 3339 time in UTC.
fn iso_of_ms(ms: int) -> string {
    let secs = ms / 1000
    let t = secs % 86400
    date_of_days(secs / 86400) + fmt("T{:02}:{:02}:{:02}Z", t / 3600, t % 3600 / 60, t % 60)
}

// The start of `date` as a Calendar API time bound.
fn bound(date: string) -> string {
    date + "T00:00:00Z"
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

// GET `url`: `{ status, body }` with the body parsed.
fn fetch(sync: Sync, url: string) -> Result[Value, string] {
    let r = http::get(url, #{ "authorization": "Bearer " + sync.token, "accept": "application/json" })?
    let body = json::parse(r.body).unwrap_or(Value::Null)
    Ok(Value::Map(#{ "status": Value::Int(r.status), "body": body }))
}

fn status_of(reply: Value) -> int {
    reply.get("status").unwrap().as_int().unwrap_or(0)
}

fn failure(what: string, reply: Value) -> string {
    let body = reply.get("body").unwrap_or(Value::Null)
    let message = match body.get("error") {
        Some(e) => text_of(e, "message"),
        None => "",
    }
    fmt("Google Calendar {} answered {}: {}", what, status_of(reply), message)
}

fn events_url(sync: Sync, calendar: string) -> string {
    sync.base + "/calendars/" + encode(calendar) + "/events"
}

// Every page of events.list for `calendar`: `{ gone, items, next }`. `gone` is true when Google
// answered 410 for `sync_token`.
fn list_events(sync: Sync, calendar: string, sync_token: string) -> Result[Value, string] {
    let items: List[Value] = []
    let page = ""
    let first = true
    while first || page != "" {
        let url = events_url(sync, calendar) + "?singleEvents=false&maxResults=250"
        if page != "" {
            url = url + "&pageToken=" + encode(page)
        } else if sync_token != "" {
            url = url + "&syncToken=" + encode(sync_token)
        }
        first = false
        let reply = fetch(sync, url)?
        let status = status_of(reply)
        if status == 410 && page == "" {
            return Ok(Value::Map(#{ "gone": Value::Bool(true), "items": Value::List([]), "next": Value::String("") }))
        }
        if status != 200 {
            return Err(failure("events.list of " + calendar, reply))
        }
        let body = reply.get("body").unwrap()
        for item in list_of(body, "items") {
            items.push(item)
        }
        page = text_of(body, "nextPageToken")
        if page == "" {
            return Ok(Value::Map(#{
                "gone": Value::Bool(false),
                "items": Value::List(items),
                "next": Value::String(text_of(body, "nextSyncToken")),
            }))
        }
    }
    Ok(Value::Map(#{ "gone": Value::Bool(false), "items": Value::List(items), "next": Value::String("") }))
}

// The occurrences of series `master` from `from` to `to`, dates; none when the series is gone.
fn instances(sync: Sync, calendar: string, master: string, from: string, to: string) -> Result[List[Value], string] {
    let items: List[Value] = []
    let page = ""
    let first = true
    while first || page != "" {
        let url = events_url(sync, calendar) + "/" + encode(master) + "/instances?maxResults=250" +
            "&timeMin=" + encode(bound(from)) + "&timeMax=" + encode(bound(to))
        if page != "" {
            url = url + "&pageToken=" + encode(page)
        }
        first = false
        let reply = fetch(sync, url)?
        let status = status_of(reply)
        if status == 404 || status == 410 {
            return Ok([])
        }
        if status != 200 {
            return Err(failure("events.instances of " + master, reply))
        }
        let body = reply.get("body").unwrap()
        for item in list_of(body, "items") {
            items.push(item)
        }
        page = text_of(body, "nextPageToken")
    }
    Ok(items)
}

// The recurrence rule of a series master as text, one rule per line.
fn rule_text(master: Value) -> string {
    strings(list_of(master, "recurrence")).join("\n")
}

// ---- the synced fields ----------------------------------------------------------------------

// The snapshot keys, grouped as they merge: `start` and `end` settle together, so a range never
// mixes a date with a datetime.
fn groups() -> List[List[string]] {
    [["title"], ["start", "end"], ["location"], ["description"], ["attendees"], ["organiser"], ["status"], ["meet_url"]]
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

// The synced fields of an event note as it is now.
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

fn group_differs(a: Map[string, string], b: Map[string, string], group: List[string]) -> bool {
    group.any(|k| value(a, k) != value(b, k))
}

fn maps_differ(a: Map[string, string], b: Map[string, string]) -> bool {
    snapshot_keys().any(|k| value(a, k) != value(b, k))
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

// ---- notes ----------------------------------------------------------------------------------

// The person whose `emails` holds `address`, ignoring case, or "".
fn person_for(sync: Sync, address: string) -> Result[string, string] {
    let wanted = address.to_lower()
    if let Some(id) = sync.people.get(wanted) {
        return Ok(id)
    }
    let found = ""
    for row in vault::query("from person where contains(emails, " + quoted(wanted) + ") limit 50")? {
        let emails = strings(list_of(row.get("fields").unwrap_or(Value::Null), "emails"))
        if found == "" && emails.any(|e| e.to_lower() == wanted) {
            found = text_of(row, "id")
        }
    }
    sync.people[wanted] = found
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

// The note body for `values` on top of `fields` and `blocks`, the note's own, empty for a new
// note.
fn body_for(sync: Sync, fields: Map[string, Value], blocks: List[Value], values: Map[string, string], snapshot: Map[string, string], meta: Map[string, string], same_attendees: bool) -> Result[Value, string] {
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
                let id = person_for(sync, a)?
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

// Move the note of `google_ref` to the trash.
fn trash(sync: Sync, google_ref: string) -> Result[(), string] {
    let Some(known) = sync.known.get(google_ref) else {
        return Ok(())
    }
    sync.known.remove(google_ref);
    match vault::get(known.id) {
        Ok(note) => {
            vault::delete(known.id, text_of(note, "version"))?;
            sync.trashed += 1
        }
        Err(_) => {}
    }
    Ok(())
}

// Move every occurrence note of series `master` to the trash.
fn trash_series(sync: Sync, master: string) -> Result[(), string] {
    for r in sync.known.keys() {
        if r.starts_with(master + "_") {
            trash(sync, r)?
        }
    }
    Ok(())
}

// Create or update the note of Google event or occurrence `item` on `calendar`.
fn apply(sync: Sync, item: Value, calendar: string, series: string) -> Result[(), string] {
    let google_ref = text_of(item, "id")
    if google_ref == "" {
        return Ok(())
    }
    if text_of(item, "status") == "cancelled" {
        return trash(sync, google_ref)
    }
    let google = synced(item)
    let meta = #{ "calendar": calendar, "series": series, "google_ref": google_ref, "etag": text_of(item, "etag") }
    let Some(known) = sync.known.get(google_ref) else {
        let body = body_for(sync, #{}, [], google, google, meta, false)?
        let made = vault::create("event", body)?
        sync.known[google_ref] = Known { id: text_of(made, "id"), series: series, calendar: calendar, start: value(google, "start") }
        sync.created += 1
        return Ok(())
    }
    let note = vault::get(known.id)?
    let body = note.get("note").unwrap()
    let fields = map_of(body, "fields")
    let blocks = list_of(body, "blocks")
    let mine = note_values(Value::Map(fields), blocks)
    let snapshot = dict_of(blocks, "snapshot")
    let note_updated = match fields.get("updated") {
        Some(Value::String(s)) => s,
        _ => "",
    }
    let google_newer = secs_of(text_of(item, "updated")) >= secs_of(note_updated)
    let values = merge(google, snapshot, mine, google_newer)
    let unchanged = !maps_differ(values, mine) &&
        !maps_differ(snapshot, google) &&
        text_of(Value::Map(fields), "etag") == meta["etag"] &&
        text_of(Value::Map(fields), "series") == series &&
        text_of(Value::Map(fields), "calendar") == calendar
    known.series = series
    known.calendar = calendar
    known.start = value(values, "start")
    if unchanged {
        return Ok(())
    }
    let same_attendees = value(values, "attendees") == value(mine, "attendees")
    let next = body_for(sync, fields, blocks, values, google, meta, same_attendees)?
    vault::update(known.id, next, text_of(note, "version"))?
    sync.changed += 1
    Ok(())
}

// The rule of series `master`: from a note of one of its occurrences, else from Google.
fn series_rule(sync: Sync, calendar: string, master: string) -> Result[string, string] {
    if let Some(rule) = sync.rules.get(master) {
        return Ok(rule)
    }
    let rule = ""
    for r in sync.known.keys() {
        let k = sync.known[r]
        if rule == "" && r.starts_with(master + "_") && k.series != "" {
            rule = k.series
        }
    }
    if rule == "" {
        let reply = fetch(sync, events_url(sync, calendar) + "/" + encode(master))?
        if status_of(reply) == 200 {
            rule = rule_text(reply.get("body").unwrap())
        }
    }
    sync.rules[master] = rule
    Ok(rule)
}

// Bring the occurrence notes of series `master` from `from` to `to` in step with Google. With
// `prune`, an occurrence note in that span Google no longer lists goes to the trash.
fn refresh_series(sync: Sync, calendar: string, master: string, rule: string, from: string, to: string, prune: bool) -> Result[(), string] {
    sync.rules[master] = rule
    let listed: Map[string, bool] = #{}
    for item in instances(sync, calendar, master, from, to)? {
        listed[text_of(item, "id")] = true
        apply(sync, item, calendar, rule)?
    }
    if prune {
        for r in sync.known.keys() {
            let k = sync.known[r]
            let day = if k.start.len() >= 10 { days_of(k.start.slice(0, 10)) } else { 0 }
            let inside = day >= days_of(from) && day < days_of(to)
            if r.starts_with(master + "_") && !listed.contains_key(r) && inside {
                trash(sync, r)?
            }
        }
    }
    Ok(())
}

// The master id of an occurrence's google_ref: what comes before its last `_`.
fn master_of(google_ref: string) -> string {
    let parts = google_ref.split("_")
    if parts.len() < 2 {
        return ""
    }
    parts.slice(0, parts.len() - 1).join("_")
}

// Sync one calendar from `sync_token`; returns the next sync token.
fn sync_calendar(sync: Sync, calendar: string, sync_token: string, start: string, end: string, old_end: string) -> Result[string, string] {
    let listed = list_events(sync, calendar, sync_token)?
    let full = sync_token == ""
    if listed.get("gone").unwrap().as_bool().unwrap_or(false) {
        vault::log("The sync token of calendar " + calendar + " expired: listing it in full.")
        listed = list_events(sync, calendar, "")?
        full = true
    }
    let items = list_of(listed, "items")
    let refreshed: Map[string, bool] = #{}

    // What Google deleted.
    for item in items {
        if text_of(item, "status") == "cancelled" {
            let id = text_of(item, "id")
            trash(sync, id)?
            trash_series(sync, id)?
        }
    }
    let live = items.filter(|i| text_of(i, "status") != "cancelled")
    // Series masters: their occurrences across the window.
    for item in live {
        if !list_of(item, "recurrence").is_empty() {
            let id = text_of(item, "id")
            trash(sync, id)?
            refresh_series(sync, calendar, id, rule_text(item), start, end, true)?
            refreshed[id] = true
        }
    }
    // Changed occurrences, then single events.
    for item in live {
        let master = text_of(item, "recurringEventId")
        if master != "" {
            let rule = series_rule(sync, calendar, master)?
            apply(sync, item, calendar, rule)?
        } else if list_of(item, "recurrence").is_empty() {
            apply(sync, item, calendar, "")?
        }
    }
    // A full listing names every event there is: a note of this calendar whose event or series
    // it left out was deleted while no token could say so.
    if full {
        let ids: Map[string, bool] = #{}
        for item in live {
            ids[text_of(item, "id")] = true
        }
        for r in sync.known.keys() {
            let k = sync.known[r]
            let gone = if k.series != "" { !ids.contains_key(master_of(r)) && !ids.contains_key(r) } else { !ids.contains_key(r) }
            if k.calendar == calendar && gone {
                trash(sync, r)?
            }
        }
    }
    // The daily window roll: the new days' occurrences of every series with a note.
    if old_end != "" && days_of(old_end) < days_of(end) {
        let masters: Map[string, string] = #{}
        for r in sync.known.keys() {
            let k = sync.known[r]
            let master = master_of(r)
            if k.calendar == calendar && k.series != "" && master != "" && !refreshed.contains_key(master) {
                masters[master] = k.series
            }
        }
        for master in masters.keys() {
            refresh_series(sync, calendar, master, masters[master], old_end, end, false)?
        }
    }
    Ok(text_of(listed, "next"))
}

// The event notes there are, by google_ref.
fn known_notes() -> Result[Map[string, Known], string] {
    let out: Map[string, Known] = #{}
    let cursor = ""
    let first = true
    while first || cursor != "" {
        first = false
        let page = vault::query_page("from event include archived", 1000, cursor)?
        for row in list_of(page, "rows") {
            let fields = row.get("fields").unwrap_or(Value::Null)
            let r = text_of(fields, "google_ref")
            if r != "" {
                let when = fields.get("when").unwrap_or(Value::Null)
                out[r] = Known { id: text_of(row, "id"), series: text_of(fields, "series"), calendar: text_of(fields, "calendar"), start: text_of(when, "start") }
            }
        }
        cursor = text_of(page, "next")
    }
    Ok(out)
}

// ---- settings and state --------------------------------------------------------------------

fn as_map(v: Value) -> Map[string, Value] {
    match v {
        Value::Map(m) => m,
        _ => #{},
    }
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

fn save_state(state: Map[string, Value]) -> Result[(), string] {
    secrets::set("google_calendar_state", json::to_string(Value::Map(state)))
}

fn record_error(state: Map[string, Value], error: string, now: string) -> Result[(), string] {
    state["last_error"] = Value::String(error);
    state["last_run"] = Value::String(now);
    save_state(state)
}

fn run() -> Result[string, string] {
    match secrets::get("google_refresh_token") {
        Ok(_) => {},
        Err(_) => {
            vault::log("Google is not connected: there is no google_refresh_token secret, so there is nothing to sync.")
            return Ok("not connected")
        },
    }
    let fields = secret_json("google_settings")
    let state = as_map(secret_json("google_calendar_state"))
    let endpoints = map_of(fields, "endpoints")
    let token_url = text_or(endpoints, "token", "https://oauth2.googleapis.com/token")
    let base = text_or(endpoints, "calendar", "https://www.googleapis.com/calendar/v3")
    let now = iso_of_ms(vault::now())

    let token = match access_token(token_url) {
        Ok(t) => t,
        Err(e) => {
            if e.starts_with("invalid_grant") {
                // Notify once: a later run finds the flag it wrote and stays quiet.
                let notified = match state.get("notified_invalid_grant") {
                    Some(Value::Bool(b)) => b,
                    _ => false,
                }
                if !notified {
                    vault::notify("Google sign-in expired", "Connect Google again in Settings > Integrations.", "")?;
                }
                state["notified_invalid_grant"] = Value::Bool(true);
                record_error(state, e, now)?
                vault::log("Google refused the refresh token: " + e)
                return Ok("sign-in expired")
            }
            record_error(state, e, now)?
            return Err(e)
        }
    }
    state["notified_invalid_grant"] = Value::Bool(false);

    let sync = Sync {
        token: token,
        base: base,
        known: known_notes()?,
        people: #{},
        rules: #{},
        created: 0,
        changed: 0,
        trashed: 0,
    }
    let calendars = strings(list_of(fields, "calendars"))
    if calendars.is_empty() {
        calendars = ["primary"]
    }
    let today = vault::today()
    let start = add_days(today, -30)
    let end = add_days(today, 365)
    let old_end = text_of(Value::Map(state), "window_end")
    let old_tokens = map_of(Value::Map(state), "sync_tokens")
    let tokens: Map[string, Value] = #{}
    for calendar in calendars {
        let old = match old_tokens.get(calendar) {
            Some(Value::String(t)) => t,
            _ => "",
        }
        match sync_calendar(sync, calendar, old, start, end, old_end) {
            Ok(next) => {
                tokens[calendar] = Value::String(next)
            }
            Err(e) => {
                record_error(state, e, now)?
                return Err(e)
            }
        }
    }
    state["sync_tokens"] = Value::Map(tokens);
    state["window_end"] = Value::String(end);
    state["last_run"] = Value::String(now);
    state["last_error"] = Value::Null;
    save_state(state)?
    let summary = fmt("{} created, {} updated, {} trashed", sync.created, sync.changed, sync.trashed)
    vault::log(summary)
    Ok(summary)
}
