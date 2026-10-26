# Writing a script tool

A script tool is a tool any agent can call, made of two files: `tools/<name>.wcl`, which says
what the tool is, and the script it runs, `scripts/<script>.ws`. Propose both together with
`propose_tool_change { name, wcl, script_source }`. Read existing ones with `list_tools` and
`get_tool`. A skill can hold tools too: `skills/<skill>/tools/<name>.wcl` with its script in
`skills/<skill>/scripts/<script>.ws`, offered as `<skill>__<name>` once the skill is in use.

## The tool file

```wcl
description   = "Count the open tasks with a status, and list their titles."
script        = "count_status"
function      = "run"
timeout       = 60
allow_domains = ["api.example.com"]
secrets       = ["example_api_key"]

param status {
  kind        = "string"
  description = "The status to count, such as Todo."
  required    = true
}

param limit {
  kind        = "int"
  description = "The most titles to list; 10 when not given."
}

param names {
  kind        = "list"
  of          = "string"
  description = "Extra titles to look for."
}
```

| Item | Required | Meaning |
|------|----------|---------|
| `description` | yes | What the model reads to decide when to call the tool. Say what it does and returns |
| `script` | yes | The script's name, the stem of `scripts/<name>.ws` |
| `function` | yes | The function the tool calls |
| `allow_domains` | no | The hosts `http` may reach; `["*"]` is any host. None when absent |
| `secrets` | no | The secrets `secrets::get` and `secrets::set` may use. None when absent |
| `timeout` | no | Seconds, 1 to 300; 30 when absent |
| `param <name> { ... }` | no | One block per parameter |

A param has `kind`, one of `string`, `int`, `float`, `bool`, `json` and `list` (with `of`:
`string`, `int`, `float` or `bool`, `string` when absent), a `description`, and `required`
(`false` when absent). The tool's name is a lowercase identifier, `[a-z][a-z0-9_]*`, and cannot
be the name of a built-in tool.

The smallest tool takes no params:

```wcl
description = "Answer how many notes the vault holds."
script      = "note_count"
function    = "run"
```

## The script

The function takes one argument, a `Value` map holding every param by name. An optional param
the model left out is `Value::Null`, so read it with a default. A `json` param arrives parsed,
as any `Value`. Adding a param later does not change the function's signature.

The return value goes back to the model as JSON; `Err(message)` is a tool error the model sees.
The tool writes to the vault as the agent that calls it, at once: write only what the
description promises.

```wscript
use vault

fn run(params: Value) -> Result[Value, string] {
    let status = params.get("status").unwrap().as_string().unwrap()
    let limit = params.get("limit").unwrap_or(Value::Null).as_int().unwrap_or(10)
    let rows = vault::query("from task where status == \"" + status + "\" && !done")?
    let titles = []
    for row in rows {
        if titles.len() >= limit { break }
        titles.push(row.get("title").unwrap())
    }
    Ok(Value::Map(#{
        "count": Value::Int(rows.len()),
        "titles": Value::List(titles),
    }))
}
```

Try the function on its own with `run_script`, passing the params map as the one argument
(`args` `[{"status": "Todo", "limit": null}]`, `function` `"run"`), before proposing the tool.
This version runs as it is:

```wscript run
use vault

fn run(params: Value) -> Result[Value, string] {
    let status = params.get("status").unwrap().as_string().unwrap()
    let rows = vault::query("from task where status == \"" + status + "\"")?
    Ok(Value::Int(rows.len()))
}

fn main() -> Result[Value, string] {
    run(Value::Map(#{ "status": Value::String("Todo"), "limit": Value::Null }))
}
```

A tool that calls a web API names the host in `allow_domains` and the key in `secrets`:

```wscript
use http
use json
use secrets

fn lookup(params: Value) -> Result[Value, string] {
    let word = params.get("word").unwrap().as_string().unwrap()
    let key = secrets::get("example_api_key")?
    let r = http::get("https://api.example.com/define?word=" + word, #{ "authorization": "Bearer " + key })?
    if r.status != 200 {
        return Err(fmt("the dictionary answered {}", r.status))
    }
    json::parse(r.body)
}
```

## Checking a tool

`propose_tool_change` checks the tool file and compiles the script, and the user sees the
change before it is written. A tool whose file is wrong, whose script is missing, or whose
script does not compile is not offered to any agent until it is fixed; `list_tools` shows why.
