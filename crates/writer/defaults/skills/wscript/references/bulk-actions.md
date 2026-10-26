# Bulk actions

`run_script` suits changes to many notes at once. Every write commits on its own and is kept
in note history, so a run that stops halfway leaves the notes it changed changed. Follow this
order:

1. **Look first.** Query the notes and read one or two with `get_note`, so the field names and
   values are right.
2. **Dry run.** Run the script with `dry_run` true. It changes nothing and returns what it would
   change. Show the user the count and a sample.
3. **Apply.** Run the same script with `dry_run` false.
4. **Check.** Query again to confirm.

Make each change safe to repeat: select only the notes still needing it (`where status !=
"Done"`), so running again after a timeout carries on where it stopped.

## Query, then update many notes

Call it with `args` `[true]` for the dry run and `[false]` to apply:

```wscript run args=[true]
use vault

fn main(dry_run: Value) -> Result[Value, string] {
    // An argument arrives as a Value; null or not a bool means a dry run.
    let dry_run = dry_run.as_bool().unwrap_or(true)
    let rows = vault::query("from task where status == \"Waiting\" && !done")?
    let changed = []
    for row in rows {
        let id = row.get("id").unwrap().as_string().unwrap()
        if !dry_run {
            vault::patch(id, Value::Map(#{ "status": Value::String("Todo") }), "")?;
        }
        changed.push(Value::Map(#{
            "id": Value::String(id),
            "title": row.get("title").unwrap(),
        }))
    }
    Ok(Value::Map(#{
        "dry_run": Value::Bool(dry_run),
        "count": Value::Int(changed.len()),
        "notes": Value::List(changed),
    }))
}
```

The examples below set `dry_run` in the code, so they run with `args` `[]`.

## Page through a large set

`vault::query` stops at 10,000 rows. `vault::query_page` reads a page at a time; pass `next` back
until it is null. Return early when the run nears its limit, and report where it stopped.

```wscript run
use vault

fn main() -> Result[Value, string] {
    let dry_run = true
    let cursor = ""
    let seen = 0
    let would_change = 0
    loop {
        let page = vault::query_page("from task sort title asc", 200, cursor)?
        for row in page.get("rows").unwrap().as_list().unwrap() {
            seen += 1
            let fields = row.get("fields").unwrap()
            let priority = fields.get("priority").unwrap_or(Value::Null).as_int().unwrap_or(0)
            if priority < 3 {
                would_change += 1
                if !dry_run {
                    let id = row.get("id").unwrap().as_string().unwrap()
                    vault::patch(id, Value::Map(#{ "priority": Value::Int(3) }), "")?;
                }
            }
        }
        match page.get("next") {
            Some(Value::String(next)) => cursor = next,
            _ => break,
        }
    }
    Ok(Value::Map(#{
        "seen": Value::Int(seen),
        "would_change": Value::Int(would_change),
    }))
}
```

## Guard against concurrent edits

`vault::patch` with `""` writes over the note as it is now. To refuse a note someone changed
since it was read, pass its `version`, and count the conflicts instead of stopping:

```wscript run
use vault

fn main() -> Result[Value, string] {
    let done = 0
    let conflicts = []
    for row in vault::query("from task where done")? {
        let id = row.get("id").unwrap().as_string().unwrap()
        let note = vault::get(id)?
        let version = note.get("version").unwrap().as_string().unwrap()
        match vault::patch(id, Value::Map(#{ "status": Value::String("Done") }), version) {
            Ok(_) => done += 1,
            Err(e) => conflicts.push(Value::String(id + ": " + e)),
        }
    }
    Ok(Value::Map(#{ "done": Value::Int(done), "conflicts": Value::List(conflicts) }))
}
```

## Create many notes from data

```wscript run
use vault
use json

fn main() -> Result[int, string] {
    let data = json::parse("[\"Pay rent\", \"Renew passport\"]")?
    let made = 0
    for item in data.as_list().unwrap() {
        vault::create("task", Value::Map(#{
            "fields": Value::Map(#{
                "title": item,
                "status": Value::String("Todo"),
            }),
        }))?;
        made += 1
    }
    Ok(made)
}
```

## Archive or move to the trash

Archiving keeps a note readable but out of queries; `vault::delete` moves it to the trash, where
the user can restore it. Nothing a script does empties the trash. Dry-run these first too.

```wscript
use vault

fn archive_done(dry_run: Value) -> Result[int, string] {
    let dry_run = dry_run.as_bool().unwrap_or(true)
    let rows = vault::query("from task where done")?
    if !dry_run {
        for row in rows.clone() {
            vault::archive(row.get("id").unwrap().as_string().unwrap())?;
        }
    }
    Ok(rows.len())
}
```
