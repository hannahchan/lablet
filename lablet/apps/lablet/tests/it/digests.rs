//! The digests a run's record holds: of its config, of the tools it
//! offered, and of the system prompt it sent.

use lablet::{FinishedRun, RunId};
use lablet_telemetry_registry::attribute as key;
use serde_json::{Value, json};

use crate::harness::{ENDS, Lab, Traced, read, request};

/// One run under the id `run`, of a config with `more` stated, and the
/// three digests of its wide event: the config's, the tools' and the system
/// prompt's.
async fn digests(scratch: &Lab, run: &str, more: Value) -> [String; 3] {
    let tree = scratch.tree(ENDS, more);
    let config = read(&tree);
    let digest = config.digest();
    let mut lablet = lablet::build(config).await.unwrap();

    let finished: FinishedRun = lablet.run(request().run_id(RunId::new(run).unwrap())).await;
    lablet.shutdown().await;

    let exported = scratch.exported();
    let traced = Traced::of(&exported, run);
    let wide = &traced.wide().attributes;
    let of = |key: &str| wide[key].as_str().unwrap().to_owned();
    assert_eq!(of(key::LABLET_CONFIG_DIGEST), digest);
    assert_eq!(of(key::LABLET_TOOLS_DIGEST), finished.summary.tools_digest);
    assert_eq!(
        of(key::LABLET_PROMPT_SYSTEM_DIGEST),
        finished.summary.system_prompt_digest
    );
    for digest in [
        &finished.summary.tools_digest,
        &finished.summary.system_prompt_digest,
    ] {
        assert_eq!(digest.len(), 64);
    }
    [
        of(key::LABLET_CONFIG_DIGEST),
        of(key::LABLET_TOOLS_DIGEST),
        of(key::LABLET_PROMPT_SYSTEM_DIGEST),
    ]
}

#[tokio::test]
async fn two_tool_sets_that_differ_in_one_description_differ_in_the_tools_digest() {
    let scratch = Lab::new("tools-digest");
    // A built-in tool's description says how long a call may take.
    let tools = |timeout: &str| {
        let mut builtin = scratch.builtin(&["bash", "read_file"]);
        builtin["timeout"] = json!(timeout);
        json!({ "tools": { "builtin": builtin } })
    };

    let [_, first, system] = digests(&scratch, "first", tools("2m")).await;
    let [_, again, _] = digests(&scratch, "again", tools("120s")).await;
    let [_, other, of_other] = digests(&scratch, "other", tools("1m")).await;
    let [_, fewer, _] = digests(
        &scratch,
        "fewer",
        json!({
            "tools": { "builtin": scratch.builtin(&["bash"]) },
        }),
    )
    .await;

    assert_eq!(first, again);
    assert_ne!(first, other);
    assert_ne!(first, fewer);
    assert_ne!(other, fewer);
    assert_eq!(system, of_other, "the system prompt is the same one");
}

#[tokio::test]
async fn one_lablet_run_twice_has_the_same_digests_both_times() {
    let scratch = Lab::new("same-digests");
    let config = scratch.config(
        ENDS,
        json!({ "tools": { "builtin": scratch.builtin(&["bash"]) } }),
    );
    let mut lablet = lablet::build(config).await.unwrap();

    let first = lablet.run(request()).await.summary;
    let second = lablet.run(request()).await.summary;
    lablet.shutdown().await;

    assert_ne!(first.outcome.run_id, second.outcome.run_id);
    let exported = scratch.exported();
    let of = |run_id: &RunId| {
        let traced = Traced::of(&exported, run_id.as_str());
        [
            key::LABLET_CONFIG_DIGEST,
            key::LABLET_TOOLS_DIGEST,
            key::LABLET_PROMPT_SYSTEM_DIGEST,
        ]
        .map(|key| traced.wide().attributes[key].clone())
    };
    assert_eq!(of(&first.outcome.run_id), of(&second.outcome.run_id));
    assert_eq!(first.tools_digest, second.tools_digest);
    assert_eq!(first.system_prompt_digest, second.system_prompt_digest);
}

#[tokio::test]
async fn two_system_prompts_differ_in_the_digest_and_one_prompt_has_one_digest() {
    let scratch = Lab::new("system-digest");
    let prompt = |system: &str| json!({ "prompt": { "system": system } });

    let [config, tools, first] = digests(&scratch, "first", prompt("You fix tests.")).await;
    let [same_config, _, again] = digests(&scratch, "again", prompt("You fix tests.")).await;
    let [other_config, of_other, other] =
        digests(&scratch, "other", prompt("You fix tests, tersely.")).await;

    assert_eq!(first, again);
    assert_eq!(config, same_config);
    assert_ne!(first, other);
    assert_ne!(
        config, other_config,
        "a prompt the config states is part of the config"
    );
    assert_eq!(tools, of_other, "the tools are the same ones");
}

/// The variable under test is in the record: a prompt file's text is no
/// part of the config, so only the digest of what was sent tells two of
/// them apart.
#[tokio::test]
async fn two_texts_of_one_prompt_file_share_a_config_digest_and_differ_in_the_prompts() {
    let scratch = Lab::new("prompt-file");
    let file = scratch.at("system.md");
    let from_file = json!({ "prompt": { "system": null, "system_file": file } });

    scratch.write("system.md", "You fix tests.");
    let [config, _, first] = digests(&scratch, "first", from_file.clone()).await;
    scratch.write("system.md", "You fix tests, tersely.");
    let [same_config, _, other] = digests(&scratch, "other", from_file).await;
    let [_, _, stated] = digests(
        &scratch,
        "stated",
        json!({ "prompt": { "system": "You fix tests, tersely." } }),
    )
    .await;

    assert_eq!(config, same_config);
    assert_ne!(first, other);
    assert_eq!(
        other, stated,
        "a prompt is the same prompt wherever it was read from"
    );
}

#[tokio::test]
async fn two_configs_that_differ_only_in_where_they_write_share_the_digest_of_their_records() {
    let scratch = Lab::new("outputs");

    let [plain, ..] = digests(&scratch, "plain", json!({})).await;
    let [elsewhere, ..] = digests(
        &scratch,
        "elsewhere",
        json!({
            "run": { "transcript_path": scratch.at("{run_id}.json") },
            "telemetry": { "capture_content": true, "resource": { "team": "other" } },
        }),
    )
    .await;
    let [capped, ..] = digests(&scratch, "capped", json!({ "run": { "max_turns": 5 } })).await;

    assert_eq!(plain, elsewhere);
    assert_ne!(plain, capped);
}
