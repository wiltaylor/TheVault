# The wscript language

wscript is statically typed with local inference. Every example here compiles; those that run
on their own have a `main` function.

## A unit

A script is one file of top-level items: `use` lines, functions, structs, enums, traits and
`impl` blocks. There are no top-level statements. The host calls one of its functions by name.

```wscript run
use vault

fn label(n: int) -> string {
    if n == 1 { "1 note" } else { fmt("{} notes", n) }
}

fn main() -> Result[string, string] {
    let rows = vault::query("from task")?
    vault::log("counted " + str(rows.len()))
    Ok(label(rows.len()))
}
```

## Comments, statements and lines

- `// comment` runs to the end of the line.
- A statement ends at a newline; `;` is allowed. A statement goes on to the next line inside an
  open `(` or `[`, after a token that cannot end an expression (`+`, `,`, `=`), when the next
  line starts with `.`, or when the next token is `else`.
- A trailing `;` throws a block's last value away. An `if` without `else` must end in unit, so
  end a last call that returns a value with `;`, or bind it:
  `if ok { let saved = vault::patch(id, fields, "")? }`.

## Values and types

| Type | Examples | Notes |
|------|----------|-------|
| `int` | `42`, `-7`, `0xFF`, `1_000_000` | 64-bit signed, wrapping |
| `float` | `3.14`, `1e9` | 64-bit IEEE |
| `bool` | `true`, `false` | Conditions must be `bool` |
| `char` | `'a'`, `'\n'`, `'\u{1F600}'` | One Unicode scalar |
| `unit` | `()` | What a function with no return type returns |
| `string` | `"text"` | Immutable UTF-8, a reference type |
| `List[T]` | `[1, 2, 3]` | Growable list, a reference type |
| `Map[K, V]` | `#{ "a": 1 }` | Keys are `int`, `bool`, `char` or `string` |
| `Option[T]` | `Some(x)`, `None` | Always available |
| `Result[T, E]` | `Ok(x)`, `Err(e)` | Always available |
| `fn(A, B) -> R` | `\|x\| x * 2` | Function values and closures |
| `Value` | `Value::Int(1)` | The dynamic JSON-like value notes travel as |

There is no implicit conversion and no truthiness. Convert with `int(x)` (a float truncates, a
char gives its code point) and `float(x)`.

```wscript run
fn main() -> float {
    let total = 1 + int(2.9)        // 3
    let ratio = float(total) / 2.0  // 1.5
    let big = 1_000_000
    let mask = 0xFF
    ratio + float(big - big) + float(mask - 255)
}
```

`let` binds a name; any binding can be assigned again (`x = x + 1`). Annotations on `let` are
allowed (`let name: string = "a"`) and never needed. Primitives are copied; strings, lists,
maps, structs and enums are references: two names for one list see each other's changes. Copy
one explicitly with `.clone()`.

## Operators

- Arithmetic `+ - * / %` on two numbers of the same type; `+` also joins two strings.
- Comparison `== != < <= > >=`, logic `&& || !`.
- Compound assignment `+= -= *= /= %=` on variables, fields, list items and map entries.
- There are no bitwise operators.

## Functions and closures

Every parameter and the return type are annotated. A missing return type is unit. `return`
leaves early.

```wscript run
fn apply(f: fn(int) -> int, x: int) -> int { f(x) }

fn larger[T: Ord](a: T, b: T) -> T { if a > b { a } else { b } }

fn main() -> int {
    let double = |x| x * 2
    let offset = 10
    let shift = |x| x + offset      // closures capture by reference
    apply(double, 21) + apply(shift, 0) + larger(3, 4)
}
```

Generic functions take type parameters with the bounds `Eq`, `Ord` and `Clone`. There are no
generic structs or enums.

## Control flow

`if` and `match` are expressions. Loops are `while`, `loop` (left with `break`), and `for`
over a range, a list, a map's keys or a string's chars. `continue` skips to the next turn. A
range such as `0..10` (exclusive) or `0..=10` (inclusive) is only valid in a `for` header.

```wscript run
fn main() -> int {
    let total = 0
    for i in 0..10 {
        if i % 2 == 0 { continue }
        total += i
    }
    for x in [1, 2, 3] { total += x }
    let ages = #{ "alice": 30, "bob": 25 }
    for name in ages { total += ages[name] }
    let n = 0
    while n < 3 { n += 1 }
    loop {
        if n >= 5 { break }
        n += 1
    }
    let sign = if total > 0 { 1 } else if total < 0 { -1 } else { 0 }
    total + n + sign
}
```

## Option, Result and `?`

`?` returns early with the `None` or `Err`. Host functions that fail return `Err(message)`.

| Type | Methods |
|------|---------|
| `Option[T]` | `is_some` `is_none` `unwrap` `unwrap_or` `expect` |
| `Result[T, E]` | `is_ok` `is_err` `unwrap` `unwrap_or` `unwrap_err` `expect` |

`unwrap()` on `None` or `Err` faults and ends the run, so prefer `?`, `unwrap_or` or `match`.

```wscript run
fn parse_double(s: string) -> Result[int, string] {
    let Some(n) = s.parse_int() else { return Err("not a number: " + s) }
    Ok(n * 2)
}

fn sum(a: string, b: string) -> Result[int, string] {
    Ok(parse_double(a)? + parse_double(b)?)
}

fn main() -> Result[int, string] {
    let bad = match sum("1", "x") {
        Ok(_) => 0,
        Err(e) => e.len(),
    }
    let fallback = "nope".parse_int().unwrap_or(7)
    if let Some(n) = "5".parse_int() {
        return Ok(sum("2", "3")? + bad + fallback + n)
    }
    Err("unreachable")
}
```

`let PATTERN = value else { ... }` binds or runs a block that must leave (`return`, `break`).

## match

`match` is checked for exhaustiveness when the script compiles. Patterns: `_`, a name, a literal,
`Enum::Variant(x)`, `Enum::Variant { x, y }`, and `a | b` (without bindings). An arm may have a
guard, `pattern if condition => ...`; guarded arms do not count toward exhaustiveness.

```wscript run
enum Shape {
    Dot,
    Circle(float),
    Rect { w: float, h: float },
}

fn area(s: Shape) -> float {
    match s {
        Shape::Dot => 0.0,
        Shape::Circle(r) if r < 0.0 => 0.0,
        Shape::Circle(r) => 3.14159 * r * r,
        Shape::Rect { w, h } => w * h,
    }
}

fn grade(n: int) -> string {
    match n {
        0 => "none",
        1 | 2 => "few",
        _ => "many",
    }
}

fn main() -> string {
    let total = area(Shape::Rect { w: 2.0, h: 3.0 }) + area(Shape::Dot) + area(Shape::Circle(1.0))
    grade(int(total))
}
```

## Strings

Strings are immutable; every method returns a new string. `len`, `slice` and `find` count
characters; `bytes_len` counts bytes.

| Group | Methods |
|-------|---------|
| Size | `len` `bytes_len` `is_empty` |
| Split | `split` |
| Trim | `trim` `trim_start` `trim_end` |
| Case | `to_upper` `to_lower` |
| Search | `starts_with` `ends_with` `contains` `find` |
| Reshape | `replace` `repeat` `pad_left` `pad_right` |
| Slice | `chars` `slice` |
| Parse | `parse_int` `parse_float` (return `Option`) |

Every string literal interpolates: `"{expr}"` puts in any expression, shown as `str()` shows
it. Inside a hole, write strings with plain quotes. A literal brace is `{{` or `}}`. `{}` and
`{:spec}` stay literal text, for `fmt`.

`fmt(template, args...)` fills `{}` placeholders, with optional specs: `{:>8}` right-aligns in
8 characters, `{:<8}` left-aligns, `{:^8}` centres, `{:.2}` gives two decimals, `{:04}` pads with
zeros, `{:x}` is hex. `str(x)` turns any value into a string.

```wscript run
fn main() -> string {
    let name = "  Ada Lovelace "
    let clean = name.trim()
    let parts = clean.split(" ")
    let first = parts[0].to_upper()
    let n = 3
    let line = "{first} has {n + 1} items"
    let padded = fmt("{:>6}|{:.2}|{:x}", "ok", 3.14159, 255)
    let braces = "{{literal}}"
    line + " " + padded + " " + braces + " " + clean.slice(0, 3) + str(clean.contains("Love"))
}
```

## Lists

| Group | Methods |
|-------|---------|
| Size | `len` `is_empty` `clear` |
| Add and remove | `push` `pop` `insert` `remove` |
| Access | `get` (an `Option`) `set` `first` `last`, `xs[i]` (faults when out of range) |
| Search | `contains` `index_of` |
| Tests | `any(\|x\| ...)` `all(\|x\| ...)` `count(\|x\| ...)` |
| Find | `find(\|x\| ...) -> Option[T]` `position(\|x\| ...) -> Option[int]` |
| Order | `reverse` `sort` `sort_by(\|a, b\| ...)` (the comparator returns a negative, zero or positive `int`) |
| Transform | `map` `filter` `fold` `join` `slice` `concat` |
| Pairs | `map_indexed(\|i, x\| ...)` `zip_with(other, \|a, b\| ...)` |
| Totals | `sum` `min` `max` (`min` and `max` return `Option`) |
| Copy | `clone` |

```wscript run
fn main() -> int {
    let xs = [5, 3, 8]
    xs.push(1)
    xs.sort()
    let evens = xs.filter(|x| x % 2 == 0)
    let doubled = xs.map(|x| x * 2)
    let total = doubled.fold(0, |a, x| a + x)
    let first_big = xs.find(|x| x > 4).unwrap_or(0)
    let none = xs.get(99).is_none()
    let words = ["a", "b"].join(",")
    if none && words.len() == 3 { total + evens.len() + first_big + xs.max().unwrap_or(0) } else { 0 }
}
```

## Maps

A map literal is `#{ key: value, ... }`. `m[k] = v` inserts or replaces.

| Group | Methods |
|-------|---------|
| Size | `len` `is_empty` `clear` |
| Add and remove | `insert` `remove` |
| Access | `get` (an `Option`) `contains_key`, `m[k]` (faults when missing) |
| Walk | `keys` `values` `each(\|k, v\| ...)` |
| Transform | `map(\|k, v\| ...) -> List[U]` `filter(\|k, v\| ...) -> Map[K, V]` |
| Copy | `clone` |

```wscript run
fn main() -> int {
    let counts = #{ "open": 2 }
    counts["done"] = 5
    counts["open"] += 1
    let missing = counts.get("late").unwrap_or(0)
    let keys = counts.keys()
    let big = counts.filter(|k, v| v > 3)
    counts["open"] + counts["done"] + missing + keys.len() + big.len()
}
```

## Structs, enums, impl and traits

A struct is a record of typed fields. Methods go in an `impl` block; `self` is always by
reference and needs no type. A function in `impl` without `self` is called as `Type::f(...)`.
`#[derive(Eq, Ord, Display, Clone)]` writes those traits for you; `==` on a struct or enum
needs `Eq`. Traits declare methods that `impl Trait for Type` provides; `dyn Trait` takes any
type that implements it.

```wscript run
#[derive(Eq, Clone)]
struct Counter { name: string, count: int }

impl Counter {
    fn new(name: string) -> Counter { Counter { name: name, count: 0 } }
    fn bump(self, by: int) { self.count += by }
}

trait Describe {
    fn describe(self) -> string
}

impl Describe for Counter {
    fn describe(self) -> string { "{self.name}={self.count}" }
}

fn show(d: dyn Describe) -> string { d.describe() }

fn main() -> string {
    let c = Counter::new("notes")
    let alias = c
    alias.bump(3)                  // c.count is 3 too: one struct, two names
    let copy = c.clone()
    copy.bump(1)
    show(c) + " " + show(copy) + " " + str(same(c, alias))
}
```

## The prelude

Always there, with no `use`: `print(x)`, `println(x)` (both write to the run log), `str(x)`,
`fmt(template, ...)`, `same(a, b)` (the same object), `weak(x)`, `int(x)` and `float(x)`.

## Faults

Some mistakes stop the run with a fault rather than an `Err`: `xs[i]` out of range, `m[k]`
missing, `unwrap()` on `None` or `Err`, division by zero, running out of instructions, and calls
nested too deep. A script cannot catch a fault. Use `.get`, `?`, `unwrap_or` and `match` where a
value may be missing.

## Standard modules

`use` them before calling.

### `value`: the `Value` type

`Value` is an enum: `Value::Null`, `Value::Bool(bool)`, `Value::Int(int)`, `Value::Float(float)`,
`Value::String(string)`, `Value::List(List[Value])` and `Value::Map(Map[string, Value])`. Notes,
query rows and JSON all arrive as `Value`. It needs no `use`.

| Method | Returns |
|--------|---------|
| `get(key: string)` | `Option[Value]`: a map's entry |
| `at(idx: int)` | `Option[Value]`: a list's item |
| `keys()` | `List[string]` |
| `len()` | `int` |
| `is_null()` | `bool` |
| `as_bool` `as_int` `as_float` `as_string` `as_list` `as_map` | `Option[...]`; `as_float` takes an `Int` too |

A `Value` has no setters: build a new one with its constructors. `match` on it like any enum.

```wscript run
fn title_of(row: Value) -> string {
    match row.get("title") {
        Some(Value::String(t)) => t,
        _ => "(untitled)",
    }
}

fn main() -> string {
    let row = Value::Map(#{
        "title": Value::String("Plan"),
        "tags": Value::List([Value::String("a"), Value::String("b")]),
        "estimate": Value::Int(3),
    })
    let tags = row.get("tags").unwrap().as_list().unwrap()
    let est = row.get("estimate").unwrap().as_int().unwrap_or(0)
    title_of(row) + str(tags.len()) + str(est)
}
```

### `json`

| Function | Returns |
|----------|---------|
| `json::parse(text: string)` | `Result[Value, string]` |
| `json::to_string(v: Value)` | `string`, keys sorted |
| `json::to_string_pretty(v: Value)` | `string` |

```wscript run
use json

fn main() -> Result[string, string] {
    let v = json::parse("[1, 2, 3]")?
    let n = v.len()
    Ok(json::to_string(Value::Map(#{ "count": Value::Int(n), "items": v })))
}
```

### `math`

Floats: `abs min max clamp floor ceil round trunc sqrt pow exp ln log2 log10 signum lerp`,
trigonometry `sin cos tan asin acos atan atan2 sinh cosh tanh asinh acosh atanh`. Ints: `iabs
imin imax iclamp isignum`. Constants `PI E TAU INF NAN`. Random: `rand() -> float` in [0, 1),
`rand_range(a: int, b: int) -> int` in [a, b).

```wscript run
use math

fn main() -> int {
    let r = math::sqrt(16.0) + math::floor(2.7)
    math::imax(int(r), 3) + math::iabs(-2)
}
```

### `regex`

The pattern comes first, the text second. An invalid pattern faults.

| Function | Returns |
|----------|---------|
| `regex::is_match(pattern, text)` | `bool` |
| `regex::find(pattern, text)` | `Option[string]`: the first match |
| `regex::find_all(pattern, text)` | `List[string]` |
| `regex::replace(pattern, text, with)` | `string`: every match replaced; `$1` and `$name` name groups |
| `regex::captures(pattern, text)` | `Option[List[string]]`: group 0 (the whole match) first |
| `regex::split(pattern, text)` | `List[string]` |

A regex quantifier's braces must be doubled in a string literal: `"[0-9]{{4}}"`.

```wscript run
use regex

fn main() -> string {
    let words = regex::find_all("[a-z]+", "one, two, three")
    let year = regex::find("[0-9]{{4}}", "due 2026-10-26").unwrap_or("")
    let masked = regex::replace("([0-9]{{4}})[0-9]+", "4111111111", "$1-")
    str(words.len()) + year + masked
}
```

wscript's own `fs`, `process` and `time` modules are not available in the vault. The vault's
`fs` module is described in `vault-api.md`.
