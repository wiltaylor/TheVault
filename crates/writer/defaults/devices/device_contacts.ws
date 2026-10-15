use vault
use device

// Phone contacts and `person` notes stay in step both ways (D16). A person and a contact are
// linked through the person's hidden `device_contacts` dict, which maps `device_<device id>` to
// the contact's id on that device (D24). The name, phones and addresses sync; when both sides
// changed, the one changed most recently wins. Deleting either side only unlinks the other.
//
// The `upsert_contact` and `delete_contact` commands change the phone's contacts when the device's
// contacts write switch is on (D23).

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

// The key of a device's link in `device_contacts`: a dict key is an identifier, and a device id,
// a ULID, starts with a digit.
fn slot(device: string) -> string {
    "device_" + device
}

// A string literal for a query.
fn quoted(s: string) -> string {
    let escaped = s.replace("\\", "\\\\").replace("\"", "\\\"").replace("\n", "\\n")
    "\"" + escaped.replace("{{", "{{{{").replace("}}", "}}}}") + "\""
}

fn same(a: List[Value], b: List[Value]) -> bool {
    if a.len() != b.len() {
        return false
    }
    for i in 0..a.len() {
        if a[i].as_string().unwrap_or("") != b[i].as_string().unwrap_or("") {
            return false
        }
    }
    true
}

fn is_links(block: Value) -> bool {
    let label = match list_of(block, "labels").get(0) {
        Some(l) => l.as_string().unwrap_or(""),
        None => "",
    }
    text_of(block, "kind") == "dict" && label == "device_contacts"
}

// A person's links, from the note `vault::get` returns.
fn links_of(person: Value) -> Map[string, Value] {
    let body = person.get("note").unwrap_or(Value::Null)
    for block in list_of(body, "blocks") {
        if is_links(block) {
            return map_of(block, "fields")
        }
    }
    #{}
}

// The JSON form of `links` as the `device_contacts` block.
fn links_block(links: Map[string, Value]) -> Value {
    Value::Map(#{
        "kind": Value::String("dict"),
        "labels": Value::List([Value::String("device_contacts")]),
        "fields": Value::Map(links),
    })
}

// Save person `person` (a note from `vault::get`) with `changes` to its fields and `links` as
// its links.
fn save(person: Value, changes: Map[string, Value], links: Map[string, Value]) -> Result[(), string] {
    let body = person.get("note").unwrap()
    let fields = map_of(body, "fields")
    for key in changes.keys() {
        fields[key] = changes[key];
    }
    let blocks = list_of(body, "blocks").filter(|b| !is_links(b))
    if !links.is_empty() {
        blocks.push(links_block(links))
    }
    let note = Value::Map(#{ "fields": Value::Map(fields), "blocks": Value::List(blocks) })
    vault::update(text_of(person, "id"), note, text_of(person, "version"))?
    Ok(())
}

// The person linked to contact `contact_id` through link `key`, as a note.
fn linked(key: string, contact_id: string) -> Result[Option[Value], string] {
    for row in vault::query("from person")? {
        let person = vault::get(text_of(row, "id"))?
        if text_of(Value::Map(links_of(person)), key) == contact_id {
            return Ok(Some(person))
        }
    }
    Ok(None)
}

fn by_phone(phones: List[Value]) -> Result[Option[Value], string] {
    for phone in phones {
        let rows = vault::query("from person where has(phones, " + quoted(phone.as_string().unwrap_or("")) + ") limit 1")?
        if let Some(row) = rows.get(0) {
            return Ok(Some(vault::get(text_of(row, "id"))?))
        }
    }
    Ok(None)
}

fn contact_changed(event: Value) -> Result[(), string] {
    let key = slot(text_of(event, "device"))
    let data = event.get("data").unwrap()
    let contact_id = text_of(data, "contact_id")
    let name = text_of(data, "name")
    let phones = list_of(data, "phones")
    let addresses = list_of(data, "addresses")
    let updated = text_of(data, "updated")
    let found = match linked(key, contact_id)? {
        Some(person) => Some(person),
        None => by_phone(phones)?,
    }
    let Some(person) = found else {
        if name == "" && phones.is_empty() {
            return Ok(())
        }
        let links: Map[string, Value] = #{}
        links[key] = Value::String(contact_id);
        vault::create("person", Value::Map(#{
            "fields": Value::Map(#{
                "title": Value::String(if name != "" { name } else { phones[0].as_string().unwrap() }),
                "phones": Value::List(phones),
                "addresses": Value::List(addresses),
            }),
            "blocks": Value::List([links_block(links)]),
        }))?
        return Ok(())
    }
    let id = text_of(person, "id")
    let fields = person.get("note").unwrap().get("fields").unwrap_or(Value::Null)
    let links = links_of(person)
    let title = text_of(fields, "title")
    let person_phones = list_of(fields, "phones")
    let person_addresses = list_of(fields, "addresses")
    let differs = (name != "" && name != title) || !same(phones, person_phones) || !same(addresses, person_addresses)
    let is_linked = text_of(Value::Map(links), key) == contact_id
    links[key] = Value::String(contact_id);
    let newer = vault::query("from person where id == " + quoted(id) + " && updated > " + quoted(updated))?
    if updated != "" && newer.len() > 0 {
        // The person changed after the contact: the contact takes the person's fields, when this
        // device may write contacts.
        if !is_linked {
            save(person, #{}, links)?
        }
        if differs {
            let contact = Value::Map(#{
                "contact_id": Value::String(contact_id),
                "name": Value::String(title),
                "phones": Value::List(person_phones),
                "addresses": Value::List(person_addresses),
                "person": Value::String(id),
            })
            match device::contacts_upsert(contact) {
                Ok(_) => {}
                Err(e) => vault::log("contact " + contact_id + " not updated: " + e),
            }
        }
        return Ok(())
    }
    if !differs && is_linked {
        return Ok(())
    }
    let changes = #{
        "phones": Value::List(phones),
        "addresses": Value::List(addresses),
    }
    if name != "" {
        changes["title"] = Value::String(name);
    }
    save(person, changes, links)
}

// A contact deleted on the phone unlinks its person and keeps it.
fn contact_deleted(event: Value) -> Result[(), string] {
    let key = slot(text_of(event, "device"))
    let contact_id = text_of(event.get("data").unwrap(), "contact_id")
    if let Some(person) = linked(key, contact_id)? {
        let links = links_of(person)
        links.remove(key)
        save(person, #{}, links)?
    }
    Ok(())
}

// The `upsert_contact` command: `{ contact_id, name, phones, addresses, person }`. The person is
// linked to the contact it mirrors.
fn upsert_contact(command: Value) -> Result[Value, string] {
    let payload = command.get("payload").unwrap()
    let saved = device::contacts_upsert(payload)?
    let contact_id = text_of(saved, "contact_id")
    let id = text_of(payload, "person")
    if id != "" && contact_id != "" {
        let person = vault::get(id)?
        let links = links_of(person)
        let key = slot(device::id())
        if text_of(Value::Map(links), key) != contact_id {
            links[key] = Value::String(contact_id);
            save(person, #{}, links)?
        }
    }
    Ok(Value::Map(#{ "contact_id": Value::String(contact_id) }))
}

// The `delete_contact` command: `{ contact_id }`. The person it was linked to is kept, unlinked.
fn delete_contact(command: Value) -> Result[Value, string] {
    let contact_id = text_of(command.get("payload").unwrap(), "contact_id")
    if contact_id == "" {
        return Err("delete_contact needs a contact_id")
    }
    device::contacts_delete(contact_id)?
    let key = slot(device::id())
    if let Some(person) = linked(key, contact_id)? {
        let links = links_of(person)
        links.remove(key)
        save(person, #{}, links)?
    }
    Ok(Value::Map(#{}))
}
