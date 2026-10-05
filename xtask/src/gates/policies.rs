//! The lablet policies against copies of the registry with one mistake
//! each, through the `weaver check` step's own arguments: each rule refuses
//! the mistake it's there for, by its id, and the copy without one passes.

use std::path::{Path, PathBuf};
use std::process::Command;

use super::weaver_check_args;
use crate::workspace::fixture::TempDir;
use crate::workspace::repo_root;

const REGISTRY: &str = "lablet/telemetry/registry";
const POLICY: &str = "lablet/telemetry/policies/annotations.rego";

const RUN_EVENTS: &str = "application/run/events.yaml";
const GROUPS: &str = "shared/groups.yaml";
const ATTRIBUTES: &str = "shared/attributes.yaml";
const ROOT_EVENTS: &str = "apps/lablet/events.yaml";

const CHAT_VALUE: &str = "            value: chat\n      - ref: gen_ai.provider.name";
const RETRY: &str = "      lablet:\n        emit: span_event\n";
const EXCEPTION: &str = "      lablet:\n        severity: warn\n";
const DIGEST: &str = "      - ref: lablet.config.digest\n        requirement_level: required\n        \
                      annotations:\n          lablet:\n            join: true\n";
const TURN: &str = "      - ref: lablet.turn\n        requirement_level: required\n      - ref: \
                    lablet.attempt";
const TRIAL: &str = "lablet.trial\n        requirement_level:\n          \
                     conditionally_required: If the run request named a trial.\n        \
                     annotations:\n          lablet:\n            join: true";
const RETRY_ATTRIBUTES: &str = "    attributes:\n      - ref: lablet.attempt\n        \
                                requirement_level: required\n      - ref: lablet.retry.will_retry";

/// One mistake: the rule that refuses it, and the change that makes it.
struct Case {
    rule: &'static str,
    change: Change,
}

enum Change {
    /// The one place `old` is in the registry's file `file` becomes `new`.
    Edit {
        file: &'static str,
        old: &'static str,
        new: String,
    },
    /// The registry's file `from` moves to `to`.
    Move {
        from: &'static str,
        to: &'static str,
    },
}

fn case(rule: &'static str, file: &'static str, old: &'static str, new: String) -> Case {
    Case {
        rule,
        change: Change::Edit { file, old, new },
    }
}

#[expect(
    clippy::too_many_lines,
    reason = "a table of cases, a few lines to each"
)]
fn cases() -> Vec<Case> {
    vec![
        case(
            "lablet_join_unmarked",
            GROUPS,
            DIGEST,
            "      - ref: lablet.config.digest\n        requirement_level: required\n".to_owned(),
        ),
        case(
            "lablet_join_not_true",
            GROUPS,
            TRIAL,
            TRIAL.replace("join: true", "join: \"true\""),
        ),
        case(
            "lablet_fixed_value_not_allowed",
            RUN_EVENTS,
            CHAT_VALUE,
            CHAT_VALUE.replace("value: chat", "value: 3"),
        ),
        case(
            "lablet_fixed_value_not_required",
            RUN_EVENTS,
            "      - ref: gen_ai.operation.name\n        brief: Always `chat`.\n        \
             requirement_level: required",
            "      - ref: gen_ai.operation.name\n        brief: Always `chat`.\n        \
             requirement_level: recommended"
                .to_owned(),
        ),
        case(
            "lablet_value_and_values",
            RUN_EVENTS,
            CHAT_VALUE,
            CHAT_VALUE.replace("value: chat", "value: chat\n            values: [chat]"),
        ),
        case(
            "lablet_annotation_unknown",
            RUN_EVENTS,
            CHAT_VALUE,
            CHAT_VALUE.replace("value: chat", "vlaue: chat"),
        ),
        case(
            "lablet_annotation_unknown",
            RUN_EVENTS,
            EXCEPTION,
            format!("{EXCEPTION}        sevrity: warn\n"),
        ),
        case(
            "lablet_annotation_not_a_map",
            RUN_EVENTS,
            "          lablet:\n            value: chat\n      - ref: gen_ai.provider.name",
            "          lablet: chat\n      - ref: gen_ai.provider.name".to_owned(),
        ),
        case(
            "lablet_annotation_not_a_map",
            RUN_EVENTS,
            EXCEPTION,
            "      lablet: warn\n".to_owned(),
        ),
        case(
            "lablet_annotation_on_definition",
            ATTRIBUTES,
            "  - key: lablet.turn\n    type: int\n",
            "  - key: lablet.turn\n    type: int\n    annotations:\n      lablet:\n        value: \
             \"1\"\n"
                .to_owned(),
        ),
        case(
            "lablet_values_not_allowed",
            RUN_EVENTS,
            TURN,
            TURN.replace(
                "required\n",
                "required\n        annotations:\n          lablet:\n            values: [one]\n",
            ),
        ),
        case(
            "lablet_values_duplicated",
            RUN_EVENTS,
            "values: [retryable, context_exhausted,",
            "values: [retryable, retryable, context_exhausted,".to_owned(),
        ),
        case(
            "lablet_join_missing",
            ROOT_EVENTS,
            "      - ref_group: attributes.lablet.join\n",
            String::new(),
        ),
        case(
            "lablet_join_inconsistent",
            RUN_EVENTS,
            TURN,
            TURN.replace(
                "required\n",
                "required\n        annotations:\n          lablet:\n            join: true\n",
            ),
        ),
        case(
            "lablet_severity_on_span_event",
            RUN_EVENTS,
            RETRY,
            format!("{RETRY}        severity: warn\n"),
        ),
        case(
            "lablet_emit_unknown",
            RUN_EVENTS,
            RETRY,
            "      lablet:\n        emit: span\n".to_owned(),
        ),
        case(
            "lablet_severity_unknown",
            RUN_EVENTS,
            EXCEPTION,
            "      lablet:\n        severity: warning\n".to_owned(),
        ),
        case(
            "lablet_join_with_value",
            GROUPS,
            DIGEST,
            format!("{DIGEST}            value: abc\n"),
        ),
        case(
            "lablet_join_with_value",
            GROUPS,
            DIGEST,
            format!("{DIGEST}            values: [abc]\n"),
        ),
        case(
            "lablet_join_on_span_event",
            RUN_EVENTS,
            RETRY_ATTRIBUTES,
            RETRY_ATTRIBUTES.replace(
                "    attributes:\n",
                "    attributes:\n      - ref_group: attributes.lablet.join\n",
            ),
        ),
        Case {
            rule: "lablet_signal_in_shared",
            change: Change::Move {
                from: "apps/lablet/spans.yaml",
                to: "shared/spans.yaml",
            },
        },
    ]
}

/// Copies the directory `from` to `to`, which doesn't exist yet.
fn copy(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for entry in std::fs::read_dir(from).unwrap() {
        let path = entry.unwrap().path();
        let target = to.join(path.file_name().unwrap());
        if path.is_dir() {
            copy(&path, &target);
        } else {
            std::fs::copy(&path, &target).unwrap();
        }
    }
}

/// A copy of the registry in `scratch`, with `change` made to it. The copy
/// is a directory named `registry`, as the tree's is.
fn mutated(scratch: &TempDir, change: Option<&Change>) -> PathBuf {
    let registry = scratch.path().join("registry");
    copy(&repo_root().join(REGISTRY), &registry);
    match change {
        None => {}
        Some(Change::Edit { file, old, new }) => {
            let path = registry.join(file);
            let text = std::fs::read_to_string(&path).unwrap();
            assert_eq!(
                text.matches(old).count(),
                1,
                "{file} holds what the case changes once, so the case changes something"
            );
            std::fs::write(&path, text.replacen(old, new, 1)).unwrap();
        }
        Some(Change::Move { from, to }) => {
            std::fs::rename(registry.join(from), registry.join(to)).unwrap();
        }
    }
    registry
}

/// What `weaver check` says of `registry`, run as the gate runs it, and
/// whether it passed.
fn check(registry: &Path) -> (bool, String) {
    let mut args: Vec<String> = weaver_check_args(false)
        .into_iter()
        .map(str::to_owned)
        .collect();
    let at = args.iter().position(|arg| arg == "--registry").unwrap() + 1;
    assert_eq!(args[at], REGISTRY);
    args[at] = registry.display().to_string();
    // The manifest's dependency paths resolve against the working directory.
    let output = Command::new("weaver")
        .args(&args)
        .current_dir(repo_root())
        .output()
        .unwrap();
    let said = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    (output.status.success(), said)
}

#[test]
fn every_rule_of_the_annotation_policy_has_a_case() {
    let policy = std::fs::read_to_string(repo_root().join(POLICY)).unwrap();
    let rules: std::collections::BTreeSet<&str> = policy
        .split("\"id\": \"")
        .skip(1)
        .filter_map(|rest| rest.split('"').next())
        .collect();
    let cased: std::collections::BTreeSet<&str> = cases().iter().map(|case| case.rule).collect();
    assert!(rules.len() >= 18, "{rules:?}");
    assert_eq!(rules, cased);
}

#[test]
fn each_mistake_in_the_registry_is_refused_by_its_rule_and_the_registry_without_one_passes() {
    let cases = cases();
    let failures: Vec<String> = std::thread::scope(|scope| {
        let checks: Vec<_> = cases
            .iter()
            .map(|case| {
                scope.spawn(move || {
                    let scratch = TempDir::new("policy");
                    let (passed, said) = check(&mutated(&scratch, Some(&case.change)));
                    (!passed && said.contains(case.rule))
                        .then_some(())
                        .ok_or_else(|| format!("{}: passed {passed}\n{said}", case.rule))
                })
            })
            .collect();
        checks
            .into_iter()
            .filter_map(|check| check.join().unwrap().err())
            .collect()
    });
    assert_eq!(failures, Vec::<String>::new());

    let scratch = TempDir::new("policy");
    let (passed, said) = check(&mutated(&scratch, None));
    assert!(passed, "{said}");
}
