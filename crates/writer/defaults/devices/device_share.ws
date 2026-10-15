use vault

// What another app shares to TheVault becomes a fleeting note in the inbox: the text, the link,
// the shared images, and any other shared file as an attachment.

fn text_of(v: Value, key: string) -> string {
    match v.get(key) {
        Some(Value::String(s)) => s,
        _ => "",
    }
}

fn paragraph(text: string) -> Value {
    Value::Map(#{
        "kind": Value::String("paragraph"),
        "fields": Value::Map(#{ "text": Value::String(text) }),
    })
}

fn share_received(event: Value) -> Result[(), string] {
    let data = event.get("data").unwrap()
    let text = text_of(data, "text")
    let subject = text_of(data, "subject")
    let url = text_of(data, "url")
    let attachments = match data.get("attachments") {
        Some(Value::List(items)) => items,
        _ => [],
    }
    let lines = text.split("\n").filter(|l| l.trim() != "")
    let blocks = lines.map(|l| paragraph(l))
    if url != "" && !text.contains(url) {
        blocks.push(paragraph(url))
    }
    let images = attachments
        .filter(|a| text_of(a, "mime").starts_with("image/"))
        .map(|a| Value::String(text_of(a, "asset")))
    for a in attachments.filter(|a| !text_of(a, "mime").starts_with("image/")) {
        blocks.push(Value::Map(#{
            "kind": Value::String("asset"),
            "fields": Value::Map(#{ "asset": Value::String(text_of(a, "asset")) }),
        }))
    }
    let first_line = lines.get(0).unwrap_or("").trim()
    let title = if subject != "" {
        subject
    } else if first_line != "" {
        if first_line.len() > 80 { first_line.slice(0, 79) + "…" } else { first_line }
    } else if attachments.len() > 0 {
        text_of(attachments[0], "filename")
    } else {
        "Shared"
    }
    let fields = #{ "title": Value::String(title) }
    if images.len() > 0 {
        fields["image"] = Value::List(images);
    }
    vault::create("fleeting", Value::Map(#{
        "fields": Value::Map(fields),
        "blocks": Value::List([Value::Map(#{
            "kind": Value::String("content"),
            "labels": Value::List([Value::String("body")]),
            "blocks": Value::List(blocks),
        })]),
    }))?
    Ok(())
}
