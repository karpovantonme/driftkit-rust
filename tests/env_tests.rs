//! Every test here names the mistake it holds down.
//!
//! Ported from the Python kit, where all of them came from live runs rather
//! than from theory. A rule with no test lives until the next edit.

use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};

/// 🔴 Cargo runs tests in parallel threads. The first version named the
/// throwaway directory after a hash of the file sizes, two tests collided,
/// and one wiped the other's tree mid-run. A counter, not a hash.
static NEXT: AtomicUsize = AtomicUsize::new(0);

fn build(files: &[(&str, &str)]) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "driftkit-test-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = fs::remove_dir_all(&dir);
    for (rel, body) in files {
        let path = dir.join(rel);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, body).unwrap();
    }
    dir
}

fn run(files: &[(&str, &str)], plain: bool) -> driftkit::env::Report {
    let dir = build(files);
    let mut report = driftkit::env::Report::default();
    driftkit::env::analyse(&dir, &mut report, plain);
    let _ = fs::remove_dir_all(&dir);
    report
}

fn names(r: &driftkit::env::Report, kind: &str) -> Vec<String> {
    let mut v: Vec<String> = r
        .findings
        .iter()
        .filter(|f| f.kind == kind)
        .map(|f| f.name.clone())
        .collect();
    v.sort();
    v
}

// ---------------------------------------------------------------------------
// Class B: required by the code, absent from every example file
// ---------------------------------------------------------------------------

#[test]
fn subscript_read_with_no_example_entry_is_hard() {
    let r = run(
        &[
            (".env.example", "DATABASE_URL=postgres://localhost/app\n"),
            (
                "app/settings.py",
                "import os\nSMTP_HOST = os.environ[\"SMTP_HOST\"]\n",
            ),
        ],
        false,
    );
    assert_eq!(names(&r, "missing-from-example"), vec!["SMTP_HOST"]);
    assert!(r.findings.iter().all(|f| f.hard));
}

/// `os.getenv` returns None quietly, so a fresh clone does not die.
#[test]
fn getenv_is_not_a_finding() {
    let r = run(
        &[
            (".env.example", "DATABASE_URL=x\n"),
            (
                "app/settings.py",
                "import os\nSMTP = os.getenv(\"SMTP_HOST\")\n",
            ),
        ],
        false,
    );
    assert!(names(&r, "missing-from-example").is_empty());
}

#[test]
fn environ_get_is_not_a_finding() {
    let r = run(
        &[
            (".env.example", "DATABASE_URL=x\n"),
            ("app/s.py", "import os\nS = os.environ.get(\"SMTP_HOST\")\n"),
        ],
        false,
    );
    assert!(names(&r, "missing-from-example").is_empty());
}

#[test]
fn django_environ_without_default_is_required() {
    let r = run(
        &[
            (".env.example", "DEBUG=1\n"),
            (
                "app/s.py",
                "SECRET = env.str(\"SECRET_KEY\")\nDEBUG = env.bool(\"DEBUG\", default=False)\n",
            ),
        ],
        false,
    );
    assert_eq!(names(&r, "missing-from-example"), vec!["SECRET_KEY"]);
}

#[test]
fn positional_default_counts_as_a_default() {
    let r = run(
        &[
            (".env.example", "DEBUG=1\n"),
            ("app/s.py", "SECRET = env(\"SECRET_KEY\", \"dev\")\n"),
        ],
        false,
    );
    assert!(names(&r, "missing-from-example").is_empty());
}

/// 🔴 Live gap found by porting: the callee of a call is part of the tree.
/// `os.getenv("X", "").strip()` keeps the inner call inside `func`, and
/// skipping it cost 49 distinct names against 11 on one live project.
#[test]
fn a_read_inside_a_method_chain_is_seen() {
    let r = run(
        &[
            (".env.example", "DEBUG=1\n"),
            (
                "app/s.py",
                "import os\nbackend = os.getenv(\"BUS_BACKEND\", \"\").strip().lower()\nurl = (os.environ[\"BUS_URL\"]).rstrip(\"/\")\n",
            ),
        ],
        false,
    );
    assert_eq!(names(&r, "missing-from-example"), vec!["BUS_URL"]);
    assert!(r.py_reads.iter().any(|x| x.name == "BUS_BACKEND"));
}

/// Species H: nobody puts PATH in `.env.example`, on purpose.
#[test]
fn ambient_names_are_not_expected_in_an_example_file() {
    let r = run(
        &[
            (".env.example", "DEBUG=1\n"),
            (
                "app/s.py",
                "import os\nP = os.environ[\"PATH\"]\nH = os.environ[\"HOME\"]\n",
            ),
        ],
        false,
    );
    assert!(names(&r, "missing-from-example").is_empty());
}

/// 🔴 torchrun exports LOCAL_RANK; nobody writes it into an example file.
#[test]
fn launcher_variables_are_not_expected_either() {
    let r = run(
        &[
            (".env.example", "DEBUG=1\n"),
            (
                "train.py",
                "import os\nR = os.environ[\"LOCAL_RANK\"]\nC = os.environ[\"MODELSCOPE_CACHE\"]\n",
            ),
        ],
        false,
    );
    assert!(names(&r, "missing-from-example").is_empty());
}

#[test]
fn reads_in_tests_do_not_speak_for_a_newcomer() {
    let r = run(
        &[
            (".env.example", "DEBUG=1\n"),
            (
                "tests/test_api.py",
                "import os\nK = os.environ[\"CI_ONLY_TOKEN\"]\n",
            ),
            (
                "app/conftest.py",
                "import os\nX = os.environ[\"ANOTHER_TEST_ONE\"]\n",
            ),
        ],
        false,
    );
    assert!(names(&r, "missing-from-example").is_empty());
}

/// 🔴 Live false finding: `e2e_suite.py` at the root reads ADMIN_PW.
#[test]
fn an_e2e_suite_does_not_speak_for_the_application() {
    let r = run(
        &[
            (".env.example", "DEBUG=1\n"),
            ("e2e_suite.py", "import os\nPW = os.environ[\"ADMIN_PW\"]\n"),
            (
                "smoke_check.py",
                "import os\nX = os.environ[\"OTHER_ONE\"]\n",
            ),
        ],
        false,
    );
    assert!(names(&r, "missing-from-example").is_empty());
}

/// 🔴 Reading one example file turns every variable in the others into a finding.
#[test]
fn all_example_files_are_taken_together() {
    let r = run(
        &[
            (".env.example", "DEBUG=1\n"),
            ("docker/.env.sample", "SMTP_HOST=mail\n"),
            (".env.test.example", "TEST_DB=x\n"),
            (
                "app/s.py",
                "import os\nA = os.environ[\"SMTP_HOST\"]\nB = os.environ[\"TEST_DB\"]\n",
            ),
        ],
        false,
    );
    assert_eq!(r.example_files.len(), 3);
    assert!(names(&r, "missing-from-example").is_empty());
}

/// 🔴 Live gap found by porting: `.env.example.keychain` and
/// `.env.example.1password` are example files too, and dropping them cost 16
/// files out of 96 across the survey.
#[test]
fn suffixed_example_files_count_as_examples() {
    let r = run(
        &[
            (".env.example", "DEBUG=1\n"),
            ("config/.env.example.keychain", "SMTP_HOST=mail\n"),
            ("config/.env.example.1password", "API_TOKEN=x\n"),
            (
                "app/s.py",
                "import os\nA = os.environ[\"SMTP_HOST\"]\nB = os.environ[\"API_TOKEN\"]\n",
            ),
        ],
        false,
    );
    assert_eq!(r.example_files.len(), 3);
    assert!(names(&r, "missing-from-example").is_empty());
}

/// A commented line is a hint. It is not a defect either way.
#[test]
fn commented_declaration_silences_the_finding() {
    let r = run(
        &[
            (".env.example", "DEBUG=1\n# SMTP_HOST=mail.example.com\n"),
            ("app/s.py", "import os\nA = os.environ[\"SMTP_HOST\"]\n"),
        ],
        false,
    );
    assert!(names(&r, "missing-from-example").is_empty());
    assert!(r.commented.contains("SMTP_HOST"));
    assert!(!r.declared.contains_key("SMTP_HOST"));
}

#[test]
fn no_example_file_means_nothing_to_compare() {
    let r = run(
        &[("app/s.py", "import os\nA = os.environ[\"ANYTHING\"]\n")],
        false,
    );
    assert!(r.findings.is_empty());
}

/// 🔴 Live false finding on chanx: `.env.EXAMPLE` holds two keys and
/// `DJANGO_SECRET_KEY` sits in a committed `.env.test`, with a value.
#[test]
fn committed_env_file_silences_side_b() {
    let r = run(
        &[
            (".env.EXAMPLE", "OPENAI_API_KEY=\n"),
            (".env.test", "DJANGO_SECRET_KEY=mock-secret\n"),
            (
                "app/settings.py",
                "SECRET = env.str(\"DJANGO_SECRET_KEY\")\n",
            ),
        ],
        false,
    );
    assert!(names(&r, "missing-from-example").is_empty());
    assert!(r.known.contains("DJANGO_SECRET_KEY"));
    assert!(!r.declared.contains_key("DJANGO_SECRET_KEY"));
}

// ---------------------------------------------------------------------------
// Reads that cannot crash: species H
// ---------------------------------------------------------------------------

#[test]
fn read_inside_try_except_is_not_required() {
    let r = run(
        &[
            (".env.example", "DEBUG=1\n"),
            (
                "app/s.py",
                "import os\ntry:\n    A = os.environ[\"SMTP_HOST\"]\nexcept KeyError:\n    A = \"localhost\"\n",
            ),
        ],
        false,
    );
    assert!(names(&r, "missing-from-example").is_empty());
}

#[test]
fn read_behind_an_in_check_is_not_required() {
    let r = run(
        &[
            (".env.example", "DEBUG=1\n"),
            (
                "app/s.py",
                "import os\nA = None\nif \"SMTP_HOST\" in os.environ:\n    A = os.environ[\"SMTP_HOST\"]\n",
            ),
        ],
        false,
    );
    assert!(names(&r, "missing-from-example").is_empty());
}

/// 🔴 Live false finding on sysreptor:
/// `elif load_from_env and 'ENABLED_PLUGINS' in os.environ:`
#[test]
fn membership_check_inside_a_boolean_condition_counts() {
    let r = run(
        &[
            (".env.example", "DEBUG=1\n"),
            (
                "app/s.py",
                "import os\nflag = True\nv = None\nif flag and \"ENABLED_PLUGINS\" in os.environ:\n    v = os.environ[\"ENABLED_PLUGINS\"]\n",
            ),
        ],
        false,
    );
    assert!(names(&r, "missing-from-example").is_empty());
}

#[test]
fn membership_check_in_an_elif_counts_too() {
    let r = run(
        &[
            (".env.example", "DEBUG=1\n"),
            (
                "app/s.py",
                "import os\nflag = False\nv = None\nif flag:\n    v = 1\nelif \"PLUG\" in os.environ:\n    v = os.environ[\"PLUG\"]\n",
            ),
        ],
        false,
    );
    assert!(names(&r, "missing-from-example").is_empty());
}

#[test]
fn a_name_the_code_sets_itself_is_not_required() {
    let r = run(
        &[
            (".env.example", "DEBUG=1\n"),
            (
                "app/s.py",
                "import os\nos.environ[\"DJANGO_SETTINGS\"] = \"app.settings\"\nA = os.environ[\"DJANGO_SETTINGS\"]\n",
            ),
        ],
        false,
    );
    assert!(names(&r, "missing-from-example").is_empty());
}

#[test]
fn setdefault_counts_as_setting_it() {
    let r = run(
        &[
            (".env.example", "DEBUG=1\n"),
            (
                "app/s.py",
                "import os\nos.environ.setdefault(\"APP_MODE\", \"dev\")\nA = os.environ[\"APP_MODE\"]\n",
            ),
        ],
        false,
    );
    assert!(names(&r, "missing-from-example").is_empty());
}

// ---------------------------------------------------------------------------
// Class A-near: a typo, and only a typo
// ---------------------------------------------------------------------------

#[test]
fn near_name_is_hard() {
    let r = run(
        &[
            (".env.example", "SMTP_HOST=mail\n"),
            ("app/s.py", "import os\nH = os.getenv(\"SMTP_HOSTNAME\")\n"),
        ],
        false,
    );
    assert_eq!(names(&r, "near-miss"), vec!["SMTP_HOST"]);
}

#[test]
fn separator_and_case_difference_counts_as_the_same_word() {
    let r = run(
        &[
            (".env.example", "REDISURL=redis://x\n"),
            ("app/s.py", "import os\nU = os.getenv(\"REDIS_URL\")\n"),
        ],
        false,
    );
    assert_eq!(names(&r, "near-miss"), vec!["REDISURL"]);
}

/// 🔴 Live false finding: SHORTIFY_UVICORN_HOST declared, ..._PORT read.
#[test]
fn a_whole_different_tail_is_not_a_near_miss() {
    let r = run(
        &[
            (".env.example", "SHORTIFY_UVICORN_HOST=0.0.0.0\n"),
            (
                "app/s.py",
                "import os\nP = os.getenv(\"SHORTIFY_UVICORN_PORT\")\n",
            ),
        ],
        false,
    );
    assert!(names(&r, "near-miss").is_empty());
}

/// 🔴 Live false finding: REGISTRY_SERVER_MTLS_CERT_FILE against ..._CA_CERT_FILE.
#[test]
fn an_added_segment_is_not_a_typo() {
    let r = run(
        &[
            (".env.example", "REGISTRY_SERVER_MTLS_CERT_FILE=/a\n"),
            (
                "app/s.py",
                "import os\nC = os.getenv(\"REGISTRY_SERVER_MTLS_CA_CERT_FILE\")\n",
            ),
        ],
        false,
    );
    assert!(names(&r, "near-miss").is_empty());
}

/// 🔴 Live false finding: API_WORKER_ID against API_WORKERS.
#[test]
fn a_plural_is_not_a_typo() {
    let r = run(
        &[
            (".env.example", "API_WORKER_ID=1\n"),
            ("app/s.py", "import os\nW = os.getenv(\"API_WORKERS\")\n"),
        ],
        false,
    );
    assert!(names(&r, "near-miss").is_empty());
}

#[test]
fn a_real_typo_inside_one_segment_is_still_caught() {
    let r = run(
        &[
            (".env.example", "MAILGUN_SENDR_ADDRESS=a@b.c\n"),
            (
                "app/s.py",
                "import os\nS = os.getenv(\"MAILGUN_SENDER_ADDRESS\")\n",
            ),
        ],
        false,
    );
    assert_eq!(names(&r, "near-miss"), vec!["MAILGUN_SENDR_ADDRESS"]);
}

#[test]
fn unread_alone_is_soft_and_hidden_by_default() {
    let files: &[(&str, &str)] = &[
        (".env.example", "LEGACY_FEATURE_FLAG=0\n"),
        ("app/s.py", "import os\nD = os.getenv(\"DEBUG\")\n"),
    ];
    assert!(run(files, false).findings.is_empty());
    let r = run(files, true);
    assert_eq!(names(&r, "unread"), vec!["LEGACY_FEATURE_FLAG"]);
    assert!(r.findings.iter().all(|f| !f.hard));
}

// ---------------------------------------------------------------------------
// Side A does not lean on the reader list
// ---------------------------------------------------------------------------

/// 🔴 Live false finding: MangAdventure reads SECRET_KEY through
/// `env.secret(...)`, a helper of its own, and it landed among the dead.
#[test]
fn a_project_specific_wrapper_still_counts_as_reading() {
    let r = run(
        &[
            (".env.example", "SECRET_KEY=x\n"),
            ("app/settings.py", "SECRET_KEY = env.secret('SECRET_KEY')\n"),
        ],
        true,
    );
    assert!(names(&r, "unread").is_empty());
}

#[test]
fn a_truly_absent_name_is_still_reported() {
    let r = run(
        &[
            (".env.example", "ANCIENT_UNUSED_KNOB=1\n"),
            ("app/s.py", "import os\nx = os.getenv('DEBUG')\n"),
        ],
        true,
    );
    assert_eq!(names(&r, "unread"), vec!["ANCIENT_UNUSED_KNOB"]);
}

#[test]
fn javascript_read_prevents_an_unread_finding() {
    let r = run(
        &[
            (".env.example", "VITE_API_URL=http://x\n"),
            ("web/main.ts", "const u = import.meta.env.VITE_API_URL\n"),
        ],
        true,
    );
    assert!(r.findings.is_empty());
}

#[test]
fn compose_substitution_counts_as_a_reader() {
    let r = run(
        &[
            (".env.example", "POSTGRES_PASSWORD=secret\n"),
            (
                "docker-compose.yml",
                "services:\n  db:\n    environment:\n      POSTGRES_PASSWORD: ${POSTGRES_PASSWORD}\n",
            ),
        ],
        true,
    );
    assert!(r.findings.is_empty());
}

#[test]
fn go_read_is_collected_but_never_required() {
    let r = run(
        &[
            (".env.example", "DEBUG=1\n"),
            (
                "main.go",
                "package main\nfunc main() { _ = os.Getenv(\"SMTP_HOST\") }\n",
            ),
        ],
        false,
    );
    assert!(r.other_reads.contains_key("SMTP_HOST"));
    assert!(names(&r, "missing-from-example").is_empty());
}

// ---------------------------------------------------------------------------
// Susceptibility belongs to side A, and only to it
// ---------------------------------------------------------------------------

/// 🔴 Derived names make absence unprovable. They do not stop a crash.
#[test]
fn pydantic_softens_side_a_and_leaves_side_b_alone() {
    let r = run(
        &[
            (".env.example", "DEBUG=1\nLEGACY_FLAG=0\n"),
            (
                "app/config.py",
                "from pydantic_settings import BaseSettings\nclass S(BaseSettings):\n    model_config = SettingsConfigDict(env_prefix='APP_')\n",
            ),
            ("app/s.py", "import os\nA = os.environ[\"SMTP_HOST\"]\n"),
        ],
        true,
    );
    assert!(r.protected());
    assert!(r
        .findings
        .iter()
        .any(|f| f.kind == "missing-from-example" && f.hard));
    assert!(r
        .findings
        .iter()
        .filter(|f| f.kind == "unread")
        .all(|f| !f.hard));
}

/// 🔴 The regression that made the tool call itself protected: a Go signal
/// matched inside a Python file.
#[test]
fn a_go_signal_does_not_fire_inside_a_python_file() {
    let r = run(
        &[
            (".env.example", "DEBUG=1\n"),
            (
                "app/s.py",
                "# this module explains AutomaticEnv() and envconfig:\"x\" in prose\nimport os\nA = os.environ[\"SMTP_HOST\"]\n",
            ),
        ],
        false,
    );
    assert!(!r.protected(), "{:?}", r.protections);
    assert!(r.findings.iter().any(|f| f.hard));
}

#[test]
fn a_dynamic_read_blinds_side_a_only() {
    let r = run(
        &[
            (".env.example", "DEBUG=1\n"),
            (
                "app/s.py",
                "import os\ndef get(name):\n    return os.environ[name]\nA = os.environ[\"SMTP_HOST\"]\n",
            ),
        ],
        false,
    );
    assert!(r.protected());
    assert_eq!(names(&r, "missing-from-example"), vec!["SMTP_HOST"]);
}

/// 🔴 On the first eight live projects, seven came back PROTECTED, usually
/// off one `os.environ[name]` in conftest.py.
#[test]
fn a_dynamic_read_in_a_test_file_protects_nothing() {
    let r = run(
        &[
            (".env.example", "DEBUG=1\n"),
            (
                "tests/conftest.py",
                "import os\ndef g(n):\n    return os.environ[n]\n",
            ),
            ("app/s.py", "import os\nA = os.environ[\"SMTP_HOST\"]\n"),
        ],
        false,
    );
    assert!(!r.protected(), "{:?}", r.protections);
}

// ---------------------------------------------------------------------------
// The contract
// ---------------------------------------------------------------------------

#[test]
fn a_file_that_does_not_parse_leaves_a_trace() {
    let r = run(
        &[
            (".env.example", "DEBUG=1\n"),
            ("app/broken.py", "def (((:\n"),
        ],
        false,
    );
    assert_eq!(r.unparsed.len(), 1);
}

#[test]
fn an_empty_file_is_counted_not_ignored() {
    let r = run(
        &[(".env.example", "DEBUG=1\n"), ("app/empty.py", "")],
        false,
    );
    assert!(r.files_skipped >= 1);
}
