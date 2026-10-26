# Writing a unit for `run_script`

`run_script` compiles a whole wscript unit and calls one function of it.

| Input | Meaning |
|-------|---------|
| `code` | The whole unit: `use` lines, helper functions and the entry function |
| `function` | The entry function's name; null is `main` |
| `args` | JSON text of an array, one element per argument of the entry function; null is `[]` |

- The entry function takes as many parameters as `args` has elements, and every one arrives as
  a `Value`, whatever the JSON holds: declare each parameter `Value` and read it with
  `.as_string()`, `.as_int()`, `.as_bool()` and the like. A parameter typed `int` or `string`
  faults with "type confusion". A `main()` with no parameters takes `args` `[]`.
- The function's return value becomes the tool result as JSON. Return `Result[T, string]` and
  `Err(message)` is reported as a failed run with that message. Unit returns `null`.
- `vault::log`, `print` and `println` go to the run's log, which comes back with the result.
- The run has every module: `vault`, `http` to any host, `ai`, `secrets` (any secret by name),
  `json`, `math`, `regex`, `value`, and `fs` without any path. It writes straight to the vault as
  the agent, without a proposal, and every run is recorded with the code's hash.
- The time limit is 5 minutes and the instruction limit 50 million (`limits-and-errors.md`).

The result looks like `{ ok: true, result, log, duration_ms, instructions }`. A failure has
`ok: false`, `error`, `kind` (such as `compile`, `returned` or `timeout`) and the log so far.

## The smallest unit

```wscript run
fn main() -> int {
    6 * 7
}
```

## Arguments

With `args` `["task", 10]` this unit is called with `Value::String("task")` and `Value::Int(10)`:

```wscript run args=["task",10]
use vault

fn main(type_name: Value, limit: Value) -> Result[Value, string] {
    let t = type_name.as_string().unwrap_or("task")
    let n = limit.as_int().unwrap_or(10)
    let rows = vault::query(fmt("from {} sort updated desc limit {}", t, n))?
    Ok(Value::List(rows))
}
```

A `Value` argument takes any JSON, such as `[{"ids": ["01J...", "01K..."], "status": "Done"}]`:

```wscript run args=[{"ids":[],"status":"Done"}]
use vault

fn main(input: Value) -> Result[int, string] {
    let status = input.get("status").unwrap().as_string().unwrap()
    let changed = 0
    for id in input.get("ids").unwrap().as_list().unwrap() {
        vault::patch(id.as_string().unwrap(), Value::Map(#{ "status": Value::String(status) }), "")?;
        changed += 1
    }
    Ok(changed)
}
```

## Returning a summary

Return a `Value` map so the result is easy to read back:

```wscript run
use vault

fn main() -> Result[Value, string] {
    let open = vault::query("from task where !done")?
    let ids = open.map(|r| r.get("id").unwrap())
    vault::log(fmt("{} open tasks", open.len()))
    Ok(Value::Map(#{
        "count": Value::Int(open.len()),
        "ids": Value::List(ids),
    }))
}
```

## Another entry function

With `function` `"report"`, the unit's `report` runs; `main` need not exist:

```wscript
use vault

fn report() -> Result[string, string] {
    let total = vault::query_page("from task", 1, "")?.get("total").unwrap().as_int().unwrap()
    Ok(fmt("{} tasks", total))
}
```

## Fixing a failed run

- `compile`: the message names the problem, such as an unknown function, a type mismatch, or a
  module without its `use` line. Fix the code and send it again.
- `entry`: the function is missing, or `args` has the wrong number of elements.
- `returned`: the function returned `Err(message)`: a host call failed, such as a write refused
  by validation or a version conflict.
- `fault`: a fault such as `unwrap()` on `None` or an index out of range; the message says which.
- `timeout` and `instruction_limit`: the run did too much. Page the work and run it again from
  where it stopped (`bulk-actions.md`).
