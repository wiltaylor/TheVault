// The `import_history` command: `{ kinds, since, until }`. The phone reads its SMS, MMS, calls
// and contacts again over that range of dates and runs each as a device event, so the handlers
// above decide the notes. A message or call that already has a note is left alone; one whose
// note was deleted gets a note again.
//
// The function answers the import to run. Returning the payload runs what was asked; a vault can
// narrow it here, for example to leave contacts out.
fn import_history(command: Value) -> Result[Value, string] {
    match command.get("payload") {
        Some(payload) => Ok(payload),
        None => Ok(Value::Map(#{})),
    }
}
