//! What the library root's check and build read of where they run: nothing
//! but the config its host filled and the environment its host handed in.

use lablet_config::{ConfigError, Format, Place, RawConfig};
use lablet_prepare::ErrorClass;
use lablet_run::telemetry::generated::GenAiClientInferenceOperationDetails;
use lablet_run::telemetry::generated::key;
use lablet_secrets::Credential;
use opentelemetry::trace::noop::NoopTextMapPropagator;

use super::*;

/// A key as long as a key is, and so one that's cut.
const KEY: &str = "sk-ant-0123456789abcdef0123456789";

/// The config `text` holds, with each `${VAR}` filled from `held`.
fn filled(text: &str, held: &[(&str, &str)]) -> Result<Config, ConfigError> {
    let held: Vec<(String, String)> = held
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect();
    RawConfig::from_str(text, Format::Yaml)?
        .substituted(move |asked: &str| -> Option<std::ffi::OsString> {
            held.iter()
                .find(|(name, _)| name == asked)
                .map(|(_, value)| value.into())
        })?
        .config()
}

/// The check of `config` on the no-op `Otel`, with nothing handed in.
async fn check(config: Config) -> Result<Checked, BuildError> {
    Lablet::builder(config, Otel::noop()).check().await
}

const ENDS: &str = "
- response:
    content:
      - text: Done.
    finish: end_turn
";

/// T16: the check names the credentials a run drops and cuts, and never a
/// value.
#[tokio::test]
async fn the_check_names_what_a_run_drops_and_cuts_and_never_a_value() {
    let scratch = lablet_test_support::Scratch::new("check-names");
    let script = scratch.write("script.yaml", ENDS);
    let text = format!(
        "model: {{ provider: fake, script: '{}', api_key: '${{WORK_KEY}}' }}\n\
         prompt: {{ system: Hi. }}\n\
         telemetry: {{ otlp: {{ headers: {{ X-Short: '${{SHORT}}' }} }} }}",
        script.display()
    );
    let config = filled(&text, &[("WORK_KEY", KEY), ("SHORT", "short")]).unwrap();

    let checked = check(config).await.unwrap();

    let own: Vec<String> = checked.own().iter().map(Credential::as_own).collect();
    assert_eq!(
        own,
        [
            "WORK_KEY (line 1) -> model.api_key; no process inherits it",
            "SHORT (line 3) (under 16 bytes, not cut) -> telemetry.otlp.headers.X-Short; no \
             process inherits it",
        ]
    );
    assert!(checked.given().is_empty());
    assert!(!format!("{checked:?}").contains(KEY), "{checked:?}");
}

/// C5, per field: two runs whose `name: ${MODEL}` differs have two digests
/// and run with their names, and two whose `root: ${REPO}` differs share
/// one.
#[tokio::test]
async fn two_runs_whose_variables_differ_share_a_digest_only_where_the_field_is_taken_as_written() {
    let scratch = lablet_test_support::Scratch::new("digest-per-field");
    let script = scratch.write("script.yaml", ENDS);
    let text = format!(
        "
model: {{ provider: fake, script: '{}', name: '${{MODEL}}' }}
prompt: {{ system: 'You fix tests.' }}
telemetry: {{ capture_content: true }}
",
        script.display(),
    );
    let mut seen = Vec::new();

    for model in ["scripted-1", "scripted-2"] {
        let config = filled(&text, &[("MODEL", model)]).unwrap();
        let host = lablet_test_support::conformance::host::Host::new();
        let otel = Otel::new(
            &host.tracer_provider(),
            &host.logger_provider(),
            NoopTextMapPropagator::new(),
        );
        let digest = config.digest().to_string();
        let mut lablet = Lablet::builder(config, otel).build().await.unwrap();
        let finished = lablet.run(crate::RunRequest::new("Fix it.").unwrap()).await;
        lablet.shutdown().await;
        let run_id = finished.summary.outcome.run_id;
        let exported = host.exported();
        let of_run = |attributes: &lablet_test_support::conformance::otlp::Attributes| {
            attributes.get(key::GEN_AI_CONVERSATION_ID)
                == Some(&serde_json::Value::String(run_id.to_string()))
        };
        let root = exported
            .spans_of(lablet_run::telemetry::generated::LabletInvokeAgent::GEN_AI_OPERATION_NAME)
            .into_iter()
            .find(|span| of_run(&span.attributes))
            .unwrap();
        assert_eq!(
            root.attributes[key::LABLET_CONFIG_DIGEST].as_str().unwrap(),
            digest
        );
        assert_eq!(
            root.attributes[key::GEN_AI_REQUEST_MODEL].as_str().unwrap(),
            model
        );
        assert!(
            exported
                .records_of(GenAiClientInferenceOperationDetails::NAME)
                .iter()
                .any(|record| of_run(&record.attributes)),
            "content was captured"
        );
        seen.push(digest);
    }
    assert_ne!(
        seen[0], seen[1],
        "a filled name is behaviour, so it enters the digest"
    );

    let for_root = |repo: &str| {
        let text = format!(
            "model: {{ provider: fake, script: '{}' }}\nprompt: {{ system: Hi. }}\ntools: {{ \
             builtin: {{ root: '${{REPO}}', enabled: [read_file] }} }}",
            script.display()
        );
        filled(&text, &[("REPO", repo)]).unwrap().digest()
    };
    assert_eq!(
        for_root("/home/a/repo"),
        for_root("/home/b/repo"),
        "a path is taken as written"
    );
}

/// C16: a value a variable gave is refused as the config writes it, and
/// nothing of what the variable holds is in the message.
#[tokio::test]
async fn a_value_a_variable_gave_is_refused_as_it_is_written() {
    const HELD: &str = "internal-gateway-7f3a";
    let text = "model: { provider: openai, base_url: '${GATEWAY}' }\nprompt: { system: Hi. }";

    let error = check(filled(text, &[("GATEWAY", HELD)]).unwrap())
        .await
        .unwrap_err();

    assert_eq!(
        error,
        BuildError::Config(ConfigError::Invalid {
            key: "model.base_url".to_owned(),
            place: Some(Place::Line(1)),
            value: Some("\"${GATEWAY}\"".to_owned()),
            reason: "a URL begins `http://` or `https://` and names a host, as \
                     `https://gateway.example` does"
                .to_owned(),
        })
    );
    for shown in [error.to_string(), format!("{error:?}")] {
        assert!(!shown.contains(HELD), "{shown}");
    }
    assert_eq!(error.class(), ErrorClass::Config);
    assert_eq!(
        check(filled(text, &[("GATEWAY", "http://localhost:4000/v1")]).unwrap())
            .await
            .map(drop),
        Ok(()),
        "a URL passes, and the provider that has no adapter yet isn't selected"
    );
}

/// C10: a variable a config names that isn't set is a config error when the
/// host fills the file.
#[test]
fn a_variable_that_is_not_set_is_refused_by_the_key_it_is_in() {
    let error = filled(
        "model: { provider: openai, name: '${MODEL}' }\nprompt: { system: Hi. }",
        &[],
    )
    .unwrap_err();

    assert_eq!(
        error.to_string(),
        "model.name (line 1): \"${MODEL}\" is refused: `MODEL` isn't set; set it, or give a default, \
         as ${MODEL:-default}"
    );
}

/// C13: a setting the provider can't apply is refused by the check, and
/// the config without it passes, with the resolved config leaving out what
/// the provider can't apply.
#[tokio::test]
async fn the_check_refuses_a_setting_the_provider_cannot_apply_and_resolves_the_rest() {
    let held = [("ANTHROPIC_API_KEY", KEY)];
    for (text, key_name, reached) in [
        (
            "model: { seed: 7, api_key: '${ANTHROPIC_API_KEY}' }\nprompt: { system: Hi. }",
            "model.seed",
            "the provider `anthropic`",
        ),
        (
            "model: { provider: openai, base_url: 'http://localhost:11434/v1', thinking: adaptive }\n\
             prompt: { system: Hi. }",
            "model.thinking",
            "the provider `openai` over its API `chat_completions`",
        ),
    ] {
        let error = check(filled(text, &held).unwrap()).await.unwrap_err();
        assert!(
            matches!(
                &error,
                BuildError::Config(ConfigError::NotApplied { key, reached: said, .. })
                    if *key == key_name && said == reached
            ),
            "{error:?}"
        );
        assert!(error.to_string().contains(key_name), "{error}");
        assert!(error.to_string().contains(reached), "{error}");
    }

    for (text, left_out) in [
        (
            "model: { api_key: '${ANTHROPIC_API_KEY}' }\nprompt: { system: Hi. }",
            "seed",
        ),
        (
            "model: { provider: openai, base_url: 'http://localhost:11434/v1' }\n\
             prompt: { system: Hi. }",
            "thinking",
        ),
    ] {
        let config = filled(text, &held).unwrap();
        let checked = check(config.clone()).await.unwrap();
        let resolved = serde_json::to_value(checked.resolved()).unwrap();

        assert!(resolved["model"].get(left_out).is_none(), "{resolved}");
        assert_eq!(checked.resolved(), &config.resolved());
    }
}

/// C17: a config that states only what it must is checked into the
/// resolved config of the spec's own block, which states every default.
#[tokio::test]
async fn a_config_that_states_only_what_it_must_resolves_to_every_default_of_the_spec() {
    let held = [("ANTHROPIC_API_KEY", KEY)];
    let mut spec: serde_json::Value =
        serde_saphyr::from_str(include_str!("../../../shared/config/src/tests/spec.yaml")).unwrap();
    // The spec's block lists two servers as examples, which no default has.
    spec["tools"]["mcp"] = serde_json::json!([]);
    let spec = RawConfig::from_str(&spec.to_string(), Format::Json)
        .unwrap()
        .substituted(|name: &str| (name == "ANTHROPIC_API_KEY").then(|| KEY.into()))
        .unwrap()
        .config()
        .unwrap();

    let checked = check(
        filled(
            "model: { api_key: '${ANTHROPIC_API_KEY}' }\nprompt: { system: You are ... }",
            &held,
        )
        .unwrap(),
    )
    .await
    .unwrap();

    assert_eq!(
        serde_json::to_value(checked.resolved()).unwrap(),
        serde_json::to_value(spec.resolved()).unwrap()
    );
    assert_eq!(checked.digest(), spec.digest());
    assert_eq!(checked.tools(), []);
}

/// R17's URL form in a library, by its check and its build alike: a
/// password the text writes out in `model.base_url` or
/// `telemetry.otlp.endpoint` is refused unshown, by line or as an
/// override; one set in code, after the text wrote none, `null` or a
/// `${VAR}` that came out empty, is own, and cut, since a host has no
/// other way to give it. A build of the gateway's config goes on past every
/// rule of the URL and makes its target.
#[tokio::test]
async fn a_url_s_password_written_out_is_refused_and_one_set_in_code_is_own() {
    let scratch = lablet_test_support::Scratch::new("library-url-password");
    let script = scratch.write("script.yaml", ENDS);
    let password = "gwsecretpassword1234567";
    let literal = format!("https://gwuser:{password}@gw.example/v1");
    let gateway = |written: &str| {
        format!(
            "model: {{ provider: anthropic, api_key: '${{K}}'{written} }}\nprompt: {{ system: Hi. }}"
        )
    };
    let collector = |written: &str| {
        format!(
            "model: {{ provider: fake, script: '{}', api_key: '${{K}}' }}\nprompt: {{ system: Hi. }}\ntelemetry: {{ otlp: {{ protocol: grpc{written} }} }}",
            script.display()
        )
    };
    let set_in_code = |config: &mut Config, key: &str| {
        if key == "model.base_url" {
            config.model.base_url = Some(literal.clone());
        } else {
            config.telemetry.otlp.endpoint = Some(literal.clone());
        }
    };
    let fill = |name: &str| (name == "K").then(|| std::ffi::OsString::from(KEY));

    for (key, text, line) in [
        ("model.base_url", &gateway as &dyn Fn(&str) -> String, 1),
        ("telemetry.otlp.endpoint", &collector, 3),
    ] {
        let short = key.rsplit('.').next().unwrap();
        let written = filled(&text(&format!(", {short}: '{literal}'")), &[("K", KEY)]).unwrap();
        let mut raw = RawConfig::from_str(&text(""), Format::Yaml).unwrap();
        raw.set(&format!("{key}={literal}")).unwrap();
        let overridden = raw.substituted(fill).unwrap().config().unwrap();
        let why = "is refused: its user or password isn't a ${VAR}, and what it holds isn't shown";
        let rotate = "and rotate it if this was one";
        for (config, expected) in [
            (
                written,
                format!(
                    "{key} (line {line}) {why}; write https://${{GW_USER}}:${{GW_PASS}}@gw/v1, \
                     {rotate}"
                ),
            ),
            (
                overridden,
                format!(
                    "{key} (an override) {why}; write --set \
                     '{key}=https://${{GW_USER}}:${{GW_PASS}}@gw/v1', {rotate}"
                ),
            ),
        ] {
            let checked = check(config.clone()).await.unwrap_err();
            let built = Lablet::builder(config, Otel::noop())
                .build()
                .await
                .err()
                .unwrap();
            for refusal in [checked.to_string(), built.to_string()] {
                assert_eq!(refusal, expected);
            }
            assert!(!format!("{checked:?}{built:?}").contains(password));
        }

        for written in [
            String::new(),
            format!(", {short}: null"),
            format!(", {short}: '${{GW:-}}'"),
            format!(", {short}: '${{GW}}'"),
        ] {
            let mut config = filled(&text(&written), &[("K", KEY), ("GW", "")]).unwrap();
            set_in_code(&mut config, key);

            let checked = check(config.clone()).await.unwrap();

            let own: Vec<String> = checked.own().iter().map(Credential::as_own).collect();
            assert_eq!(
                own,
                [
                    "K (line 1) -> model.api_key; no process inherits it".to_owned(),
                    format!("{key} (set in code); no process inherits it"),
                ],
                "{written}"
            );
            assert!(!format!("{checked:?}").contains(password));
            Lablet::builder(config, Otel::noop())
                .build()
                .await
                .unwrap_or_else(|error| panic!("{key}{written}: {error}"));
        }
    }
}

/// A URL as written, the variables that fill it, and the refusal expected.
type Case<'a> = (&'a str, Vec<(&'a str, &'a str)>, String);

/// A URL whose text holds an `@` past its authority, as a password's `/`
/// puts it, written out, in a default whatever its variable holds, or in a
/// whole-URL default, is refused by a library check and build by its line,
/// at the base URL and at the endpoint, and so is one a variable fills
/// with one, naming each variable; no refusal or Debug shows any of it.
#[tokio::test]
async fn a_url_that_holds_an_at_past_its_authority_is_refused_by_a_library_build() {
    let scratch = lablet_test_support::Scratch::new("library-url-at-past-authority");
    let script = scratch.write("script.yaml", ENDS);
    let gateway = |url: &str| {
        format!(
            "model: {{ provider: anthropic, api_key: '${{K}}', base_url: '{url}' }}\nprompt: {{ system: Hi. }}"
        )
    };
    let collector = |url: &str| {
        format!(
            "model: {{ provider: fake, script: '{}', api_key: '${{K}}' }}\nprompt: {{ system: Hi. }}\ntelemetry: {{ otlp: {{ endpoint: '{url}' }} }}",
            script.display()
        )
    };
    let fix = "percent-encode a `/`, `?`, `#`, `\\` or `@` in a user or password, or write an `@` \
               in a path as %40";
    for (key, text, line) in [
        ("model.base_url", &gateway as &dyn Fn(&str) -> String, 1),
        ("telemetry.otlp.endpoint", &collector, 3),
    ] {
        let written = format!(
            "{key} (line {line}) is refused: its text holds an `@` a URL can't read as a user or \
             password, and isn't shown; {fix}; rotate it if this was one"
        );
        let user = ("U", "user-abcdefghijk");
        let defaulted = "http://${U}:${P:-pa/ss-abcdefghijk}@127.0.0.1:9";
        let mut cases: Vec<Case<'_>> = vec![
            (
                "http://user-abcdefghijk:pa/ss-abcdefghijk@127.0.0.1:9",
                vec![],
                written.clone(),
            ),
            (defaulted, vec![user], written.clone()),
            (defaulted, vec![user, ("P", "")], written.clone()),
            (defaulted, vec![user, ("P", "plain")], written.clone()),
            (
                "${URL:-http://u:pa/ss-abcdefghijk@127.0.0.1:9}",
                vec![],
                written.clone(),
            ),
        ];
        cases.push((
            "${S}://${U}:${P}@127.0.0.1:9",
            vec![("S", "http://x/"), user, ("P", "sekrit-abcdefghijk")],
            format!(
                "{key} (line {line}) is refused: `S`, `U` and `P` give it an `@` a URL can't read \
                 as a user or password, and what they hold isn't shown; {fix}"
            ),
        ));
        for (url, mut held, expected) in cases {
            held.push(("K", KEY));
            let config = filled(&text(url), &held).unwrap();
            let checked = check(config.clone()).await.unwrap_err();
            let built = Lablet::builder(config, Otel::noop())
                .build()
                .await
                .err()
                .unwrap();
            for refusal in [checked.to_string(), built.to_string()] {
                assert_eq!(refusal, expected, "{url}");
            }
            assert!(
                !format!("{checked:?}{built:?}").contains("abcdefghijk"),
                "{url}"
            );
        }
        let whole = filled(
            &text("${URL}"),
            &[
                ("URL", "http://u:pa/ss-abcdefghijk@127.0.0.1:9"),
                ("K", KEY),
            ],
        )
        .unwrap();
        let refusal = check(whole).await.unwrap_err();
        assert!(
            refusal
                .to_string()
                .contains("is refused: `URL` holds an `@`"),
            "{refusal}"
        );
        assert!(!format!("{refusal}{refusal:?}").contains("abcdefghijk"));
    }
}

/// A URL a host sets in code, after the text wrote none, `null` or a
/// `${VAR}` that came out empty, that holds an `@` past its authority, as a
/// password's `/` puts it, is refused unshown by a library check and build,
/// at the base URL and at the endpoint, since nothing would find its user or
/// password to cut or to leave out of the digest.
#[tokio::test]
async fn a_url_set_in_code_that_holds_an_at_past_its_authority_is_refused_unshown() {
    let scratch = lablet_test_support::Scratch::new("library-url-at-in-code");
    let script = scratch.write("script.yaml", ENDS);
    let url = "https://gwuser:pa/ss-abcdefghijk@gw.example/v1";
    let gateway = |written: &str| {
        format!(
            "model: {{ provider: anthropic, api_key: '${{K}}'{written} }}\nprompt: {{ system: Hi. }}"
        )
    };
    let collector = |written: &str| {
        format!(
            "model: {{ provider: fake, script: '{}', api_key: '${{K}}' }}\nprompt: {{ system: Hi. }}\ntelemetry: {{ otlp: {{ protocol: grpc{written} }} }}",
            script.display()
        )
    };
    for (key, text) in [
        ("model.base_url", &gateway as &dyn Fn(&str) -> String),
        ("telemetry.otlp.endpoint", &collector),
    ] {
        let short = key.rsplit('.').next().unwrap();
        let expected = format!(
            "{key} is refused: it holds an `@` a URL can't read as a user or password, and isn't \
             shown; percent-encode a `/`, `?`, `#`, `\\` or `@` in a user or password, or write \
             an `@` in a path as %40"
        );
        for written in [
            String::new(),
            format!(", {short}: null"),
            format!(", {short}: '${{GW:-}}'"),
            format!(", {short}: '${{GW}}'"),
        ] {
            let mut config = filled(&text(&written), &[("K", KEY), ("GW", "")]).unwrap();
            if key == "model.base_url" {
                config.model.base_url = Some(url.to_owned());
            } else {
                config.telemetry.otlp.endpoint = Some(url.to_owned());
            }
            let checked = check(config.clone()).await.unwrap_err();
            let built = Lablet::builder(config, Otel::noop())
                .build()
                .await
                .err()
                .unwrap();
            for refusal in [checked.to_string(), built.to_string()] {
                assert_eq!(refusal, expected, "{key}{written}");
            }
            assert!(
                !format!("{checked:?}{built:?}").contains("abcdefghijk"),
                "{key}{written}"
            );
        }
    }
}

/// A library reads no telemetry setting, the client key's file among them,
/// so its check claims no cut of it: `own:` names the credentials it does
/// cut and not `telemetry.otlp.tls.client_key`, even for a file that
/// doesn't exist.
#[tokio::test]
async fn a_library_check_claims_no_cut_of_a_client_key_it_never_reads() {
    let scratch = lablet_test_support::Scratch::new("library-client-key");
    let script = scratch.write("script.yaml", ENDS);
    let missing = scratch.at("no-such-client.key");
    let config = filled(
        &format!(
            "model: {{ provider: fake, script: '{}', api_key: '${{WORK_KEY}}' }}\nprompt: {{ system: Hi. }}\ntelemetry: {{ otlp: {{ endpoint: 'https://collector.internal:4318', tls: {{ client_key: '{}', client_cert: '{}' }} }} }}",
            script.display(),
            missing.display(),
            missing.display(),
        ),
        &[("WORK_KEY", KEY)],
    )
    .unwrap();

    let checked = check(config).await.unwrap();

    let own: Vec<String> = checked.own().iter().map(Credential::as_own).collect();
    assert_eq!(
        own,
        ["WORK_KEY (line 1) -> model.api_key; no process inherits it"]
    );
}
