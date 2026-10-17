use vault

// Each call becomes a `call` note linked to the `person` holding the number (D15, D24). An unknown
// number creates no person. Each call is captured once, by its device_ref.

fn text_of(v: Value, key: string) -> string {
    match v.get(key) {
        Some(Value::String(s)) => s,
        _ => "",
    }
}

// A string literal for a query.
fn quoted(s: string) -> string {
    let escaped = s.replace("\\", "\\\\").replace("\"", "\\\"").replace("\n", "\\n")
    "\"" + escaped.replace("{{", "{{{{").replace("}}", "}}}}") + "\""
}

fn first(query: string) -> Result[Option[Value], string] {
    let rows = vault::query(query)?
    Ok(rows.get(0))
}

fn call_ended(event: Value) -> Result[(), string] {
    let data = event.get("data").unwrap()
    let device_ref = text_of(data, "device_ref")
    if device_ref != "" {
        if first("from call include archived where device_ref == " + quoted(device_ref) + " limit 1")?.is_some() {
            return Ok(())
        }
    }
    let number = text_of(data, "number")
    let raw = if number != "" { number } else { text_of(data, "number_raw") }
    let person = if number == "" {
        None
    } else {
        first("from person where has(phones, " + quoted(number) + ") limit 1")?
    }
    let contact_name = match data.get("contact") {
        Some(c) => text_of(c, "name"),
        None => "",
    }
    let name = match person {
        Some(p) => text_of(p, "title"),
        None => if contact_name != "" { contact_name } else if raw != "" { raw } else { "unknown number" },
    }
    let direction = text_of(data, "direction")
    let title = if direction == "in" {
        "Call from " + name
    } else if direction == "out" {
        "Call to " + name
    } else {
        "Missed call from " + name
    }
    let fields = #{
        "title": Value::String(title),
        "direction": Value::String(direction),
        "number": Value::String(raw),
        "started": Value::String(text_of(data, "started")),
        "duration": Value::Int(data.get("duration").unwrap_or(Value::Int(0)).as_int().unwrap_or(0)),
        "device_ref": Value::String(device_ref),
    }
    if let Some(p) = person {
        fields["person"] = Value::String(text_of(p, "id"));
    }
    vault::create("call", Value::Map(#{ "fields": Value::Map(fields) }))?
    Ok(())
}
