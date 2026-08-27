# driftkit

Finds where a project's declarations and its behaviour disagree.

A declaration is a promise a reader can check: an example env file, a tool
schema, a docstring, a support matrix. Nobody runs the promise, so it rots
quietly while the code moves on. `driftkit` reads both sides and reports only
where they actually disagree.

```
driftkit env --dir ~/some/project
```

## Checks

| check | species |
|---|---|
| `env` | the example env file against what the code actually reads |

## What `env` reports, and how much each class is worth

Three classes, and they are not equal evidence.

**B, required by the code and missing from every example file.** A positive
claim proved locally: `os.environ["SMTP_HOST"]` raises `KeyError`, so a fresh
clone dies at that line. Hard.

**A-near, declared but unread, while a name one typo away IS read.** Name
against name. Hard.

**A-plain, declared, unread, nothing similar.** A negative claim from a source
that cannot be complete, so soft, and printed only under `--plain`.

## Deliberately blind

An honest list beats a silent gap.

- **The required/optional verdict is Python only.** It comes from the
  difference between a crash and a shrug, and that is a property of the
  language: `os.environ["X"]` raises, `os.getenv("X")` returns `None`,
  `os.Getenv` in Go returns `""`, `process.env.X` in JS is `undefined`. Reads
  in Go, JavaScript, Rust, Java, PHP, Ruby, shell, compose files and CI are
  collected so that absence is never claimed off a partial view, and are
  never turned into a hard finding.
- **Python 3.14 syntax that the parser does not know yet.** `rustpython-parser`
  0.4.0 rejects the unparenthesised `except A, B:` from PEP 758. On a live
  Django project that was 6 files out of 496. Such files are counted and
  listed under `--verbose` rather than passed over.
- **Names that are derived rather than written** -- pydantic `env_prefix`,
  viper `AutomaticEnv()`, `envconfig:"..."` tags, Spring `@Value`, or any read
  whose key is a variable. The report says `PROTECTED` and refuses to make the
  negative claim. It still makes the positive one.

## The contract

Every check obeys the same rules, and they exist because the Python kit this
grew out of broke all of them at least once:

1. `--json FILE` and `-v/--verbose` everywhere.
2. `--json` writes a list of objects, each carrying a boolean `hard`.
3. The report ends with `=== Coverage ===` and a `findings: N hard, M soft`
   line.
4. Exit code is 1 if and only if there is at least one hard finding.
5. The coverage block prints a binding counter: how much was declared, how
   much was read, how much was matched. A scan that matched 3 of 40 must not
   look healthy.

## Building

```
cargo build --release
cargo test
```
