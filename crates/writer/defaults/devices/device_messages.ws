use vault
use device

// SMS, MMS and RCS messages become `message` notes, each in the `conversation` note of its thread
// (D15). A sender or recipient is linked to the `person` holding the number; an RCS sender, who
// has no number, to the person of that name or of the phone contact with that name. An unknown
// number creates no person (D24). Each message is captured once, by its device_ref.
//
// The `send_sms` command sends a message when the device's SMS send switch is on (D23), and
// `show_notification` shows one on the phone. A sent SMS that an Outbox item asked for is linked
// as that item's sent copy (outbox X6).

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

// A string literal for a query.
fn quoted(s: string) -> string {
    let escaped = s.replace("\\", "\\\\").replace("\"", "\\\"").replace("\n", "\\n")
    "\"" + escaped.replace("{{", "{{{{").replace("}}", "}}}}") + "\""
}

// The id of the first note `query` finds, or "".
fn first_id(query: string) -> Result[string, string] {
    let rows = vault::query(query)?
    match rows.get(0) {
        Some(row) => Ok(text_of(row, "id")),
        None => Ok(""),
    }
}

fn captured(device_ref: string) -> Result[bool, string] {
    if device_ref == "" {
        return Ok(false)
    }
    let id = first_id("from message include archived where device_ref == " + quoted(device_ref) + " limit 1")?
    Ok(id != "")
}

fn person_by_number(number: string) -> Result[string, string] {
    if number == "" {
        return Ok("")
    }
    first_id("from person where any(phones, |p| p.number == " + quoted(number) + ") limit 1")
}

// The person an RCS sender's display name belongs to: the person of that name, or the person
// holding a number of the phone contact of that name.
fn person_by_name(name: string) -> Result[string, string] {
    if name == "" {
        return Ok("")
    }
    let id = first_id("from person where title == " + quoted(name) + " limit 1")?
    if id != "" {
        return Ok(id)
    }
    let contacts = match device::contacts_list(Value::Map(#{ "name": Value::String(name) })) {
        Ok(c) => c,
        Err(_) => [],
    }
    for contact in contacts {
        for phone in list_of(contact, "phones") {
            let found = person_by_number(phone.as_string().unwrap_or(""))?
            if found != "" {
                return Ok(found)
            }
        }
    }
    Ok("")
}

// The person behind a party `{ number, number_raw, contact }`, and how to show it.
fn party(p: Value) -> Result[Map[string, string], string] {
    let number = text_of(p, "number")
    let raw = if number != "" { number } else { text_of(p, "number_raw") }
    let contact_name = match p.get("contact") {
        Some(c) => text_of(c, "name"),
        None => "",
    }
    let person = person_by_number(number)?
    let name = if contact_name != "" { contact_name } else { raw }
    Ok(#{ "person": person, "raw": raw, "name": name })
}

// The conversation note of the phone's thread, created on its first message.
fn conversation(thread_ref: string, channel: string, title: string, people: List[string]) -> Result[string, string] {
    if thread_ref == "" {
        return Ok("")
    }
    let rows = vault::query("from conversation where thread_ref == " + quoted(thread_ref) + " limit 1")?
    let Some(row) = rows.get(0) else {
        let ids = people.filter(|p| p != "").map(|p| Value::String(p))
        let made = vault::create("conversation", Value::Map(#{
            "fields": Value::Map(#{
                "title": Value::String(title),
                "channel": Value::String(channel),
                "thread_ref": Value::String(thread_ref),
                "participants": Value::List(ids),
            }),
        }))?
        return Ok(text_of(made, "id"))
    }
    let id = text_of(row, "id")
    let fields = row.get("fields").unwrap_or(Value::Map(#{}))
    let known = list_of(fields, "participants")
    let missing = people.filter(|p| p != "" && !known.any(|k| k.as_string().unwrap_or("") == p))
    if missing.len() > 0 {
        let all = known.concat(missing.map(|p| Value::String(p)))
        vault::patch(id, Value::Map(#{ "participants": Value::List(all) }), "")?;
    }
    Ok(id)
}

// What a message note is called: its first line, or who it is from or to.
fn title_for(body: string, fallback: string) -> string {
    let line = body.trim().split("\n").get(0).unwrap_or("").trim()
    if line == "" {
        fallback
    } else if line.len() > 80 {
        line.slice(0, 79) + "…"
    } else {
        line
    }
}

fn paragraphs(text: string) -> List[Value] {
    text.split("\n")
        .filter(|l| l.trim() != "")
        .map(|l| Value::Map(#{
            "kind": Value::String("paragraph"),
            "fields": Value::Map(#{ "text": Value::String(l) }),
        }))
}

// Create the message note, answering its id.
fn save_message(fields: Map[string, Value], body: string) -> Result[string, string] {
    let note = Value::Map(#{
        "fields": Value::Map(fields),
        "blocks": Value::List([Value::Map(#{
            "kind": Value::String("content"),
            "labels": Value::List([Value::String("body")]),
            "blocks": Value::List(paragraphs(body)),
        })]),
    })
    let made = vault::create("message", note)?
    Ok(text_of(made, "id"))
}

// ---- the Outbox's sent copies (outbox X6) ---------------------------------------------------

// How far apart an Outbox SMS and its sent copy may be, in milliseconds: ten minutes.
fn copy_window_ms() -> int {
    600000
}

// The letters and digits of `s`, lower case: an SMS compared by what it says, whatever markup,
// spacing or line breaks the Outbox item and the phone's copy differ by.
fn words(s: string) -> string {
    let out: List[string] = []
    for c in s.to_lower() {
        let n = int(c)
        if (n >= 48 && n <= 57) || (n >= 97 && n <= 122) || n > 127 {
            out.push(str(c))
        }
    }
    out.join("")
}

fn digits(s: string) -> string {
    let out: List[string] = []
    for c in s {
        let n = int(c)
        if n >= 48 && n <= 57 {
            out.push(str(c))
        }
    }
    out.join("")
}

// Whether two phone numbers are the same: their digits equal, or their last nine digits when both
// have nine or more, so `0400 000 002` is `+61400000002`.
fn same_number(a: string, b: string) -> bool {
    let x = digits(a)
    let y = digits(b)
    if x == "" || y == "" {
        return false
    }
    if x.len() >= 9 && y.len() >= 9 {
        return x.slice(x.len() - 9, x.len()) == y.slice(y.len() - 9, y.len())
    }
    x == y
}

fn days_from_civil(y0: int, m: int, d: int) -> int {
    let y = if m <= 2 { y0 - 1 } else { y0 }
    let era = (if y >= 0 { y } else { y - 399 }) / 400
    let yoe = y - era * 400
    let mp = (m + 9) % 12
    let doy = (153 * mp + 2) / 5 + d - 1
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy
    era * 146097 + doe - 719468
}

fn num(s: string) -> int {
    s.parse_int().unwrap_or(-1)
}

// Milliseconds since the epoch of an ISO 8601 datetime `YYYY-MM-DDTHH:MM:SS[.fff](Z|±HH:MM)`,
// or -1 when `s` is not one.
fn ms_of(s: string) -> int {
    if s.len() < 20 || s.slice(10, 11) != "T" {
        return -1
    }
    let y = num(s.slice(0, 4))
    let mo = num(s.slice(5, 7))
    let d = num(s.slice(8, 10))
    let h = num(s.slice(11, 13))
    let mi = num(s.slice(14, 16))
    let sec = num(s.slice(17, 19))
    if y < 0 || mo < 1 || d < 1 || h < 0 || mi < 0 || sec < 0 {
        return -1
    }
    let rest = s.slice(19, s.len())
    if rest.starts_with(".") {
        let n = 1
        while n < rest.len() && digits(rest.slice(n, n + 1)) != "" {
            n = n + 1
        }
        rest = rest.slice(n, rest.len())
    }
    let offset = if rest == "Z" {
        0
    } else if rest.len() == 6 && (rest.starts_with("+") || rest.starts_with("-")) {
        let oh = num(rest.slice(1, 3))
        let om = num(rest.slice(4, 6))
        if oh < 0 || om < 0 {
            return -1
        }
        (if rest.starts_with("-") { -1 } else { 1 }) * (oh * 3600 + om * 60)
    } else {
        return -1
    }
    ((days_from_civil(y, mo, d) * 86400 + h * 3600 + mi * 60 + sec) - offset) * 1000
}

// The text of an Outbox item's body, its paragraphs one per line.
fn item_text(item: Value) -> string {
    let lines: List[string] = []
    for b in list_of(item.get("note").unwrap_or(Value::Null), "blocks") {
        if text_of(b, "kind") == "content" && list_of(b, "labels").any(|l| l.as_string().unwrap_or("") == "body") {
            for p in list_of(b, "blocks") {
                lines.push(text_of(p.get("fields").unwrap_or(Value::Null), "text"))
            }
        }
    }
    lines.join("\n")
}

// Link message note `message`, an SMS the phone sent, as the sent copy (`result`) of the Outbox
// item it came from: the item whose `sent_ref` is the message's `device_ref`, as the phone
// reported it; or, while the item still waits for the phone's report (or the phone is an older
// app that reports none), the SMS item still sending with no copy, to the same number, saying
// the same, sent within ten minutes of the message. The home server's `outbox_send` handler then
// removes the item, since a sent item leaves the Outbox (outbox X14).
fn link_outbox(message: string, device_ref: string, number: string, body: string, sent: string) -> Result[(), string] {
    if message == "" {
        return Ok(())
    }
    if device_ref != "" {
        let rows = vault::query("from outbox where kind == \"sms\" && sent_ref == " + quoted(device_ref) + " limit 1")?
        match rows.get(0) {
            Some(row) => {
                if text_of(row.get("fields").unwrap_or(Value::Null), "result") == "" {
                    vault::patch(text_of(row, "id"), Value::Map(#{ "result": Value::String(message) }), "")?;
                }
                return Ok(())
            },
            None => {},
        }
    }
    let at = ms_of(sent)
    let said = words(body)
    if at < 0 || said == "" || number == "" {
        return Ok(())
    }
    let rows = vault::query("from outbox where kind == \"sms\" && status == \"sending\" sort updated desc limit 20")?
    for row in rows {
        let f = row.get("fields").unwrap_or(Value::Null)
        if text_of(f, "result") != "" || !same_number(text_of(f, "phone"), number) {
            continue
        }
        let when = ms_of(if text_of(f, "sent_at") != "" { text_of(f, "sent_at") } else { text_of(f, "updated") })
        let apart = if when > at { when - at } else { at - when }
        if when < 0 || apart > copy_window_ms() {
            continue
        }
        let item = match vault::get(text_of(row, "id")) {
            Ok(n) => n,
            Err(_) => continue,
        }
        if words(item_text(item)) == said {
            vault::patch(text_of(row, "id"), Value::Map(#{ "result": Value::String(message) }), "")?;
            return Ok(())
        }
    }
    Ok(())
}

fn ids(people: List[string]) -> Value {
    Value::List(people.filter(|p| p != "").map(|p| Value::String(p)))
}

// An SMS or MMS, received or sent.
fn capture(event: Value, channel: string, direction: string) -> Result[(), string] {
    let data = event.get("data").unwrap()
    let device_ref = text_of(data, "device_ref")
    if captured(device_ref)? {
        return Ok(())
    }
    let them = party(data)?
    let others = list_of(data, "to").map(|p| party(p).unwrap_or(#{ "person": "", "raw": "", "name": "" }))
    let body = text_of(data, "body")
    let subject = text_of(data, "subject")
    let label = if channel == "mms" { "MMS" } else { "SMS" }
    let people = [them["person"]].concat(others.map(|o| o["person"]))
    let fallback = if direction == "in" { label + " from " + them["name"] } else { label + " to " + them["name"] }
    let thread = conversation(text_of(data, "thread_ref"), channel, them["name"], people)?
    let fields = #{
        "title": Value::String(title_for(if subject != "" { subject } else { body }, fallback)),
        "channel": Value::String(channel),
        "direction": Value::String(direction),
        "sent": Value::String(text_of(data, "sent")),
        "device_ref": Value::String(device_ref),
        "partial": Value::Bool(false),
    }
    if direction == "in" {
        fields["from_raw"] = Value::String(them["raw"]);
        if them["person"] != "" {
            fields["from"] = Value::String(them["person"]);
        }
        fields["to"] = ids(others.map(|o| o["person"]));
        fields["to_raw"] = Value::List(others.map(|o| Value::String(o["raw"])));
    } else {
        fields["to"] = ids([them["person"]].concat(others.map(|o| o["person"])));
        fields["to_raw"] = Value::List([Value::String(them["raw"])].concat(others.map(|o| Value::String(o["raw"]))));
    }
    if thread != "" {
        fields["conversation"] = Value::String(thread);
    }
    let assets = list_of(data, "attachments").map(|a| Value::String(text_of(a, "asset"))).filter(|a| a.as_string().unwrap_or("") != "")
    if assets.len() > 0 {
        fields["attachments"] = Value::List(assets);
    }
    let text = if subject != "" && body != "" { subject + "\n" + body } else if subject != "" { subject } else { body }
    let id = save_message(fields, text)?
    if channel == "sms" && direction == "out" {
        // The copy is saved either way; a link that cannot be made is only logged.
        let number = if text_of(data, "number") != "" { text_of(data, "number") } else { them["raw"] }
        match link_outbox(id, device_ref, number, body, text_of(data, "sent")) {
            Ok(_) => {},
            Err(e) => vault::log("The sent SMS was not linked to its Outbox item: " + e),
        }
    }
    Ok(())
}

fn sms_received(event: Value) -> Result[(), string] {
    capture(event, "sms", "in")
}

fn sms_sent(event: Value) -> Result[(), string] {
    capture(event, "sms", "out")
}

fn mms_received(event: Value) -> Result[(), string] {
    capture(event, "mms", "in")
}

// An RCS message read from a Google Messages notification: partial, with a display name and no
// number (D13).
fn rcs_received(event: Value) -> Result[(), string] {
    let data = event.get("data").unwrap()
    let device_ref = text_of(data, "device_ref")
    if captured(device_ref)? {
        return Ok(())
    }
    let sender = text_of(data, "sender")
    let person = person_by_name(sender)?
    let group = text_of(data, "conversation")
    let thread = conversation(text_of(data, "thread_ref"), "rcs", if group != "" { group } else { sender }, [person])?
    let body = text_of(data, "body")
    let fields = #{
        "title": Value::String(title_for(body, "RCS from " + sender)),
        "channel": Value::String("rcs"),
        "direction": Value::String("in"),
        "from_raw": Value::String(sender),
        "sent": Value::String(text_of(data, "sent")),
        "device_ref": Value::String(device_ref),
        "partial": Value::Bool(true),
    }
    if person != "" {
        fields["from"] = Value::String(person);
    }
    if thread != "" {
        fields["conversation"] = Value::String(thread);
    }
    save_message(fields, body)?
    Ok(())
}

// The `send_sms` command: `{ to, body }`.
fn send_sms(command: Value) -> Result[Value, string] {
    let payload = command.get("payload").unwrap()
    let to = text_of(payload, "to")
    if to == "" {
        return Err("send_sms needs a number in to")
    }
    device::sms_send(to, text_of(payload, "body"))
}

// The `show_notification` command: `{ title, body, note }`.
fn show_notification(command: Value) -> Result[Value, string] {
    let payload = command.get("payload").unwrap()
    device::show_notification(text_of(payload, "title"), text_of(payload, "body"), text_of(payload, "note"))
}
