# Limits and errors

## Limits

| Limit | Value |
|-------|-------|
| Instructions | 50 million per run. Time spent waiting in a host call, such as HTTP, does not count |
| Call depth | 256 nested calls |
| Time | 5 minutes for `run_script`; a tool's `timeout`, 30 seconds by default and at most 300; a task's `timeout`; 30 seconds for a handler |
| HTTP response | 1 MiB of body |
| File or asset read | 16 MiB |
| Query rows | 10,000 per `vault::query`; page larger sets with `vault::query_page` (1 to 1000 rows a page) |
| Log | 64 KiB per run; later lines are dropped |
| Secret value | 64 KiB |

## Error kinds

A failed run reports one kind:

| Kind | Means | Fix |
|------|-------|-----|
| `compile` | The code does not compile: a syntax or type error, an unknown function, or a module the run lacks or has no `use` for | Read the message, which names each problem, and fix the code |
| `entry` | No function of that name, or it takes a different number of arguments than were passed | Check `function` and `args`; a tool function takes exactly one `Value` |
| `returned` | The function returned `Err(message)` | Read the message: often a host call failed, such as a validation error or a version conflict |
| `fault` | A fault stopped the run: `unwrap()` on `None` or `Err`, an index or key that is missing, division by zero, or "type confusion" when an entry function's parameter is not declared `Value` | Use `.get`, `unwrap_or`, `?` or `match` where a value may be missing; declare entry parameters `Value` |
| `instruction_limit` | The run used more than 50 million instructions | A loop that never ends, or too much work in one run: page it |
| `call_depth` | Calls nested deeper than 256 | Turn deep recursion into a loop |
| `timeout` | The run took longer than its time limit | Do less per run, page it, or raise a tool's `timeout` |
| `closed` | The script host shut down | Run it again once the server is up |
| `unavailable` | Scripts do not run here, such as in an app away from the home server | Run it on the home server |

What a run wrote before an error stays written: every vault write commits on its own. Make
bulk changes safe to run again (see `bulk-actions.md`).

```wscript run
fn safe_div(a: int, b: int) -> Result[int, string] {
    if b == 0 { return Err("division by zero") }
    Ok(a / b)
}

fn main() -> Result[int, string] {
    let xs = [1, 2, 3]
    let missing = xs.get(10).unwrap_or(0)       // xs[10] would fault
    let quotient = match safe_div(6, 0) {
        Ok(q) => q,
        Err(_) => -1,
    }
    Ok(missing + quotient + safe_div(6, 3)?)
}
```
