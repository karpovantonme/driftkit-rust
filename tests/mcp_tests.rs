//! Tests for `driftkit mcp`, each named after the mistake it holds down.
//!
//! Ported from the Python scout, where every one of them came from a live run.
//! The species: a tool schema declares a property name, the handler reads a
//! different one, and nothing in Go connects the two string literals.

use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};

static NEXT: AtomicUsize = AtomicUsize::new(0);

fn build(files: &[(&str, &str)]) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "driftkit-mcp-{}-{}",
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

fn run(files: &[(&str, &str)]) -> driftkit::mcp::Report {
    let dir = build(files);
    let mut report = driftkit::mcp::Report::default();
    driftkit::mcp::analyse(&dir, &mut report);
    let _ = fs::remove_dir_all(&dir);
    report
}

fn kinds(r: &driftkit::mcp::Report, kind: &str) -> Vec<String> {
    let mut v: Vec<String> = r
        .findings
        .iter()
        .filter(|f| f.kind == kind)
        .map(|f| f.tool.clone())
        .collect();
    v.sort();
    v
}

const SCHEMA_TOOL: &str = r#"
package github

func NewSetLayerTool() Tool {
	return Tool{
		Name: "set_active_layer",
		InputSchema: &jsonschema.Schema{
			Type: "object",
			Properties: map[string]*jsonschema.Schema{
				"layerName": {Type: "string"},
			},
			Required: []string{"layerName"},
		},
		Handler: func(args map[string]any) error {
			layer, err := RequiredParam[string](args, "layer")
			_ = layer
			return err
		},
	}
}
"#;

#[test]
fn a_name_the_handler_never_reads_is_a_finding() {
    let r = run(&[("main.go", SCHEMA_TOOL)]);
    assert_eq!(kinds(&r, "declared-unread"), vec!["set_active_layer"]);
    assert_eq!(kinds(&r, "read-undeclared"), vec!["set_active_layer"]);
    assert!(r.findings.iter().all(|f| f.hard));
}

/// 🔴 Live phantom in github-mcp-server: a commented-out
/// `// mcp.WithString("pullRequestReviewID", ...)` was read as a live schema
/// property that exists nowhere.
#[test]
fn commented_out_code_is_not_a_schema() {
    let r = run(&[(
        "main.go",
        r#"
package github

func NewTool() Tool {
	return Tool{
		Name: "thing",
		InputSchema: &jsonschema.Schema{
			Properties: map[string]*jsonschema.Schema{
				"owner": {Type: "string"},
			},
		},
		// Properties: map[string]*jsonschema.Schema{"ghostProperty": {}},
		Handler: func(args map[string]any) error {
			_, err := RequiredParam[string](args, "owner")
			return err
		},
	}
}
"#,
    )]);
    assert!(r.findings.is_empty(), "{:?}", r.findings);
}

/// 🔴 The schema is finished by code after the literal. In github-mcp-server
/// `WithPagination(schema)` adds page and perPage, and reading the literal
/// alone puts a phantom on every paginated tool -- a third of the server.
#[test]
fn a_schema_enricher_is_followed() {
    let r = run(&[(
        "main.go",
        r#"
package github

func WithPagination(schema *jsonschema.Schema) *jsonschema.Schema {
	schema.Properties["page"] = &jsonschema.Schema{Type: "number"}
	schema.Properties["perPage"] = &jsonschema.Schema{Type: "number"}
	return schema
}

func NewListTool() Tool {
	return Tool{
		Name: "list_things",
		InputSchema: WithPagination(&jsonschema.Schema{
			Properties: map[string]*jsonschema.Schema{
				"owner": {Type: "string"},
			},
		}),
		Handler: func(args map[string]any) error {
			_, _ = RequiredParam[string](args, "owner")
			_, _ = OptionalIntParam(args, "page")
			_, _ = OptionalIntParam(args, "perPage")
			return nil
		},
	}
}
"#,
    )]);
    assert!(r.findings.is_empty(), "{:?}", r.findings);
}

/// 🔴 The handler passes `args` on whole, and the helper does the reading.
/// Without following it, `ui_get` looked like it never read `repo`, while
/// `uiGetLabels(.., args, ..)` did.
#[test]
fn a_helper_handed_args_counts_as_reading() {
    let r = run(&[(
        "main.go",
        r#"
package github

func uiGetLabels(ctx context.Context, args map[string]any) error {
	_, err := RequiredParam[string](args, "repo")
	return err
}

func NewUiTool() Tool {
	return Tool{
		Name: "ui_get",
		InputSchema: &jsonschema.Schema{
			Properties: map[string]*jsonschema.Schema{
				"repo": {Type: "string"},
			},
		},
		Handler: func(args map[string]any) error {
			return uiGetLabels(ctx, args)
		},
	}
}
"#,
    )]);
    assert!(r.findings.is_empty(), "{:?}", r.findings);
}

/// 🔴 Helpers cannot be listed by name: every project writes its own.
/// `RequiredBigInt` was not on the list and produced a phantom.
#[test]
fn an_unknown_helper_shape_still_counts_as_a_read() {
    let r = run(&[(
        "main.go",
        r#"
package github

func NewTool() Tool {
	return Tool{
		Name: "thing",
		InputSchema: &jsonschema.Schema{
			Properties: map[string]*jsonschema.Schema{
				"issue_number": {Type: "number"},
			},
		},
		Handler: func(args map[string]any) error {
			_, err := RequiredBigInt(args, "issue_number")
			return err
		},
	}
}
"#,
    )]);
    assert!(r.findings.is_empty(), "{:?}", r.findings);
}

/// A protected server is skipped, not scanned and then filtered. Where zod
/// feeds both the schema and the handler type, a mismatch cannot exist.
#[test]
fn a_protected_server_is_not_scanned_at_all() {
    let r = run(&[(
        "index.ts",
        r#"
const schema = z.object({ owner: z.string() });
server.registerTool("thing", { inputSchema: zodToJsonSchema(schema) },
  async (args: z.infer<typeof schema>) => args.owner);
"#,
    )]);
    assert_eq!(r.verdict, "protected");
    assert!(r.findings.is_empty());
    assert_eq!(r.tools_total, 0, "a protected server is never parsed");
}

/// 🔴 Both halves of a risk signal must meet in ONE file. Gluing a repository
/// into one string married an `inputSchema` from one file to an `args.foo`
/// from another and gave 4 false susceptibles out of 5.
#[test]
fn risk_signals_must_meet_in_one_file() {
    let dir = build(&[
        ("schema.ts", "export const inputSchema: {type: 'object', properties: {}}\n"),
        ("other.ts", "function f(x) { return x.args.foo }\n"),
    ]);
    let c = driftkit::mcp::classify::classify(&dir);
    let _ = fs::remove_dir_all(&dir);
    assert_ne!(c.verdict, driftkit::mcp::classify::Verdict::Susceptible);
}

/// The boundary between tools has to be a function, not a window of N
/// characters: a sliding window overlapped neighbours and reported the same
/// tool twice with two different schemas.
#[test]
fn two_tools_in_one_file_do_not_bleed_into_each_other() {
    let r = run(&[(
        "main.go",
        r#"
package github

func NewFirst() Tool {
	return Tool{
		Name: "first_tool",
		InputSchema: &jsonschema.Schema{
			Properties: map[string]*jsonschema.Schema{
				"alpha": {Type: "string"},
			},
		},
		Handler: func(args map[string]any) error {
			_, err := RequiredParam[string](args, "alpha")
			return err
		},
	}
}

func NewSecond() Tool {
	return Tool{
		Name: "second_tool",
		InputSchema: &jsonschema.Schema{
			Properties: map[string]*jsonschema.Schema{
				"beta": {Type: "string"},
			},
		},
		Handler: func(args map[string]any) error {
			_, err := RequiredParam[string](args, "beta")
			return err
		},
	}
}
"#,
    )]);
    assert_eq!(r.tools_bound, 2);
    assert!(r.findings.is_empty(), "{:?}", r.findings);
}

/// An opaque handler hands `args` somewhere the scanner cannot follow, so
/// "declared but never read" is not a claim it may make about that tool.
#[test]
fn an_opaque_handler_gets_no_declared_unread_claim() {
    let r = run(&[(
        "main.go",
        r#"
package github

func NewTool() Tool {
	return Tool{
		Name: "opaque_tool",
		InputSchema: &jsonschema.Schema{
			Properties: map[string]*jsonschema.Schema{
				"owner": {Type: "string"},
				"repo":  {Type: "string"},
			},
		},
		Handler: func(args map[string]any) error {
			_, _ = RequiredParam[string](args, "owner")
			payload, _ := json.Marshal(args)
			return send(payload)
		},
	}
}
"#,
    )]);
    assert!(kinds(&r, "declared-unread").is_empty(), "{:?}", r.findings);
}
