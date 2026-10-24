use vault
use http
use secrets
use json
use regex

// The Gmail sync (google-sync D2, D3, D4, D5, D9, D11, D14, D18, D19). The `google_gmail` task
// runs `run` every 10 minutes. It pulls Gmail messages into `email` notes, grouped into
// `email_thread` notes, with senders and recipients linked to the `person` holding the address.
//
// - The settings are the JSON object in the `google_settings` secret (google-sync D21):
//   `gmail_mode` (`labels` pulls the labels named in `gmail_labels`, `all` pulls all mail),
//   `gmail_window_days` (how far back the first pull reaches) and `gmail_backfill_to` (a date
//   older mail is paged back to, a page each run). Its `endpoints` object points the script at
//   other hosts than Google's, for tests. A missing secret means the defaults.
// - The run's state is the JSON object in the `google_gmail_state` secret, written with
//   `secrets::set` and by nothing else: `history_id`, `listed`, `backfill_page`, `last_run`,
//   `last_error` and `notified_invalid_grant`. No note is written for it.
// - The first run lists the window. Later runs read Gmail's history from `history_id`:
//   new messages are pulled, a label change updates the email's labels and nothing else, and a
//   message deleted in Gmail archives its note. A history id Gmail no longer keeps (404) lists
//   the window again; messages already pulled are skipped, by their `gmail_ref`.
// - `listed` is what the last finished window listing covered: `{ mode, labels, window_days }`,
//   the labels as sorted Gmail ids (google-sync D11, D21). A run whose settings differ lists the
//   window again besides reading history: all of it for a new mode or window, only the added
//   labels otherwise, nothing when labels were only removed. The back-fill starts over too. A
//   run with nothing to list (labels mode, no labels) saves no `history_id`.
// - The body is the text/plain part as paragraphs, or else the text/html part turned into text.
//   The HTML part is kept as an asset in `html`. Attachments up to 700 KB are kept in
//   `attachments`; larger ones are listed by name and size. A message too large to fetch keeps
//   its headers and snippet.
// - Each run refreshes an access token with the three secrets and stores none. Without a refresh
//   token Google is not connected, and the run ends successfully with a log line. A refresh token
//   Google refuses (`invalid_grant`) is written to `last_error`, notified once, and ends
//   the run successfully. Any other failure is written there too and fails the run.
// - A run stops early, before its time or work limit, and leaves the cursor where it was, so the
//   next run carries on.
// - The Outbox's sent emails (outbox X4, X14): each run imports every `outbox` email item that
//   is `sending` with a `sent_ref` (its Gmail message id) and `sent_at` (the send happened) and
//   no `result` yet, whatever the label settings, with the same `save`, and sets the item's
//   `result` to the email note, after which the `outbox_send` handler removes the item: the copy
//   is a regular email. The `outbox` type also runs `copy_sent` after each update of an item, so
//   the copy usually arrives as soon as the send handler records the send.

// The longest a run works before it stops early: the task's timeout is five minutes.
fn time_budget() -> int {
    240000
}

// The most base64 a run decodes, in characters, so a run keeps within its instruction limit.
fn work_budget() -> int {
    1200000
}

// The most base64 of one body part decoded, in characters. The rest is cut.
fn part_cap() -> int {
    120000
}

// The largest attachment kept, in bytes.
fn max_attachment() -> int {
    716800
}

fn day_ms() -> int {
    86400000
}

fn expired_title() -> string {
    "Google sign-in expired"
}

fn expired_body() -> string {
    "Connect Google again in Settings > Integrations."
}

struct Ctx {
    codec: Codec,
    gmail: string,
    auth: Map[string, string],
    labels: Map[string, string],
    terms: Map[string, bool],
    people: Map[string, string],
    started: int,
    work: int,
    stopped: bool,
    imported: int,
    updated: int,
    archived: int,
    failed: int,
    first_failure: string,
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
    list_of(v, key).map(|s| s.as_string().unwrap_or("")).filter(|s| s != "")
}

// A string literal for a query.
fn quoted(s: string) -> string {
    let escaped = s.replace("\\", "\\\\").replace("\"", "\\\"").replace("\n", "\\n")
    "\"" + escaped.replace("{{", "{{{{").replace("}}", "}}}}") + "\""
}

fn first_row(query: string) -> Result[Value, string] {
    let rows = vault::query(query)?
    match rows.get(0) {
        Some(row) => Ok(row),
        None => Ok(Value::Null),
    }
}

// ---- encoding ---------------------------------------------------------------------------------

fn hex() -> string {
    "0123456789ABCDEF"
}

fn hex_byte(b: int) -> string {
    "%" + hex().slice(b / 16, b / 16 + 1) + hex().slice(b % 16, b % 16 + 1)
}

// `s` percent-encoded for a query string or a form body.
fn encode(s: string) -> string {
    let out: List[string] = []
    for ch in s {
        let c = int(ch)
        if (c >= 48 && c <= 57) || (c >= 65 && c <= 90) || (c >= 97 && c <= 122) || c == 45 || c == 46 || c == 95 || c == 126 {
            out.push(str(ch))
        } else if c < 128 {
            out.push(hex_byte(c))
        } else if c < 2048 {
            out.push(hex_byte(192 + c / 64))
            out.push(hex_byte(128 + c % 64))
        } else if c < 65536 {
            out.push(hex_byte(224 + c / 4096))
            out.push(hex_byte(128 + (c / 64) % 64))
            out.push(hex_byte(128 + c % 64))
        } else {
            out.push(hex_byte(240 + c / 262144))
            out.push(hex_byte(128 + (c / 4096) % 64))
            out.push(hex_byte(128 + (c / 64) % 64))
            out.push(hex_byte(128 + c % 64))
        }
    }
    out.join("")
}

fn b64() -> string {
    "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789"
}

fn printable() -> string {
    " !\"#$%&'()*+,-./0123456789:;<=>?@ABCDEFGHIJKLMNOPQRSTUVWXYZ[\\]^_`abcdefghijklmnopqrstuvwxyz{{|}}~"
}

// What decoding needs, built once a run: the value of each base64url character and of each
// pair of them, and how each ASCII byte is written inside a JSON string.
struct Codec {
    single: Map[string, int],
    pairs: Map[string, int],
    ascii: List[string],
}

fn codec() -> Codec {
    let chars = (b64() + "-_").chars().map(|c| str(c))
    let single: Map[string, int] = #{}
    let pairs: Map[string, int] = #{}
    let i = 0
    for a in chars {
        single[a] = i
        let j = 0
        for b in chars {
            pairs[a + b] = i * 64 + j
            j = j + 1
        }
        i = i + 1
    }
    let ascii: List[string] = []
    for b in 0..32 {
        ascii.push(fmt("\\u{:04x}", b))
    }
    for ch in printable() {
        if ch == '"' {
            ascii.push("\\\"")
        } else if ch == '\\' {
            ascii.push("\\\\")
        } else {
            ascii.push(str(ch))
        }
    }
    ascii.push("\\u007f")
    Codec { single: single, pairs: pairs, ascii: ascii }
}

// A code point as a JSON escape.
fn json_escape(cp: int) -> string {
    if cp < 65536 {
        fmt("\\u{:04x}", cp)
    } else {
        let v = cp - 65536
        fmt("\\u{:04x}\\u{:04x}", 55296 + v / 1024, 56320 + v % 1024)
    }
}

// UTF-8 decoding in progress: the JSON fragments so far, and a character's missing bytes.
struct Utf8 {
    parts: List[string],
    need: int,
    cp: int,
}

// Add byte `b`. A byte that is not UTF-8 becomes U+FFFD.
fn utf8_byte(st: Utf8, ascii: List[string], b: int) {
    if st.need > 0 {
        if b >= 128 && b < 192 {
            st.cp = st.cp * 64 + b - 128
            st.need = st.need - 1
            if st.need == 0 {
                st.parts.push(json_escape(st.cp))
            }
            return
        }
        st.parts.push("\\ufffd")
        st.need = 0
    }
    if b < 128 {
        st.parts.push(ascii[b])
    } else if b >= 194 && b < 224 {
        st.need = 1
        st.cp = b - 192
    } else if b >= 224 && b < 240 {
        st.need = 2
        st.cp = b - 224
    } else if b >= 240 && b < 245 {
        st.need = 3
        st.cp = b - 240
    } else {
        st.parts.push("\\ufffd")
    }
}

// Base64 `data`, in either alphabet, padded or not, decoded as UTF-8 text. The characters are
// read in pairs, and three ASCII bytes in a row take a short path, since a run's instructions
// are counted.
fn decode_text(ctx: Ctx, data: string) -> Result[string, string] {
    let clean = data.replace("+", "-").replace("/", "_").replace("=", "").replace("\n", "").replace("\r", "")
    ctx.work = ctx.work + clean.len()
    let c = ctx.codec
    let ascii = c.ascii
    let pairs = regex::find_all("..", clean)
    let quads = clean.len() / 4
    let st = Utf8 { parts: ["\""], need: 0, cp: 0 }
    let parts = st.parts
    let k = 0
    while k < quads {
        let n = c.pairs.get(pairs[2 * k]).unwrap_or(0) * 4096 + c.pairs.get(pairs[2 * k + 1]).unwrap_or(0)
        let b1 = n / 65536
        let b2 = (n / 256) % 256
        let b3 = n % 256
        if st.need == 0 && b1 < 128 && b2 < 128 && b3 < 128 {
            parts.push(ascii[b1])
            parts.push(ascii[b2])
            parts.push(ascii[b3])
        } else {
            utf8_byte(st, ascii, b1)
            utf8_byte(st, ascii, b2)
            utf8_byte(st, ascii, b3)
        }
        k = k + 1
    }
    let rest = clean.len() % 4
    if rest >= 2 {
        let tail = clean.slice(quads * 4, clean.len())
        let n = c.pairs.get(tail.slice(0, 2)).unwrap_or(0)
        if rest == 2 {
            utf8_byte(st, ascii, n / 16)
        } else {
            let v = n * 64 + c.single.get(tail.slice(2, 3)).unwrap_or(0)
            utf8_byte(st, ascii, v / 1024)
            utf8_byte(st, ascii, (v / 4) % 256)
        }
    }
    if st.need > 0 {
        parts.push("\\ufffd")
    }
    parts.push("\"")
    match json::parse(parts.join(""))? {
        Value::String(s) => Ok(s),
        _ => Err("decoding a body part failed"),
    }
}

// Gmail's base64url as the standard base64 assets take.
fn standard_b64(data: string) -> string {
    let s = data.replace("-", "+").replace("_", "/").replace("=", "").replace("\n", "").replace("\r", "")
    let pad = (4 - s.len() % 4) % 4
    s + "=".repeat(pad)
}

// ---- dates ----------------------------------------------------------------------------------

fn days_from_civil(year: int, month: int, day: int) -> int {
    let y = if month <= 2 { year - 1 } else { year }
    let era = y / 400
    let yoe = y - era * 400
    let mp = (month + 9) % 12
    let doy = (153 * mp + 2) / 5 + day - 1
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy
    era * 146097 + doe - 719468
}

// Milliseconds since the epoch of `YYYY-MM-DD` at midnight UTC, or -1.
fn date_ms(date: string) -> int {
    let parts = date.slice(0, 10).split("-")
    if parts.len() != 3 {
        return -1
    }
    let y = parts[0].parse_int().unwrap_or(-1)
    let m = parts[1].parse_int().unwrap_or(-1)
    let d = parts[2].parse_int().unwrap_or(-1)
    if y < 1970 || m < 1 || d < 1 {
        return -1
    }
    days_from_civil(y, m, d) * day_ms()
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
    fmt("{:04}-{}-{}T{}:{}:{}+00:00", y, pad2(m), pad2(d), pad2(rest / 3600), pad2((rest / 60) % 60), pad2(rest % 60))
}

// ---- text -----------------------------------------------------------------------------------

// `text` with every inline markup character escaped, so a body shows as it was sent.
fn literal(text: string) -> string {
    regex::replace("([\\\\*+~`={{}}\\[\\]()|])", text, "\\$1")
}

// HTML entities in `text` decoded.
fn entities(text: string) -> string {
    let t = text.replace("&nbsp;", " ").replace("&lt;", "<").replace("&gt;", ">").replace("&quot;", "\"").replace("&#39;", "'").replace("&apos;", "'")
    for found in regex::find_all("&#[0-9]+;", t) {
        let cp = found.slice(2, found.len() - 1).parse_int().unwrap_or(65533)
        let ch = match json::parse("\"" + json_escape(cp) + "\"") {
            Ok(Value::String(s)) => s,
            _ => "",
        }
        t = t.replace(found, ch)
    }
    t.replace("&amp;", "&")
}

// HTML turned into lines of text.
fn html_text(html: string) -> string {
    let t = regex::replace("(?is)<(script|style|head|title)\\b.*?</(script|style|head|title)>", html, " ")
    t = regex::replace("(?s)<!--.*?-->", t, " ")
    t = regex::replace("(?i)<br\\s*/?>", t, "\n")
    t = regex::replace("(?i)</?(p|div|h[1-6]|li|tr|table|blockquote|pre|ul|ol|section|article|header|footer)\\b[^>]*>", t, "\n")
    t = regex::replace("(?s)<[^>]*>", t, "")
    t = regex::replace("[ \\t\\r\\f]+", t, " ")
    entities(t)
}

fn paragraph(text: string) -> Value {
    Value::Map(#{
        "kind": Value::String("paragraph"),
        "fields": Value::Map(#{ "text": Value::String(text) }),
    })
}

// Each non-blank line of `text` as a paragraph block.
fn paragraphs(text: string) -> List[Value] {
    text.replace("\r\n", "\n").split("\n")
        .map(|l| l.trim())
        .filter(|l| l != "")
        .map(|l| paragraph(literal(l)))
}

fn size_text(size: int) -> string {
    if size >= 1048576 {
        fmt("{:.1} MB", float(size) / 1048576.0)
    } else {
        fmt("{} KB", (size + 1023) / 1024)
    }
}

// ---- addresses and people -------------------------------------------------------------------

// The addresses of a header such as `To`, split on the commas outside quotes and angle brackets.
fn split_addresses(header: string) -> List[string] {
    let out: List[string] = []
    let current: List[string] = []
    let quoted_text = false
    let angle = false
    for ch in header {
        if ch == '"' {
            quoted_text = !quoted_text
        } else if ch == '<' && !quoted_text {
            angle = true
        } else if ch == '>' && !quoted_text {
            angle = false
        }
        if ch == ',' && !quoted_text && !angle {
            let one = current.join("").trim()
            if one != "" {
                out.push(one)
            }
            current = []
        } else {
            current.push(str(ch))
        }
    }
    let last = current.join("").trim()
    if last != "" {
        out.push(last)
    }
    out
}

// The bare address of `Name <addr>` or `addr`, lower case, or "".
fn address_of(raw: string) -> string {
    match regex::captures("<([^>]+)>", raw) {
        Some(groups) => groups[1].trim().to_lower(),
        None => if raw.contains("@") { raw.trim().to_lower() } else { "" },
    }
}

// The person whose `emails` hold the address of `raw`, or "". No person is created (D5).
fn person_of(ctx: Ctx, raw: string) -> Result[string, string] {
    let address = address_of(raw)
    if address == "" {
        return Ok("")
    }
    match ctx.people.get(address) {
        Some(id) => return Ok(id),
        None => {},
    }
    // An address matches ignoring case, however the person stores it.
    let id = text_of(first_row("from person where has_ci(emails, " + quoted(address) + ") limit 1")?, "id")
    ctx.people[address] = id
    Ok(id)
}

fn people_of(ctx: Ctx, raws: List[string]) -> Result[List[string], string] {
    let out: List[string] = []
    for raw in raws {
        let id = person_of(ctx, raw)?
        if id != "" && !out.contains(id) {
            out.push(id)
        }
    }
    Ok(out)
}

fn ids(list: List[string]) -> Value {
    Value::List(list.map(|s| Value::String(s)))
}

// ---- labels ---------------------------------------------------------------------------------

// The term path of each Gmail label id: a user label by its name, Gmail's own under `system`.
fn load_labels(ctx: Ctx) -> Result[List[Value], string] {
    let body = get_json(ctx, ctx.gmail + "/users/me/labels")?
    let labels = list_of(body, "labels")
    for label in labels {
        let name = text_of(label, "name")
        let path = if text_of(label, "type") == "system" {
            "system/" + name.replace("/", "_")
        } else {
            name.split("/").map(|p| p.trim()).filter(|p| p != "").join("/")
        }
        if path != "" && path != "system/" {
            ctx.labels[text_of(label, "id")] = path
        }
    }
    for t in vault::taxonomies()? {
        if text_of(t, "name") == "gmail_label" {
            for term in strings_of(t, "terms") {
                ctx.terms[term.to_lower()] = true
            }
        }
    }
    Ok(labels)
}

// The term paths of Gmail label ids, each term added to `gmail_label` when it is new.
fn terms_of(ctx: Ctx, label_ids: List[string]) -> Result[List[string], string] {
    let out: List[string] = []
    for id in label_ids {
        match ctx.labels.get(id) {
            Some(path) => {
                let parts = path.split("/")
                let prefix = ""
                for part in parts {
                    prefix = if prefix == "" { part } else { prefix + "/" + part }
                    if !ctx.terms.contains_key(prefix.to_lower()) {
                        vault::add_term("gmail_label", prefix)?;
                        ctx.terms[prefix.to_lower()] = true
                    }
                }
                if !out.contains(path) {
                    out.push(path)
                }
            },
            None => {},
        }
    }
    Ok(out)
}

fn same_terms(a: List[string], b: List[string]) -> bool {
    let x = a.map(|s| s.to_lower())
    let y = b.map(|s| s.to_lower())
    x.len() == y.len() && x.all(|s| y.contains(s))
}

// ---- HTTP -----------------------------------------------------------------------------------

// GET `url` with the access token: `{ status, json }`, `json` null when the body is not JSON.
fn fetch(ctx: Ctx, url: string) -> Result[Value, string] {
    let r = http::get(url, ctx.auth)?
    let body = match json::parse(r.body) {
        Ok(v) => v,
        Err(_) => Value::Null,
    }
    Ok(Value::Map(#{ "status": Value::Int(r.status), "json": body }))
}

fn get_json(ctx: Ctx, url: string) -> Result[Value, string] {
    let r = fetch(ctx, url)?
    let status = int_of(r, "status")
    let body = r.get("json").unwrap_or(Value::Null)
    if status != 200 {
        let why = match body.get("error") {
            Some(e) => text_of(e, "message"),
            None => "",
        }
        let path = url.split("?").get(0).unwrap_or(url)
        return Err(fmt("Gmail answered {} for {}: {}", status, path, why))
    }
    Ok(body)
}

// A new access token from the refresh token. `Err` starts with `invalid_grant` when Google
// refuses the refresh token.
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
            return Err("the token endpoint answered no access token")
        }
        return Ok(token)
    }
    let error = text_of(body, "error")
    let why = text_of(body, "error_description")
    if error == "invalid_grant" {
        return Err("invalid_grant: " + why)
    }
    Err(fmt("the token endpoint answered {}: {} {}", r.status, error, why))
}

fn out_of_budget(ctx: Ctx) -> bool {
    if ctx.stopped {
        return true
    }
    if vault::now() - ctx.started > time_budget() || ctx.work > work_budget() {
        ctx.stopped = true
        vault::log("Stopping early, before the run's limits; the next run carries on.")
    }
    ctx.stopped
}

// ---- messages -------------------------------------------------------------------------------

struct Parts {
    text: Value,
    html: Value,
    attachments: List[Value],
}

// The text part, the HTML part and the attachments of a message payload.
fn walk(part: Value, found: Parts) {
    let mime = text_of(part, "mimeType").to_lower()
    let filename = text_of(part, "filename")
    if filename != "" {
        found.attachments.push(part)
    } else if mime == "text/plain" && found.text.is_null() {
        found.text = part
    } else if mime == "text/html" && found.html.is_null() {
        found.html = part
    }
    for child in list_of(part, "parts") {
        walk(child, found)
    }
}

fn header_of(payload: Value, name: string) -> string {
    let wanted = name.to_lower()
    for h in list_of(payload, "headers") {
        if text_of(h, "name").to_lower() == wanted {
            return text_of(h, "value")
        }
    }
    ""
}

// The base64url data of a body part, fetched when Gmail holds it as an attachment. "" when
// it is too large to fetch.
fn part_data(ctx: Ctx, message_id: string, part: Value) -> Result[string, string] {
    let body = part.get("body").unwrap_or(Value::Null)
    let data = text_of(body, "data")
    if data != "" {
        return Ok(data)
    }
    let aid = text_of(body, "attachmentId")
    if aid == "" {
        return Ok("")
    }
    match http::get(ctx.gmail + "/users/me/messages/" + encode(message_id) + "/attachments/" + encode(aid), ctx.auth) {
        Ok(r) => {
            if r.status != 200 {
                return Err(fmt("Gmail answered {} for an attachment of {}", r.status, message_id))
            }
            Ok(text_of(json::parse(r.body)?, "data"))
        },
        Err(e) => if e.contains("larger than") { Ok("") } else { Err(e) },
    }
}

// The thread note of Gmail thread `thread_ref`, created on its first message, with any new
// participants added.
fn thread_note(thread_ref: string, subject: string, people: List[string]) -> Result[string, string] {
    if thread_ref == "" {
        return Ok("")
    }
    let row = first_row("from email_thread include archived where thread_ref == " + quoted(thread_ref) + " limit 1")?
    if row.is_null() {
        let title = regex::replace("^(?i)((re|fwd?|aw)\\s*:\\s*)+", subject, "")
        let made = vault::create("email_thread", Value::Map(#{
            "fields": Value::Map(#{
                "title": Value::String(if title == "" { subject } else { title }),
                "thread_ref": Value::String(thread_ref),
                "participants": ids(people),
            }),
        }))?
        return Ok(text_of(made, "id"))
    }
    let id = text_of(row, "id")
    let fields = row.get("fields").unwrap_or(Value::Null)
    let known = strings_of(fields, "participants")
    let missing = people.filter(|p| !known.contains(p))
    if missing.len() > 0 {
        vault::patch(id, Value::Map(#{ "participants": ids(known.concat(missing)) }), "")?;
    }
    Ok(id)
}

// The stored email of Gmail message `id`, archived ones included, or null.
fn stored(id: string) -> Result[Value, string] {
    first_row("from email include archived where gmail_ref == " + quoted(id) + " limit 1")
}

// Fetch Gmail message `id` and save it as an email note. `Ok(false)` when the run must stop.
fn import(ctx: Ctx, id: string) -> Result[bool, string] {
    if out_of_budget(ctx) {
        return Ok(false)
    }
    if !stored(id)?.is_null() {
        return Ok(true)
    }
    match save(ctx, id) {
        Ok(_) => {},
        Err(e) => {
            ctx.failed = ctx.failed + 1
            if ctx.first_failure == "" {
                ctx.first_failure = e
            }
            vault::log("Message " + id + " was not saved: " + e)
        },
    }
    Ok(true)
}

fn save(ctx: Ctx, id: string) -> Result[(), string] {
    let base = ctx.gmail + "/users/me/messages/" + encode(id)
    ctx.work = ctx.work + 2000
    let too_large = false
    let message = match http::get(base + "?format=full", ctx.auth) {
        Ok(r) => {
            if r.status == 404 {
                return Ok(())
            }
            if r.status != 200 {
                return Err(fmt("Gmail answered {} for message {}", r.status, id))
            }
            json::parse(r.body)?
        },
        Err(e) => {
            if !e.contains("larger than") {
                return Err(e)
            }
            too_large = true
            get_json(ctx, base + "?format=metadata")?
        },
    }
    let payload = message.get("payload").unwrap_or(Value::Null)
    let subject = header_of(payload, "Subject").trim()
    let title = if subject == "" { "(no subject)" } else { subject }
    let from_raw = header_of(payload, "From").trim()
    let to_raw = split_addresses(header_of(payload, "To"))
    let cc_raw = split_addresses(header_of(payload, "Cc"))
    let from = person_of(ctx, from_raw)?
    let to = people_of(ctx, to_raw)?
    let cc = people_of(ctx, cc_raw)?
    let labels = terms_of(ctx, strings_of(message, "labelIds"))?

    let blocks: List[Value] = []
    let html_asset = ""
    let assets: List[Value] = []
    if too_large {
        blocks.push(paragraph("The message was too large to store; only its headers and snippet are kept."))
    } else {
        let found = Parts { text: Value::Null, html: Value::Null, attachments: [] }
        walk(payload, found)
        let html_data = if found.html.is_null() { "" } else { part_data(ctx, id, found.html)? }
        if html_data != "" {
            let asset = vault::upload_asset(id + ".html", standard_b64(html_data))?
            html_asset = text_of(asset, "id")
        }
        let text_data = if found.text.is_null() { "" } else { part_data(ctx, id, found.text)? }
        let cut = false
        let text = if text_data != "" {
            let d = if text_data.len() > part_cap() { cut = true; text_data.slice(0, part_cap()) } else { text_data }
            decode_text(ctx, d)?
        } else if html_data != "" {
            let d = if html_data.len() > part_cap() { cut = true; html_data.slice(0, part_cap()) } else { html_data }
            html_text(decode_text(ctx, d)?)
        } else {
            ""
        }
        blocks = paragraphs(text)
        if cut {
            blocks.push(paragraph("The rest of the message is longer than the sync keeps."))
        }
        let skipped: List[string] = []
        for a in found.attachments {
            let name = text_of(a, "filename")
            let size = int_of(a.get("body").unwrap_or(Value::Null), "size")
            if size > max_attachment() {
                skipped.push(name + " (" + size_text(size) + ")")
            } else {
                let data = part_data(ctx, id, a)?
                if data == "" {
                    skipped.push(name + " (" + size_text(size) + ")")
                } else {
                    let asset = vault::upload_asset(name, standard_b64(data))?
                    assets.push(Value::String(text_of(asset, "id")))
                }
            }
        }
        if skipped.len() > 0 {
            blocks.push(paragraph(literal("Attachments not stored, too large: " + skipped.join(", "))))
        }
    }

    let people: List[string] = []
    for p in [from].concat(to).concat(cc) {
        if p != "" && !people.contains(p) {
            people.push(p)
        }
    }
    let thread = thread_note(text_of(message, "threadId"), title, people)?
    let fields = #{
        "title": Value::String(title),
        "from_raw": Value::String(from_raw),
        "snippet": Value::String(entities(text_of(message, "snippet"))),
        "gmail_ref": Value::String(id),
    }
    if from != "" {
        fields["from"] = Value::String(from)
    }
    if to.len() > 0 {
        fields["to"] = ids(to)
    }
    if cc.len() > 0 {
        fields["cc"] = ids(cc)
    }
    if to_raw.len() > 0 {
        fields["to_raw"] = ids(to_raw)
    }
    if cc_raw.len() > 0 {
        fields["cc_raw"] = ids(cc_raw)
    }
    let internal = int_of(message, "internalDate")
    if internal > 0 {
        fields["sent"] = Value::String(rfc3339(internal))
    }
    if thread != "" {
        fields["thread"] = Value::String(thread)
    }
    if labels.len() > 0 {
        fields["labels"] = ids(labels)
    }
    if html_asset != "" {
        fields["html"] = Value::String(html_asset)
    }
    if assets.len() > 0 {
        fields["attachments"] = Value::List(assets)
    }
    let note = #{ "fields": Value::Map(fields) }
    if blocks.len() > 0 {
        note["blocks"] = Value::List([Value::Map(#{
            "kind": Value::String("content"),
            "labels": Value::List([Value::String("body")]),
            "blocks": Value::List(blocks),
        })])
    }
    vault::create("email", Value::Map(note))?
    ctx.imported = ctx.imported + 1
    Ok(())
}

// Give the stored email of Gmail message `id` the labels `label_ids`, when they changed.
fn relabel(ctx: Ctx, row: Value, label_ids: List[string]) -> Result[(), string] {
    let labels = terms_of(ctx, label_ids)?
    let fields = row.get("fields").unwrap_or(Value::Null)
    if same_terms(labels, strings_of(fields, "labels")) {
        return Ok(())
    }
    let value = if labels.len() > 0 { ids(labels) } else { Value::Null }
    vault::patch(text_of(row, "id"), Value::Map(#{ "labels": value }), "")?
    ctx.updated = ctx.updated + 1
    Ok(())
}

// ---- listing --------------------------------------------------------------------------------

// Whether a message with `label_ids` is one the settings pull.
fn in_scope(wanted: List[string], all: bool, label_ids: List[string]) -> bool {
    if all {
        !label_ids.contains("SPAM") && !label_ids.contains("TRASH")
    } else {
        label_ids.any(|l| wanted.contains(l))
    }
}

fn list_url(ctx: Ctx, label: string, q: string, token: string) -> string {
    let url = ctx.gmail + "/users/me/messages?maxResults=100&q=" + encode(q)
    if label != "" {
        url = url + "&labelIds=" + encode(label)
    }
    if token != "" {
        url = url + "&pageToken=" + encode(token)
    }
    url
}

// Import every message of each listing, the window or a back-fill page. `Ok(false)` when the run
// stopped early.
fn import_page(ctx: Ctx, page: Value) -> Result[bool, string] {
    // Gmail lists newest first: the oldest of a page goes first, so a thread is named by its
    // first message.
    let messages = list_of(page, "messages")
    messages.reverse()
    for m in messages {
        if !import(ctx, text_of(m, "id"))? {
            return Ok(false)
        }
    }
    Ok(true)
}

fn full_list(ctx: Ctx, lists: List[string], q: string) -> Result[bool, string] {
    for label in lists {
        let token = ""
        loop {
            let page = get_json(ctx, list_url(ctx, label, q, token))?
            if !import_page(ctx, page)? {
                return Ok(false)
            }
            token = text_of(page, "nextPageToken")
            if token == "" {
                break
            }
        }
    }
    Ok(true)
}

// Apply Gmail's history since `start`. `Ok("")` when the history id is gone (404), `Ok("stop")`
// when the run stopped early, else the newest history id.
fn incremental(ctx: Ctx, start: string, wanted: List[string], all: bool) -> Result[string, string] {
    let token = ""
    let newest = start
    loop {
        let url = ctx.gmail + "/users/me/history?maxResults=500&startHistoryId=" + encode(start)
        if token != "" {
            url = url + "&pageToken=" + encode(token)
        }
        let r = fetch(ctx, url)?
        let status = int_of(r, "status")
        if status == 404 {
            return Ok("")
        }
        let body = r.get("json").unwrap_or(Value::Null)
        if status != 200 {
            return Err(fmt("Gmail answered {} for its history", status))
        }
        for record in list_of(body, "history") {
            for added in list_of(record, "messagesAdded") {
                let m = added.get("message").unwrap_or(Value::Null)
                if in_scope(wanted, all, strings_of(m, "labelIds")) {
                    if !import(ctx, text_of(m, "id"))? {
                        return Ok("stop")
                    }
                }
            }
            for key in ["labelsAdded", "labelsRemoved"] {
                for change in list_of(record, key) {
                    let m = change.get("message").unwrap_or(Value::Null)
                    let id = text_of(m, "id")
                    let label_ids = strings_of(m, "labelIds")
                    let row = stored(id)?
                    if !row.is_null() {
                        relabel(ctx, row, label_ids)?
                    } else if in_scope(wanted, all, label_ids) {
                        if !import(ctx, id)? {
                            return Ok("stop")
                        }
                    }
                }
            }
            for deleted in list_of(record, "messagesDeleted") {
                let m = deleted.get("message").unwrap_or(Value::Null)
                let row = first_row("from email where gmail_ref == " + quoted(text_of(m, "id")) + " limit 1")?
                if !row.is_null() {
                    vault::archive(text_of(row, "id"))?;
                    ctx.archived = ctx.archived + 1
                }
            }
        }
        let id = text_of(body, "historyId")
        if id != "" {
            newest = id
        }
        token = text_of(body, "nextPageToken")
        if token == "" {
            break
        }
    }
    Ok(newest)
}

// One page of mail older than the window, down to `gmail_backfill_to`. The cursor is
// `<listing>:<page token>` while paging and `done:<date>` once the date is reached.
fn backfill(ctx: Ctx, cursor: string, to: string, lists: List[string], window_start: int) -> Result[string, string] {
    if to == "" || cursor == "done:" + to {
        return Ok(cursor)
    }
    if lists.len() == 0 {
        return Ok("")
    }
    let to_ms = date_ms(to)
    if to_ms < 0 {
        return Ok("done:" + to)
    }
    if to_ms >= window_start {
        // Said once: the cursor is done from here, and a done cursor returns above.
        vault::log(fmt("Gmail: the back-fill date is inside the window; nothing older to fetch ({} is not before the window's start, {}).", to, rfc3339(window_start).slice(0, 10)))
        return Ok("done:" + to)
    }
    let at = cursor.find(":").unwrap_or(-1)
    let index = if at > 0 && !cursor.starts_with("done:") { cursor.slice(0, at).parse_int().unwrap_or(0) } else { 0 }
    let token = if at > 0 && !cursor.starts_with("done:") { cursor.slice(at + 1, cursor.len()) } else { "" }
    if index >= lists.len() {
        return Ok("done:" + to)
    }
    let q = fmt("after:{} before:{}", to_ms / 1000, window_start / 1000)
    let page = get_json(ctx, list_url(ctx, lists[index], q, token))?
    if !import_page(ctx, page)? {
        return Ok(cursor)
    }
    let next = text_of(page, "nextPageToken")
    if next != "" {
        return Ok(fmt("{}:{}", index, next))
    }
    if index + 1 < lists.len() {
        return Ok(fmt("{}:", index + 1))
    }
    Ok("done:" + to)
}

// What a window listing covered (google-sync D11, D21): the mode, the label ids sorted, and the
// window in days.
fn listing_of(all: bool, wanted: List[string], days: int) -> Value {
    let labels: List[string] = []
    for id in wanted {
        if !labels.contains(id) {
            labels.push(id)
        }
    }
    labels.sort()
    Value::Map(#{
        "mode": Value::String(if all { "all" } else { "labels" }),
        "labels": ids(labels),
        "window_days": Value::Int(days),
    })
}

// The listings to run again when the settings moved from listing `before` to `now`: every one
// for a new mode or window, or a listing never recorded; else only the labels added.
fn relist_of(before: Value, now: Value, lists: List[string]) -> List[string] {
    if before.is_null() || text_of(before, "mode") != text_of(now, "mode") || int_of(before, "window_days") != int_of(now, "window_days") {
        return lists
    }
    let had = strings_of(before, "labels")
    let added: List[string] = []
    for label in lists {
        if !had.contains(label) {
            added.push(label)
        }
    }
    added
}

// ---- the run --------------------------------------------------------------------------------

fn endpoint(endpoints: Value, key: string, fallback: string) -> string {
    let url = text_of(endpoints, key)
    let base = if url == "" { fallback } else { url }
    if base.ends_with("/") { base.slice(0, base.len() - 1) } else { base }
}

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
    secrets::set("google_gmail_state", json::to_string(Value::Map(state)))
}

fn now_text() -> string {
    rfc3339(vault::now())
}

fn record_error(state: Map[string, Value], error: string) -> Result[(), string] {
    state["last_run"] = Value::String(now_text());
    state["last_error"] = Value::String(error);
    save_state(state)
}

fn run() -> Result[(), string] {
    let started = vault::now()
    match secrets::get("google_refresh_token") {
        Ok(_) => {},
        Err(_) => {
            vault::log("Google is not connected: there is no google_refresh_token secret, so there is nothing to sync.")
            return Ok(())
        },
    }
    let fields = secret_json("google_settings")
    let state = as_map(secret_json("google_gmail_state"))
    let endpoints = fields.get("endpoints").unwrap_or(Value::Null)
    let token_url = endpoint(endpoints, "token", "https://oauth2.googleapis.com/token")
    let token = match access_token(token_url) {
        Ok(t) => t,
        Err(e) => {
            if e.starts_with("invalid_grant") {
                // The user must connect again: say so once, not on every run (D18).
                let message = e + ". Connect Google again in Settings > Integrations."
                let notified = match state.get("notified_invalid_grant") {
                    Some(Value::Bool(b)) => b,
                    _ => false,
                }
                if !notified {
                    vault::notify(expired_title(), expired_body(), "")?;
                }
                state["notified_invalid_grant"] = Value::Bool(true);
                record_error(state, message)?
                vault::log(message)
                return Ok(())
            }
            record_error(state, e)?
            return Err(e)
        },
    }
    state["notified_invalid_grant"] = Value::Bool(false);
    let ctx = new_ctx(endpoints, token, started)
    match sync(ctx, state, fields) {
        Ok(_) => Ok(()),
        Err(e) => {
            record_error(state, e)?
            Err(e)
        },
    }
}

fn new_ctx(endpoints: Value, token: string, started: int) -> Ctx {
    Ctx {
        codec: codec(),
        gmail: endpoint(endpoints, "gmail", "https://gmail.googleapis.com/gmail/v1"),
        auth: #{ "authorization": "Bearer " + token, "accept": "application/json" },
        labels: #{},
        terms: #{},
        people: #{},
        started: started,
        work: 0,
        stopped: false,
        imported: 0,
        updated: 0,
        archived: 0,
        failed: 0,
        first_failure: "",
    }
}

fn sync(ctx: Ctx, state: Map[string, Value], fields: Value) -> Result[(), string] {
    let all = text_of(fields, "gmail_mode") == "all"
    let labels = load_labels(ctx)?
    let wanted: List[string] = []
    if !all {
        for name in strings_of(fields, "gmail_labels") {
            let lower = name.trim().to_lower()
            match labels.find(|l| text_of(l, "name").to_lower() == lower || text_of(l, "id").to_lower() == lower) {
                Some(l) => wanted.push(text_of(l, "id")),
                None => vault::log("Gmail has no label " + quoted(name) + "."),
            }
        }
    }
    let lists = if all { [""] } else { wanted }
    let days = match fields.get("gmail_window_days") {
        Some(Value::Int(n)) => n,
        _ => 90,
    }
    let window_start = ctx.started - days * day_ms()
    let q = fmt("after:{}", window_start / 1000)

    // What the last finished window listing covered. Settings that differ list again.
    let listing = listing_of(all, wanted, days)
    let before = state.get("listed").unwrap_or(Value::Null)
    let same = !before.is_null() && json::to_string(before) == json::to_string(listing)
    let next_listed = before

    let history = text_of(Value::Map(state), "history_id")
    let next_history = history
    if history != "" {
        let got = incremental(ctx, history, wanted, all)?
        if got == "" {
            vault::log("Gmail no longer has history from " + history + ", so the window is listed again.")
            history = ""
        } else if got != "stop" {
            next_history = got
        }
    }
    if history == "" {
        if lists.len() == 0 {
            // Nothing to list: no history id either, so the first listing happens once there
            // are labels to list.
            vault::log("Gmail has no labels to sync, so nothing is listed.")
            next_history = ""
            next_listed = Value::Null
        } else {
            // The mailbox's history id before the listing, so later changes are read from history.
            let profile = get_json(ctx, ctx.gmail + "/users/me/profile")?
            if full_list(ctx, lists, q)? {
                next_history = text_of(profile, "historyId")
                next_listed = listing
            } else {
                next_history = ""
            }
        }
    } else if !same && !ctx.stopped {
        // The settings changed since the window was listed: history covers only changes after
        // the cursor, so list what the new settings add. Imported messages skip by gmail_ref.
        let relist = relist_of(before, listing, lists)
        if relist.len() > 0 {
            vault::log("The Gmail settings changed, so the window is listed again for them.")
        }
        if full_list(ctx, relist, q)? {
            next_listed = listing
        }
    }

    // New settings page the back-fill again from the start.
    let cursor = if same { text_of(Value::Map(state), "backfill_page") } else { "" }
    let to = text_of(fields, "gmail_backfill_to")
    let next_cursor = if ctx.stopped { cursor } else { backfill(ctx, cursor, to, lists, window_start)? }
    if !ctx.stopped {
        copy_outbox(ctx)
    }

    let error = if ctx.failed > 0 {
        Value::String(fmt("{} messages were not saved. The first: {}", ctx.failed, ctx.first_failure))
    } else {
        Value::Null
    }
    state["last_run"] = Value::String(now_text());
    state["last_error"] = error;
    state["history_id"] = if next_history == "" { Value::Null } else { Value::String(next_history) };
    state["backfill_page"] = if next_cursor == "" { Value::Null } else { Value::String(next_cursor) };
    state["listed"] = next_listed;
    save_state(state)?;
    vault::log(fmt("Gmail: {} new, {} relabelled, {} archived, {} not saved.", ctx.imported, ctx.updated, ctx.archived, ctx.failed))
    Ok(())
}

// ---- the Outbox's sent copies (outbox X4) ---------------------------------------------------

// Whether an `outbox` item, by its fields, is a sent email whose copy is not linked yet.
fn wants_copy(fields: Value) -> bool {
    text_of(fields, "kind") == "email" && text_of(fields, "status") == "sending" && text_of(fields, "sent_ref") != "" && text_of(fields, "sent_at") != "" && text_of(fields, "result") == ""
}

// Import Gmail message `gmail_id`, the sent copy of Outbox item `item`, unless it is stored
// already, and link it as the item's `result`. `Ok(false)` when Gmail has no such message yet.
fn copy_one(ctx: Ctx, item: string, gmail_id: string) -> Result[bool, string] {
    if stored(gmail_id)?.is_null() {
        save(ctx, gmail_id)?
    }
    let row = stored(gmail_id)?
    if row.is_null() {
        vault::log("Gmail has no message " + gmail_id + " for Outbox item " + item + " yet.")
        return Ok(false)
    }
    vault::patch(item, Value::Map(#{ "result": Value::String(text_of(row, "id")) }), "")?;
    Ok(true)
}

// Link the copy of every sent Outbox email that has none yet. A failure is logged, and the next
// run tries again.
fn copy_outbox(ctx: Ctx) {
    let rows = match vault::query("from outbox where kind == \"email\" && status == \"sending\"") {
        Ok(rows) => rows,
        Err(e) => {
            vault::log("The Outbox's sent emails were not read: " + e)
            return
        },
    }
    for row in rows {
        let fields = row.get("fields").unwrap_or(Value::Null)
        if wants_copy(fields) && !out_of_budget(ctx) {
            match copy_one(ctx, text_of(row, "id"), text_of(fields, "sent_ref")) {
                Ok(_) => {},
                Err(e) => vault::log("The sent copy of Outbox item " + text_of(row, "id") + " was not imported: " + e),
            }
        }
    }
}

// The `outbox` type's `after_update` handler: import the sent copy of an item the send handler
// just recorded the send of. Without Google connected, or when the token refresh fails, it leaves the
// copy to the next run of the task.
fn copy_sent(note: Value) -> Result[(), string] {
    let fields = note.get("note").unwrap_or(Value::Null).get("fields").unwrap_or(Value::Null)
    if !wants_copy(fields) {
        return Ok(())
    }
    match secrets::get("google_refresh_token") {
        Ok(_) => {},
        Err(_) => return Ok(()),
    }
    let endpoints = secret_json("google_settings").get("endpoints").unwrap_or(Value::Null)
    let token = match access_token(endpoint(endpoints, "token", "https://oauth2.googleapis.com/token")) {
        Ok(t) => t,
        Err(e) => {
            vault::log("The sent copy is left to the next Gmail sync: " + e)
            return Ok(())
        },
    }
    let ctx = new_ctx(endpoints, token, vault::now())
    load_labels(ctx)?;
    copy_one(ctx, text_of(note, "id"), text_of(fields, "sent_ref"))?;
    Ok(())
}
