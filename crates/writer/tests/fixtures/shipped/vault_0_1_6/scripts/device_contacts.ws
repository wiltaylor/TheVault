use vault
use device
use json
use regex

// Phone contacts and `person` notes stay in step both ways (D16). A person and a contact are
// linked through the person's hidden `device_contacts` dict, which maps `device_<device id>` to
// the contact's id on that device, and the contact holds the person's id (D24). Every detail a
// contact holds syncs field by field: the name; the phones, emails and postal addresses, each with
// its labels as one field; organisation, job title, nickname, birthday, websites and the contact
// note; and the photo, compared by its hash. The contact also holds what the two sides last agreed
// on, so a field changed on one side only takes that side, and a field changed on both takes the
// side changed most recently. A detail the event does not report, as from an older app, is left
// as the person has it. Deleting either side only unlinks the other.
//
// A linked person changing reaches the phone as a `contact_changed` event for its contact, queued
// after a sync round, so this one handler settles both directions. A person with no contact here
// yet reaches the phone as an `upsert_contact` command the home server queues.
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
        if changes[key].is_null() {
            fields.remove(key);
        } else {
            fields[key] = changes[key];
        }
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

// The lists a contact holds with a label list beside each, as `[values, labels]`. A list and its
// labels settle as one field.
fn labelled() -> List[List[string]] {
    [["phones", "phone_labels"], ["emails", "email_labels"], ["addresses", "address_labels"]]
}

// The single-string details a contact holds.
fn details() -> List[string] {
    ["organisation", "job_title", "nickname", "birthday", "contact_note"]
}

fn has(v: Value, key: string) -> bool {
    match v.get(key) {
        Some(_) => true,
        None => false,
    }
}

fn same(a: Value, b: Value) -> bool {
    json::to_string(a) == json::to_string(b)
}

fn strings(items: List[Value]) -> List[string] {
    items.map(|v| v.as_string().unwrap_or(""))
}

fn values(items: List[string]) -> List[Value] {
    items.map(|s| Value::String(s))
}

// `labels` cut or padded with "" to one per value.
fn pad(labels: List[string], n: int) -> List[string] {
    let out: List[string] = []
    for i in 0..n {
        out.push(labels.get(i).unwrap_or(""))
    }
    out
}

// The labels of `items` when the side reported none: each value keeps the person's label for it.
fn labels_from(items: List[string], person_items: List[string], person_labels: List[string]) -> List[string] {
    items.map(|v| match person_items.index_of(v) {
        Some(i) => person_labels.get(i).unwrap_or(""),
        None => "",
    })
}

fn pair(items: List[string], labels: List[string]) -> Value {
    Value::List([Value::List(values(items)), Value::List(values(labels))])
}

fn pair_items(p: Value) -> List[string] {
    strings(p.as_list().unwrap()[0].as_list().unwrap())
}

fn pair_labels(p: Value) -> List[string] {
    strings(p.as_list().unwrap()[1].as_list().unwrap())
}

// A list and its labels from `v`, the labels taken from the person's where `v` reports none.
fn pair_of(v: Value, field: List[string], person_items: List[string], person_labels: List[string]) -> Value {
    let items = strings(list_of(v, field[0]))
    let labels = if has(v, field[1]) { pad(strings(list_of(v, field[1])), items.len()) } else { labels_from(items, person_items, person_labels) }
    pair(items, labels)
}

// A website as the person keeps it: one without a scheme gets "https://", and one that still is
// not a URL is "", which drops it.
fn website(raw: string) -> string {
    let s = raw.trim()
    if s == "" {
        return ""
    }
    let full = if regex::is_match("^[A-Za-z][A-Za-z0-9+.-]*://", s) || regex::is_match("^(mailto|tel|sms):", s) { s } else { "https://" + s }
    if regex::is_match("^[A-Za-z][A-Za-z0-9+.-]*:(//[^/?#\\s]|[^/\\s])\\S*$", full) { full } else { "" }
}

fn websites(v: Value) -> Value {
    Value::List(values(strings(list_of(v, "websites")).map(|w| website(w)).filter(|w| w != "")))
}

fn is_empty(v: Value) -> bool {
    let s = json::to_string(v)
    s == "\"\"" || s == "[]" || s == "null" || s == "[[],[]]"
}

// Which side a field settles on: `true` for the contact. Sides that agree stay. With `base`, what
// the sides last agreed on, the side still holding it gives way to the side that changed; when
// both changed, the side changed most recently wins. With no base the sides never agreed on the
// field, so a side without it gives way, and otherwise the more recent side wins.
fn contact_wins(contact: Value, person: Value, has_base: bool, base: Value, contact_newer: bool) -> bool {
    if same(contact, person) {
        return true
    }
    if has_base {
        if same(contact, base) {
            return false
        }
        if same(person, base) {
            return true
        }
    } else {
        if is_empty(person) {
            return true
        }
        if is_empty(contact) {
            return false
        }
    }
    contact_newer
}

// Whether a settled field needs no recording: it is the agreed value, or with none agreed yet,
// both sides already held it.
fn agrees(has_base: bool, base: Value, contact: Value, person: Value, settled: Value) -> bool {
    if has_base { same(base, settled) } else { same(contact, person) }
}

// What a field change writes to the person: an empty value removes the field.
fn put(changes: Map[string, Value], key: string, v: Value) {
    changes[key] = if is_empty(v) { Value::Null } else { v };
}

// The photo's key for settling: the contact's photo by its hash, the person's by the asset it
// names, which is the agreed one's hash when it is the asset they agreed on.
fn photo_hash(v: Value) -> string {
    match v.get("photo") {
        Some(Value::Map(m)) => match m.get("hash") {
            Some(Value::String(h)) => h,
            _ => "",
        },
        _ => "",
    }
}

fn photo_asset(v: Value) -> string {
    match v.get("photo") {
        Some(Value::Map(m)) => match m.get("asset") {
            Some(Value::String(a)) => a,
            _ => "",
        },
        _ => "",
    }
}

fn person_photo_key(asset: string, contact: Value, base: Value) -> string {
    if asset == "" {
        return ""
    }
    if asset == photo_asset(base) {
        return photo_hash(base)
    }
    if asset == photo_asset(contact) {
        return photo_hash(contact)
    }
    "asset:" + asset
}

// The event's contact with an older app's keys read as today's: an app from before emails had
// their own key reported email addresses as `addresses`, and knows no postal addresses.
fn current_keys(v: Value) -> Value {
    let m = match v {
        Value::Map(m) => m,
        _ => return v,
    }
    if !m.contains_key("emails") && m.contains_key("addresses") {
        m["emails"] = m["addresses"];
        m.remove("addresses");
        m.remove("address_labels");
    }
    Value::Map(m)
}

// The person fields a new contact makes.
fn new_person(title: string, data: Value) -> Map[string, Value] {
    let fields: Map[string, Value] = #{ "title": Value::String(title) }
    for field in labelled() {
        let p = pair_of(data, field, [], [])
        if !is_empty(p) {
            fields[field[0]] = Value::List(values(pair_items(p)));
            fields[field[1]] = Value::List(values(pair_labels(p)));
        }
    }
    for key in details() {
        let v = text_of(data, key)
        if v != "" {
            fields[key] = Value::String(v);
        }
    }
    let sites = websites(data)
    if !is_empty(sites) {
        fields["websites"] = sites;
    }
    let photo = photo_asset(data)
    if photo != "" {
        fields["photo"] = Value::String(photo);
    }
    fields
}

// The contact as the person now holds it: every detail key of the `upsert_contact` payload.
fn contact_of(fields: Value) -> Map[string, Value] {
    let out: Map[string, Value] = #{ "name": Value::String(text_of(fields, "title")) }
    for field in labelled() {
        let items = strings(list_of(fields, field[0]))
        out[field[0]] = Value::List(values(items));
        out[field[1]] = Value::List(values(pad(strings(list_of(fields, field[1])), items.len())));
    }
    for key in details() {
        out[key] = Value::String(text_of(fields, key));
    }
    out["websites"] = Value::List(list_of(fields, "websites"));
    let photo = text_of(fields, "photo")
    out["photo"] = if photo == "" { Value::Null } else { Value::String(photo) };
    out
}

// Write `contact`, the settled details, and the person's id to the phone contact, which also
// records them as what the sides last agreed on. An app from before emails had their own key
// takes them as `addresses`. Without the contacts write switch the phone keeps its contact.
fn record(contact_id: string, contact: Map[string, Value], person: string, legacy: bool) {
    contact["contact_id"] = Value::String(contact_id);
    contact["person"] = Value::String(person);
    if legacy {
        contact["addresses"] = contact["emails"];
    }
    match device::contacts_upsert(Value::Map(contact)) {
        Ok(_) => {}
        Err(e) => vault::log("contact " + contact_id + " not updated: " + e),
    }
}

fn contact_changed(event: Value) -> Result[(), string] {
    let key = slot(text_of(event, "device"))
    let raw = event.get("data").unwrap()
    let legacy = !has(raw, "emails") && has(raw, "addresses")
    let data = current_keys(raw)
    let contact_id = text_of(data, "contact_id")
    let name = text_of(data, "name")
    let phones = list_of(data, "phones")
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
            "fields": Value::Map(new_person(title, data)),
            "blocks": Value::List([links_block(links)]),
        }))?
        let id = text_of(created, "id")
        let fields = created.get("note").unwrap().get("fields").unwrap_or(Value::Null)
        let contact = contact_of(fields)
        // The contact keeps its own name when it has one, and its photo.
        contact["name"] = Value::String(name);
        if has(data, "photo") {
            contact["photo"] = if photo_asset(data) == "" { Value::Null } else { Value::String(photo_asset(data)) };
        }
        record(contact_id, contact, id, legacy)
        return Ok(())
    }
    let id = text_of(person, "id")
    let fields = person.get("note").unwrap().get("fields").unwrap_or(Value::Null)
    let links = links_of(person)
    let title = text_of(fields, "title")
    let newer = vault::query("from person where id == " + quoted(id) + " && updated > " + quoted(updated))?
    let contact_newer = updated == "" || newer.len() == 0
    let synced = match data.get("synced") {
        Some(Value::Map(_)) => current_keys(data.get("synced").unwrap()),
        _ => Value::Null,
    }
    let has_synced = !synced.is_null()
    let changes: Map[string, Value] = #{}
    // `behind`: the contact lacks a settled value. `agreed`: every settled value is the agreed one.
    let behind = false
    let agreed = has_synced

    // The name. A contact with no name keeps the person's.
    let contact_name = if name != "" { name } else { title }
    let base_name = Value::String(text_of(synced, "name"))
    let has_base = has_synced && has(synced, "name")
    let new_name = if contact_wins(Value::String(contact_name), Value::String(title), has_base, base_name, contact_newer) { contact_name } else { title }
    if new_name != title {
        changes["title"] = Value::String(new_name);
    }
    behind = behind || new_name != name
    agreed = agreed && agrees(has_base, base_name, Value::String(contact_name), Value::String(title), Value::String(new_name))

    // Each list with its labels.
    for field in labelled() {
        if !has(data, field[0]) {
            continue
        }
        let person_items = strings(list_of(fields, field[0]))
        let person_labels = pad(strings(list_of(fields, field[1])), person_items.len())
        let c = pair_of(data, field, person_items, person_labels)
        // A person that never had labels for the same values takes the contact's.
        let p = if person_labels.all(|l| l == "") && same(Value::List(values(person_items)), Value::List(values(pair_items(c)))) { c } else { pair(person_items, person_labels) }
        let has_base = has_synced && has(synced, field[0])
        let b = pair_of(synced, field, person_items, person_labels)
        let settled = if contact_wins(c, p, has_base, b, contact_newer) { c } else { p }
        if !same(settled, pair(person_items, person_labels)) {
            put(changes, field[0], Value::List(values(pair_items(settled))));
            put(changes, field[1], Value::List(values(pair_labels(settled))));
        }
        behind = behind || !same(settled, c)
        agreed = agreed && agrees(has_base, b, c, p, settled)
    }

    // The single details.
    for detail in details() {
        if !has(data, detail) {
            continue
        }
        let c = Value::String(text_of(data, detail))
        let p = Value::String(text_of(fields, detail))
        let has_base = has_synced && has(synced, detail)
        let b = Value::String(text_of(synced, detail))
        let settled = if contact_wins(c, p, has_base, b, contact_newer) { c } else { p }
        if !same(settled, p) {
            put(changes, detail, settled);
        }
        behind = behind || !same(settled, c)
        agreed = agreed && agrees(has_base, b, c, p, settled)
    }

    // The websites.
    if has(data, "websites") {
        let c = websites(data)
        let p = Value::List(list_of(fields, "websites"))
        let has_base = has_synced && has(synced, "websites")
        let b = websites(synced)
        let settled = if contact_wins(c, p, has_base, b, contact_newer) { c } else { p }
        if !same(settled, p) {
            put(changes, "websites", settled);
        }
        behind = behind || !same(settled, c)
        agreed = agreed && agrees(has_base, b, c, p, settled)
    }

    // The photo, by its hash.
    let person_photo = text_of(fields, "photo")
    let new_photo = person_photo
    if has(data, "photo") {
        let c = Value::String(photo_hash(data))
        let p = Value::String(person_photo_key(person_photo, data, synced))
        let has_base = has_synced && has(synced, "photo")
        let b = Value::String(photo_hash(synced))
        let wins = contact_wins(c, p, has_base, b, contact_newer)
        // The same photo, even as another asset, keeps the person's.
        if wins && !same(c, p) {
            new_photo = photo_asset(data)
        }
        let settled = if wins { c } else { p }
        if new_photo != person_photo {
            put(changes, "photo", Value::String(new_photo));
        }
        behind = behind || !same(settled, c)
        agreed = agreed && agrees(has_base, b, c, p, settled)
    }

    let is_linked = text_of(Value::Map(links), key) == contact_id
    links[key] = Value::String(contact_id);
    if !changes.is_empty() || !is_linked {
        save(person, changes, links)?
    }
    if behind || !agreed || held != id {
        let settled = fields.as_map().unwrap_or(#{})
        for k in changes.keys() {
            settled[k] = changes[k];
        }
        record(contact_id, contact_of(Value::Map(settled)), id, legacy)
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

// The `upsert_contact` command: `{ contact_id, person }` with every detail key of the contact.
// The person is linked to the contact it mirrors. The home server queues one for a person no
// contact here mirrors yet, which may run before the person has synced to the phone: the contact
// still holds the person's id, and the server links the person when the command reports done.
fn upsert_contact(command: Value) -> Result[Value, string] {
    let payload = command.get("payload").unwrap()
    let saved = device::contacts_upsert(payload)?
    let contact_id = text_of(saved, "contact_id")
    let id = text_of(payload, "person")
    if id != "" && contact_id != "" {
        match vault::get(id) {
            Ok(person) => {
                let links = links_of(person)
                let key = slot(device::id())
                if text_of(Value::Map(links), key) != contact_id {
                    links[key] = Value::String(contact_id);
                    save(person, #{}, links)?
                }
            }
            Err(_) => vault::log("person " + id + " has not synced here yet; the home server links it"),
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
