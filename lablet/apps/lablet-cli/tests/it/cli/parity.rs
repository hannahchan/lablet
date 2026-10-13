//! The command line and the library make the same run of one config: the
//! config and the script of the library's doctest, whose outcome the
//! doctest asserts; and one config has one digest through both.

use std::ffi::OsString;

use lablet::{Config, Format, Lablet, Otel, OutcomeDocument, RawConfig, RunRequest, Secret};
use serde_json::json;

use super::harness::{ENDS, Lab, ran, shared};

/// The script the doctest plays: a call of `bash`, and a response that ends
/// the run.
const SCRIPT: &str = "
- response:
    content:
      - text: I'll look at what's there.
      - tool_use: { id: call_1, name: bash, input: { json: { command: echo parser.rs } } }
    finish: tool_use
    usage: { input_tokens: 120, output_tokens: 30 }
- response:
    content:
      - text: There's one file, parser.rs.
    finish: end_turn
    usage: { input_tokens: 180, output_tokens: 12 }
";

const PROMPT: &str = "What's in the directory?";

/// The test's own environment, which stands in for the one a host hands in.
fn process_environment() -> Vec<(OsString, OsString)> {
    #[expect(
        clippy::disallowed_methods,
        reason = "the test's own environment stands in for the one a host hands in"
    )]
    let environment = std::env::vars_os().collect();
    environment
}

#[tokio::test]
async fn the_command_line_and_the_library_make_the_doctest_s_outcome_of_its_config() {
    let lab = Lab::new("parity");
    let directory = lab.path().display();
    lab.write("script.yaml", SCRIPT);
    let path = lab.write(
        "lablet.yaml",
        &format!(
            "
model:
  provider: fake
  script: {directory}/script.yaml
  name: scripted-1
prompt:
  system: You answer tersely.
tools:
  builtin:
    root: {directory}/work
    enabled: [bash]
telemetry:
  file:
    path: {directory}/telemetry.otlp.jsonl
"
        ),
    );

    let command_line = lab.run(&["run", "--config", "lablet.yaml", "--prompt", PROMPT]);
    assert_eq!(command_line.code, Some(0), "{command_line:?}");

    let mut lablet = Lablet::builder(Config::from_path(&path).unwrap(), Otel::noop())
        .with_process_environment(process_environment())
        .build()
        .await
        .unwrap();
    let library = lablet.run(RunRequest::new(PROMPT).unwrap()).await;
    lablet.shutdown().await;
    let library = serde_json::to_value(OutcomeDocument::from(library.summary.outcome)).unwrap();

    let outcome = command_line.outcome();
    assert_eq!(shared(outcome.clone()), shared(library));
    // What the doctest asserts of each of its runs.
    assert_eq!(outcome["stop_reason"], json!("completed"));
    assert_eq!(
        outcome["result"]["text"],
        json!("There's one file, parser.rs.")
    );
    assert_eq!(
        (&outcome["turns"], &outcome["tool_calls"]),
        (&json!(2), &json!(1))
    );
    assert_eq!(
        (
            &outcome["usage"]["input_tokens"],
            &outcome["usage"]["output_tokens"]
        ),
        (&json!(300), &json!(42))
    );
}

/// The variables the config of the digest test below names, in order:
/// `MODEL`, `REPO`, `KEY` and `TOK`.
type Held<'a> = [(&'a str, &'a str); 4];

/// The digest `lablet check` prints for the lab's `lablet.yaml` with `held`
/// set in its environment.
fn digest_through_the_command_line(lab: &Lab, held: &Held<'_>) -> String {
    let mut command = lab.lablet(&["check", "--config", "lablet.yaml"]);
    command.envs(held.iter().copied());
    let run = ran(command, "");
    assert_eq!(run.code, Some(0), "{run:?}");
    run.stderr
        .lines()
        .find_map(|line| line.strip_prefix("digest:"))
        .unwrap_or_else(|| panic!("no digest line: {run:?}"))
        .trim()
        .to_owned()
}

/// The digest the library's check gives the config `text`, filled from
/// `held` alone, with the key set in code when `in_code` holds one.
async fn digest_through_the_library(text: &str, held: &Held<'_>, in_code: Option<&str>) -> String {
    let held: Vec<(String, String)> = held
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect();
    let mut config = RawConfig::from_str(text, Format::Yaml)
        .unwrap()
        .substituted(move |asked: &str| -> Option<OsString> {
            held.iter()
                .find(|(name, _)| name == asked)
                .map(|(_, value)| value.into())
        })
        .unwrap()
        .config()
        .unwrap();
    if let Some(key) = in_code {
        config.model.api_key = Some(Secret::new(key));
    }
    Lablet::builder(config, Otel::noop())
        .with_process_environment([(OsString::from("PATH"), OsString::from("/usr/bin"))])
        .check()
        .await
        .unwrap()
        .digest()
        .to_string()
}

/// One config has one digest through `lablet check` and through the
/// library's check. It changes with `MODEL`, which says what a run does,
/// and with nothing else: not with `REPO`, a path, which enters as written;
/// not with `KEY` or `TOK`, credentials, which leave it; not with the key
/// set in code in place of its `${VAR}`; and not with no key at all.
#[tokio::test]
async fn one_config_has_one_digest_through_both_roots_which_changes_with_the_model_and_nothing_else()
 {
    let lab = Lab::new("parity-digest");
    let directory = lab.path().display().to_string();
    lab.write("script.yaml", ENDS);
    std::fs::create_dir_all(lab.at("other")).unwrap();
    let text = |api_key: &str| {
        format!(
            "
model:
  provider: fake
  script: {directory}/script.yaml
  name: ${{MODEL}}
{api_key}prompt:
  system: You answer tersely.
tools:
  builtin:
    root: ${{REPO}}
    enabled: [bash]
    credentials:
      T: ${{TOK}}
"
        )
    };
    let with_key = text("  api_key: ${KEY}\n");
    lab.write("lablet.yaml", &with_key);
    let work = format!("{directory}/work");
    let other = format!("{directory}/other");
    let first: Held<'_> = [
        ("MODEL", "scripted-1"),
        ("REPO", &work),
        ("KEY", "sk-test-0123456789abcdef"),
        ("TOK", "tok-0123456789abcdef"),
    ];

    let digest = digest_through_the_command_line(&lab, &first);
    assert_eq!(digest.len(), 64, "{digest}");
    assert_eq!(
        digest_through_the_library(&with_key, &first, None).await,
        digest
    );

    let other_model: Held<'_> = [("MODEL", "scripted-2"), first[1], first[2], first[3]];
    assert_ne!(digest_through_the_command_line(&lab, &other_model), digest);
    assert_ne!(
        digest_through_the_library(&with_key, &other_model, None).await,
        digest
    );

    let other_repo: Held<'_> = [first[0], ("REPO", &other), first[2], first[3]];
    let other_credentials: Held<'_> = [
        first[0],
        first[1],
        ("KEY", "sk-test-fedcba9876543210"),
        ("TOK", "tok-fedcba9876543210"),
    ];
    for held in [&other_repo, &other_credentials] {
        assert_eq!(
            digest_through_the_command_line(&lab, held),
            digest,
            "{held:?}"
        );
        assert_eq!(
            digest_through_the_library(&with_key, held, None).await,
            digest,
            "{held:?}"
        );
    }
    let key_in_code = text("");
    assert_eq!(
        digest_through_the_library(&key_in_code, &first, Some("sk-test-0123456789abcdef")).await,
        digest
    );
    assert_eq!(
        digest_through_the_library(&key_in_code, &first, None).await,
        digest,
        "the key leaves the digest, so a config with none has the same"
    );
}
