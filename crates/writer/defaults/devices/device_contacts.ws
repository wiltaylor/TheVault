use vault
use device

// Phone contacts and `person` notes stay in step both ways (D16). A person and a contact are
// linked through the person's hidden `device_contacts` dict, which maps `device_<device id>` to
// the contact's id on that device, and the contact holds the person's id (D24). The name, phones
// and addresses sync field by field: the contact also holds what the two sides last agreed on, so
// a field changed on one side only takes that side, and a field changed on both takes the side
// changed most recently. Deleting either side only unlinks the other.
//
// A person changing reaches the phone as a `contact_changed` event for its contact, queued after
// a sync round, so this one handler settles both directions.
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

// The person whose id the contact holds. `Err` when the contact holds an id the vault no longer
// has: the person was deleted, which unlinks the contact rather than bringing the person back.
fn stored(id: string) -> Result[Option[Value], string] {
    if id == "" {
        return Ok(None)
    }
    match vault::get(id) {
        Ok(person) => Ok(Some(person)),
        Err(_) => Err("person " + id + " was deleted"),
    }
}

// Where one field settles. Sides that agree stay. With `base`, what the sides last agreed on, the
// side still holding it gives way to the side that changed; when both changed, or with no base,
// the side changed most recently wins.
fn settle(contact: List[Value], person: List[Value], has_base: bool, base: List[Value], contact_newer: bool) -> List[Value] {
    if same(contact, person) {
        return contact
    }
    if has_base {
        if same(contact, base) {
            return person
        }
        if same(person, base) {
            return contact
        }
    }
    if contact_newer { contact } else { person }
}

// Write the settled fields and the person's id to the phone contact, which also records them as
// what the sides last agreed on. Without the contacts write switch the phone keeps its contact.
fn record(contact_id: string, name: string, phones: List[Value], addresses: List[Value], person: string) {
    let contact = Value::Map(#{
        "contact_id": Value::String(contact_id),
        "name": Value::String(name),
        "phones": Value::List(phones),
        "addresses": Value::List(addresses),
        "person": Value::String(person),
    })
    match device::contacts_upsert(contact) {
        Ok(_) => {}
        Err(e) => vault::log("contact " + contact_id + " not updated: " + e),
    }
}

fn contact_changed(event: Value) -> Result[(), string] {
    let key = slot(text_of(event, "device"))
    let data = event.get("data").unwrap()
    let contact_id = text_of(data, "contact_id")
    let name = text_of(data, "name")
    let phones = list_of(data, "phones")
    let addresses = list_of(data, "addresses")
    let updated = text_of(data, "updated")
    let held = text_of(data, "person")
    let found = match stored(held) {
        Ok(Some(person)) => Some(person),
        Ok(None) => match linked(key, contact_id)? {
            Some(person) => Some(person),
            None => by_phone(phones)?,
        },
        Err(e) => {
            vault::log("contact " + contact_id + " left alone: " + e)
            return Ok(())
        }
    }
    let Some(person) = found else {
        if name == "" && phones.is_empty() {
            return Ok(())
        }
        let links: Map[string, Value] = #{}
        links[key] = Value::String(contact_id);
        let title = if name != "" { name } else { phones[0].as_string().unwrap() }
        let created = vault::create("person", Value::Map(#{
            "fields": Value::Map(#{
                "title": Value::String(title),
                "phones": Value::List(phones),
                "addresses": Value::List(addresses),
            }),
            "blocks": Value::List([links_block(links)]),
        }))?
        record(contact_id, name, phones, addresses, text_of(created, "id"))
        return Ok(())
    }
    let id = text_of(person, "id")
    let fields = person.get("note").unwrap().get("fields").unwrap_or(Value::Null)
    let links = links_of(person)
    let title = text_of(fields, "title")
    let person_phones = list_of(fields, "phones")
    let person_addresses = list_of(fields, "addresses")
    let newer = vault::query("from person where id == " + quoted(id) + " && updated > " + quoted(updated))?
    let contact_newer = updated == "" || newer.len() == 0
    let has_base = match data.get("synced") {
        Some(Value::Map(_)) => true,
        _ => false,
    }
    let base = data.get("synced").unwrap_or(Value::Null)
    // A contact with no name keeps the person's.
    let contact_name = if name != "" { name } else { title }
    let new_name = settle([Value::String(contact_name)], [Value::String(title)], has_base, [Value::String(text_of(base, "name"))], contact_newer)[0].as_string().unwrap_or("")
    let new_phones = settle(phones, person_phones, has_base, list_of(base, "phones"), contact_newer)
    let new_addresses = settle(addresses, person_addresses, has_base, list_of(base, "addresses"), contact_newer)
    let is_linked = text_of(Value::Map(links), key) == contact_id
    links[key] = Value::String(contact_id);
    let changes: Map[string, Value] = #{}
    if new_name != title {
        changes["title"] = Value::String(new_name);
    }
    if !same(new_phones, person_phones) {
        changes["phones"] = Value::List(new_phones);
    }
    if !same(new_addresses, person_addresses) {
        changes["addresses"] = Value::List(new_addresses);
    }
    if !changes.is_empty() || !is_linked {
        save(person, changes, links)?
    }
    let agreed = has_base && text_of(base, "name") == new_name && same(list_of(base, "phones"), new_phones) && same(list_of(base, "addresses"), new_addresses)
    let behind = new_name != name || !same(new_phones, phones) || !same(new_addresses, addresses)
    if behind || !agreed || held != id {
        record(contact_id, new_name, new_phones, new_addresses, id)
    }
    Ok(())
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
