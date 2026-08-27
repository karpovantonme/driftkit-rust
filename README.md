# driftkit

**Finds where a project's promises and its code disagree.**

A promise is anything a reader can check: an example env file, a tool schema,
a docstring, a support matrix. Nobody runs a promise, so it rots quietly while
the code moves on. `driftkit` reads both sides and reports only where they
actually disagree.

```console
$ driftkit env --dir ./metron

--- Required by the code, missing from the example (2) ---
  STATIC_ROOT  metron/settings.py:407
     a fresh clone raises KeyError here
  MEDIA_ROOT  metron/settings.py:410
     a fresh clone raises KeyError here

=== Coverage ===
  example files:          1 (.env.example)
  variables declared:     31 (+0 commented out)
  python files read:      496
  python reads:           29 (22 required)
  reads in other langs:   11 (collected, never judged)
  declared and read:      29 of 31
  susceptibility:         susceptible
  findings:               2 hard, 0 soft
```

That is a real project, and the two lines are real. Its `.env.example` opens
with `# Copy this file to .env and fill in all values.` Do exactly that, and:

```console
$ cp .env.example .env
$ python -c "from decouple import config; config('STATIC_ROOT')"
UndefinedValueError: STATIC_ROOT not found. Declare it as envvar or define a default value.
```

The example sets `DEBUG=True`, `settings.py` branches on `if not DEBUG`, and
the development branch reads two variables the example never mentions. Every
newcomer meets this in their first ten minutes.

## Install

```console
cargo install --git https://github.com/karpovantonme/driftkit
```

Or build it: `cargo build --release`, and the binary lands in
`target/release/driftkit`.

## Usage

```console
driftkit env --dir PATH          # the check
driftkit env --dir PATH --plain  # also list declared-but-unread names
driftkit env --dir PATH --json out.json
driftkit env --dir PATH -v       # what was read, what failed to parse

driftkit mcp --dir SERVER        # read one MCP server
driftkit mcp --dir A B C         # triage a pool: which are worth reading
```

Exit code is `1` if and only if there is at least one hard finding, so it fits
straight into a shell `if` or a CI step.

## `mcp`: a tool schema against what its handler reads

An MCP server declares each tool's input as a JSON Schema, and the agent obeys
it literally. When the handler reads a different key, the agent has no way to
know: either a capability is silently unreachable, or the call fails on input
that matched the published schema.

```console
$ driftkit mcp --dir ./KiCAD-MCP-Server
  set_active_layer   the schema requires layerName while the handler reads layer
```

That server had 15 tools that could not work at all. Go is the most
susceptible substrate there is, because the property name is a string literal
on both sides and the compiler has no opinion about whether the two match.

🔴 **Susceptibility is decided before scanning.** Where one source feeds both
the schema and the handler -- FastMCP, `zodToJsonSchema`, `z.infer` -- a
mismatch is inexpressible, and the server is skipped rather than scanned and
filtered. On a pool of 265 servers that removed 60% of the work.

```console
$ driftkit mcp --dir servers/*
!! SUSCEPTIBLE  KiCAD-MCP-Server      the property name is a literal on both sides
!! SUSCEPTIBLE  opencaselaw           the property name is a literal on both sides
?  unclear      warp-sql-server-mcp   no protective signal and no risk signal, read it by hand
```

Reads Go and Python. On the KiCAD server that is 123 schemas out of 123 bound
to their handlers.

The TypeScript scout is deliberately held back until it can be done on a parse
tree rather than regular expressions, which bound only 25% of its tools.
Shipping it as it stands would mean shipping a scanner that misses three
quarters of what it looks at and says nothing about it.

## What `env` reports, and what each class is worth

Not all findings are equal evidence, and a tool that pretends otherwise
teaches you to ignore it.

| class | the claim | weight |
|---|---|---|
| **B** | the code reads it with no default, no example file declares it | **hard.** A positive claim, checked on the spot: `os.environ["X"]` raises, so a fresh clone dies at that line |
| **A-near** | declared, unread, and a name one typo away *is* read | **hard.** Name against name |
| **A-plain** | declared, unread, nothing similar anywhere | **soft**, and printed only under `--plain` |

A-plain is soft by construction, not by caution: it is a negative claim, and
the sources for it can never be complete. Env vars are also consumed by
compose files, CI, Makefiles and shell, and a project may wrap the whole
environment in a helper of its own.

## Deliberately blind

An honest list beats a silent gap. A scanner earns its place by knowing where
to keep quiet, and every line below is a place this one does.

**The required/optional verdict is Python only.** It comes from the difference
between a crash and a shrug, and that is a property of the language:

```text
os.environ["X"]      raises KeyError     -> required
os.getenv("X")       returns None        -> optional
os.Getenv("X")  (Go) returns ""          -> never required
process.env.X   (JS) undefined           -> never required
```

Reads in Go, JavaScript, Rust, Java, PHP, Ruby, shell, compose files and CI
are collected, so that "nothing reads this name" is never claimed off a
partial view, and are never turned into a hard finding.

**Names that are derived rather than written** -- pydantic `env_prefix`, viper
`AutomaticEnv()`, `envconfig:"..."` tags, Spring `@Value`, or any read whose
key is a variable. The report says `PROTECTED` and refuses to make the
negative claim. It still makes the positive one: `os.environ["X"]` on line 40
raises whatever the rest of the project does.

**Python 3.14 syntax the parser does not know yet.** PEP 758 allows
`except A, B:` without parentheses; `rustpython-parser` rejects it. On a live
Django project that was 6 files out of 496. Such files are counted in the
coverage block and listed under `-v` rather than passed over in silence.

## Things it will not tell you about

These are decisions, not oversights, and each cost a false finding before it
became a rule:

- variables the machine provides (`PATH`, `HOME`, `CI`) or a launcher exports (`LOCAL_RANK`, `MODELSCOPE_CACHE`);
- reads the code already handles -- inside `try/except`, behind `if "X" in os.environ`, or where the code sets the value itself with `os.environ.setdefault`;
- reads in tests, `conftest.py`, examples, benchmarks and end-to-end suites: they do not speak for what a newcomer runs;
- names declared in a committed `.env.test` -- the claim is "nowhere to learn this name", and a committed env file is such a place;
- commented-out lines in an example file: a hint, not a promise.

## The coverage block is part of the output

Every run prints what was compared, not only what was found. A scan that
matched 3 of 40 must not look as healthy as one that matched 40 of 40, and
without a binding counter it does.

## The contract

Every check obeys the same rules. They exist because the Python kit this grew
out of broke all of them at least once, and one of those breaks silently
counted soft findings as hard for a week.

1. `--json FILE` and `-v/--verbose` on every subcommand.
2. `--json` writes a list of objects, each carrying a boolean `hard`.
3. The report ends with `=== Coverage ===` and a `findings: N hard, M soft` line.
4. Exit code is 1 if and only if there is at least one hard finding.
5. The coverage block prints the binding counter.

## Development

```console
cargo test     # 39 tests, each named after a live case it holds down
cargo clippy
```

Every test in `tests/env_tests.rs` is named after the mistake it prevents,
most of them found on real repositories rather than imagined:
`a_dynamic_read_in_a_test_file_protects_nothing`,
`a_whole_different_tail_is_not_a_near_miss`,
`a_read_inside_a_method_chain_is_seen`.

## Licence

MIT.
