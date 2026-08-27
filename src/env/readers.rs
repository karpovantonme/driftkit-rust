//! Everything that is not the Python tree: example files, readers in other
//! languages, and the signals that say a name is derived rather than written.

use regex::Regex;
use std::sync::OnceLock;

/// 🔴 Take the union of ALL example files before saying a name is absent.
/// Repos carry `.env.example` at the root, `.env.test.example` beside it and
/// `docker/.env.example` one level down. Reading one of the three turns every
/// variable in the other two into a finding.
pub fn is_example_file(name: &str) -> bool {
    static RX: OnceLock<Regex> = OnceLock::new();
    RX.get_or_init(|| {
        Regex::new(
            // 🔴 The last alternative is not decoration: projects ship
            // `.env.example.keychain`, `.env.example.1password` and
            // `.env.example.local` beside the plain one. Dropping it cost 16
            // example files out of 96 across 60 projects, and the port only
            // showed it because the Python numbers were the acceptance test.
            r"(?ix)^(
                  \.?env(\.[\w.-]+)?\.(example|sample|template|dist|defaults)
                | (example|sample|template)\.env
                | \.env\.example[\w.-]*
            )$",
        )
        .unwrap()
    })
    .is_match(name)
}

/// 🔴 Live false finding on chanx: `.env.EXAMPLE` holds two OpenAI keys, and
/// `DJANGO_SECRET_KEY` sits in a committed `.env.test` with a working value.
/// The claim being made is "a newcomer has nowhere to learn this name", and a
/// committed env file of any kind is such a place. It counts against side B
/// without counting as an example for side A.
pub fn is_committed_env(name: &str) -> bool {
    static RX: OnceLock<Regex> = OnceLock::new();
    RX.get_or_init(|| Regex::new(r"(?i)^\.?env(\.[\w.-]+)?$").unwrap())
        .is_match(name)
}

pub fn declaration(line: &str) -> Option<String> {
    static RX: OnceLock<Regex> = OnceLock::new();
    RX.get_or_init(|| Regex::new(r"^\s*(?:export\s+)?([A-Za-z_][A-Za-z0-9_]*)\s*=").unwrap())
        .captures(line)
        .map(|c| c[1].to_string())
}

/// A commented-out declaration is a hint, not a promise. Counted separately
/// so it neither becomes a finding nor makes a required variable look
/// declared.
pub fn commented_declaration(line: &str) -> Option<String> {
    static RX: OnceLock<Regex> = OnceLock::new();
    RX.get_or_init(|| Regex::new(r"^\s*#\s*(?:export\s+)?([A-Za-z_][A-Za-z0-9_]*)\s*=").unwrap())
        .captures(line)
        .map(|c| c[1].to_string())
}

/// Reads in languages this tool collects but never judges.
///
/// The required/optional verdict comes from crash-against-silence, and that
/// is a property of the language: `os.Getenv` in Go returns an empty string,
/// `process.env.FOO` in JS is undefined. Neither can crash, so neither can
/// carry side B. They exist here so that "nothing reads this name" is not
/// said off a partial view.
pub fn other_reads(ext: &str, name: &str, src: &str) -> Vec<String> {
    let mut out = Vec::new();
    let lower = name.to_lowercase();

    let rx: &Regex = match ext {
        "js" | "jsx" | "ts" | "tsx" | "mjs" | "cjs" | "svelte" | "vue" | "astro" => {
            static R: OnceLock<Regex> = OnceLock::new();
            R.get_or_init(|| {
                Regex::new(
                    r#"(?:process|import\.meta)\.env\.([A-Z_][A-Z0-9_]*)|(?:process|import\.meta)\.env\[\s*["'`]([A-Za-z_][A-Za-z0-9_]*)["'`]\s*\]"#,
                )
                .unwrap()
            })
        }
        "go" => {
            static R: OnceLock<Regex> = OnceLock::new();
            R.get_or_init(|| {
                Regex::new(r#"os\.(?:Getenv|LookupEnv)\(\s*["`]([A-Za-z_][A-Za-z0-9_]*)["`]"#)
                    .unwrap()
            })
        }
        "rs" => {
            static R: OnceLock<Regex> = OnceLock::new();
            R.get_or_init(|| {
                Regex::new(r#"env::(?:var|var_os)\(\s*"([A-Za-z_][A-Za-z0-9_]*)"|env!\(\s*"([A-Za-z_][A-Za-z0-9_]*)""#).unwrap()
            })
        }
        "java" | "kt" | "kts" | "scala" => {
            static R: OnceLock<Regex> = OnceLock::new();
            R.get_or_init(|| {
                Regex::new(r#"System\.getenv\(\s*"([A-Za-z_][A-Za-z0-9_]*)""#).unwrap()
            })
        }
        "php" => {
            static R: OnceLock<Regex> = OnceLock::new();
            R.get_or_init(|| {
                Regex::new(r#"(?:getenv|env)\(\s*['"]([A-Za-z_][A-Za-z0-9_]*)['"]|\$_(?:ENV|SERVER)\[\s*['"]([A-Za-z_][A-Za-z0-9_]*)['"]\s*\]"#).unwrap()
            })
        }
        "rb" | "rake" => {
            static R: OnceLock<Regex> = OnceLock::new();
            R.get_or_init(|| {
                Regex::new(r#"ENV(?:\.fetch)?\s*[\[(]\s*['"]([A-Za-z_][A-Za-z0-9_]*)['"]"#).unwrap()
            })
        }
        "sh" | "bash" | "zsh" | "mk" | "yml" | "yaml" | "tf" => {
            static R: OnceLock<Regex> = OnceLock::new();
            R.get_or_init(|| {
                Regex::new(r"\$\{?([A-Z_][A-Z0-9_]*)\}?|\$\{\{\s*(?:secrets|vars|env)\.([A-Za-z_][A-Za-z0-9_]*)\s*\}\}").unwrap()
            })
        }
        _ if lower.starts_with("dockerfile")
            || lower.ends_with(".dockerfile")
            || lower == "makefile"
            || lower == "justfile"
            || lower == "procfile" =>
        {
            static R: OnceLock<Regex> = OnceLock::new();
            R.get_or_init(|| {
                Regex::new(
                    r"(?m)^\s*(?:ENV|ARG)\s+([A-Za-z_][A-Za-z0-9_]*)|\$\{?([A-Z_][A-Z0-9_]*)\}?",
                )
                .unwrap()
            })
        }
        _ => return out,
    };

    for caps in rx.captures_iter(src) {
        for i in 1..caps.len() {
            if let Some(m) = caps.get(i) {
                out.push(m.as_str().to_string());
            }
        }
    }
    out
}

/// 🔴 Every project may wrap the environment in its own helper, and the
/// method names cannot be guessed. MangAdventure reads `env.secret('SECRET_KEY')`
/// through a wrapper of its own, and the reader list put SECRET_KEY among the
/// dead variables of a Django project.
///
/// So the negative claim is not made off the reader list at all: a name that
/// appears anywhere in the sources as a bare token counts as read. Coarse on
/// purpose -- missing a finding is cheap here, claiming a false one is not.
pub fn shouty_tokens(src: &str) -> Vec<String> {
    static RX: OnceLock<Regex> = OnceLock::new();
    RX.get_or_init(|| Regex::new(r"\b[A-Z][A-Z0-9_]{2,}\b").unwrap())
        .find_iter(src)
        .map(|m| m.as_str().to_string())
        .collect()
}

/// Signals that the variable name is derived rather than written, each tied
/// to the languages where it can be true.
///
/// 🔴 The first version matched them in any file, and the tool declared
/// ITSELF protected: its own source quotes `AutomaticEnv` inside these very
/// patterns. One false protection turns every hard finding soft.
pub fn protection(ext: &str, src: &str) -> Option<&'static str> {
    macro_rules! rx {
        ($pat:expr) => {{
            static R: OnceLock<Regex> = OnceLock::new();
            R.get_or_init(|| Regex::new($pat).unwrap())
        }};
    }

    match ext {
        "py" | "pyi" => {
            if rx!(r"env_prefix\s*=|SettingsConfigDict\s*\(|class\s+\w+\s*\(\s*BaseSettings\s*\)")
                .is_match(src)
            {
                return Some(
                    "pydantic settings with env_prefix: names are derived from field names",
                );
            }
            if rx!(r"from\s+dynaconf|Dynaconf\s*\(").is_match(src) {
                return Some("dynaconf: the loader walks a declared schema");
            }
        }
        "go" => {
            if rx!(r"AutomaticEnv\s*\(\s*\)|SetEnvPrefix\s*\(").is_match(src) {
                return Some("viper AutomaticEnv: every key is bound to an env var implicitly");
            }
            if rx!(r#"envconfig:""#).is_match(src) {
                return Some("envconfig struct tags: names come from the tag, not from a call");
            }
        }
        "java" | "kt" | "kts" | "scala"
            if rx!(r#"@Value\s*\(\s*["']\$\{|@ConfigurationProperties"#).is_match(src) =>
        {
            return Some("Spring @Value: names live in property files");
        }
        _ => {}
    }
    None
}

/// A read whose key is a variable blinds the negative claim just as much.
pub fn dynamic_read(ext: &str, src: &str) -> bool {
    if !matches!(
        ext,
        "py" | "pyi" | "js" | "jsx" | "ts" | "tsx" | "mjs" | "cjs"
    ) {
        return false;
    }
    static RX: OnceLock<Regex> = OnceLock::new();
    RX.get_or_init(|| {
        Regex::new(
            r"os\.environ(?:\.get)?\(\s*[a-z_]\w*\s*[,)]|os\.environ\[\s*[a-z_]\w*\s*\]|os\.getenv\(\s*[a-z_]\w*\s*[,)]|process\.env\[\s*[a-z_$]\w*\s*\]|for\s+\w+\s*,?\s*\w*\s+in\s+os\.environ",
        )
        .unwrap()
    })
    .is_match(src)
}
