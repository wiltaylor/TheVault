use vault
use device

// SMS, MMS and RCS messages become `message` notes, each in the `conversation` note of its thread
// (D15). A sender or recipient is linked to the `person` holding the number; an RCS sender, who
// has no number, to the person of that name or of the phone contact with that name. An unknown
// number creates no person (D24). Each message is captured once, by its device_ref.
//
// The `send_sms` command sends a message when the device's SMS send switch is on (D23), and
// `show_notification` shows one on the phone.

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
    first_id("from person where has(phones, " + quoted(number) + ") limit 1")
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

fn save_message(fields: Map[string, Value], body: string) -> Result[(), string] {
    let note = Value::Map(#{
        "fields": Value::Map(fields),
        "blocks": Value::List([Value::Map(#{
            "kind": Value::String("content"),
            "labels": Value::List([Value::String("body")]),
            "blocks": Value::List(paragraphs(body)),
        })]),
    })
    vault::create("message", note)?
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
    save_message(fields, text)
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
    save_message(fields, body)
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
