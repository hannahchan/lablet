//! What the check holds a config's credentials to, from the record the fill
//! kept and the typed fields: the key the provider needs, a credential
//! written out, an `env` entry that names a credential's variable, and
//! `bash` without an environment handed in.

use std::ffi::OsString;
use std::sync::{Arc, Mutex};

use lablet_config::{Format, RawConfig, Secret};

use super::*;

/// A key as long as a key is.
const KEY: &str = "sk-ant-0123456789abcdef0123456789";

/// The config `text` holds, with each `${VAR}` filled from `held`.
fn filled(text: &str, held: &[(&str, &str)]) -> Config {
    let held: Vec<(String, String)> = held
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect();
    RawConfig::from_str(text, Format::Yaml)
        .unwrap()
        .substituted(move |asked: &str| -> Option<OsString> {
            held.iter()
                .find(|(variable, _)| variable == asked)
                .map(|(_, value)| value.into())
        })
        .unwrap()
        .config()
        .unwrap()
}

/// The base a root hands in, holding `names`, each set to something.
#[expect(
    clippy::unnecessary_wraps,
    reason = "a base is handed in or not, and here it is"
)]
fn base(names: &[&str]) -> Base {
    Some(set(names))
}

/// An environment holding `names`, each set to something.
fn set(names: &[&str]) -> Vec<(OsString, OsString)> {
    names
        .iter()
        .map(|name| (OsString::from(name), OsString::from("set")))
        .collect()
}

/// Holds that `prepare` passes `config` with `base` handed in.
fn passes(config: &Config, base: Base) {
    prepare(config, base).map(drop).unwrap();
}

/// The refusal `prepare` makes of `config` with `base` handed in, as a
/// message.
fn refused(config: &Config, base: Base) -> String {
    prepare(config, base).map(drop).unwrap_err().to_string()
}

/// A config of the fake model with `tools` stated, whose script exists.
fn fake(scratch: &lablet_test_support::Scratch, tools: &str, held: &[(&str, &str)]) -> Config {
    let script = scratch.write(
        "script.yaml",
        "- response: { content: [{ text: Done. }], finish: end_turn }\n",
    );
    scratch.create_dir("work");
    filled(
        &format!(
            "model: {{ provider: fake, script: '{}' }}\nprompt: {{ system: Hi. }}\ntools:\n  builtin:\n    root: '{}'\n{tools}",
            script.display(),
            scratch.at("work").display()
        ),
        held,
    )
}

/// R16 and R48: a provider that needs a key is refused without one, naming
/// the variable where the config wrote one, the fix where it wrote none,
/// and the `Secret` to set where the config was made in code.
#[test]
fn a_provider_that_needs_a_key_is_refused_without_one_by_how_the_key_was_written() {
    let anthropic = |text: &str, held: &[(&str, &str)]| {
        filled(
            &format!("model: {{ {text} }}\nprompt: {{ system: Hi. }}"),
            held,
        )
    };

    assert_eq!(
        refused(&anthropic("api_key: '${K}'", &[]), None),
        "model.api_key (line 1): \"${K}\" is refused: `K` isn't set, and the provider \
         `anthropic` needs a key; set it"
    );
    assert_eq!(
        refused(&anthropic("api_key: '${K}'", &[("K", "")]), None),
        "model.api_key (line 1): \"${K}\" is refused: `K` is empty, and the provider \
         `anthropic` needs a key; set it"
    );
    assert_eq!(
        refused(&anthropic("name: m", &[]), None),
        "model.api_key isn't set: the provider `anthropic` needs a key; add api_key: \
         ${ANTHROPIC_API_KEY}"
    );
    let mut in_code = Config::default();
    in_code.prompt.system = Some("Hi.".to_owned());
    assert_eq!(
        refused(&in_code, None),
        "model.api_key isn't set: the provider `anthropic` needs a key, and a library reads no \
         variable; set config.model.api_key = Some(Secret::new(key))"
    );
    in_code.model.api_key = Some(Secret::new("  "));
    assert_eq!(
        refused(&in_code, None),
        "model.api_key is refused: it's empty, and the provider `anthropic` needs a key"
    );
    in_code.model.api_key = Some(Secret::new(KEY));
    passes(&in_code, None);
    passes(&anthropic("api_key: '${K}'", &[("K", KEY)]), None);
    // A provider that needs no key passes with the key's variable unset.
    let openai = filled(
        "model: { provider: openai, api_key: '${K}' }\nprompt: { system: Hi. }",
        &[],
    );
    passes(&openai, None);
}

/// R17, R18, R19 and R37: a credential the text writes is one `${VAR}` and
/// nothing else, with no default but an empty one, and a literal, or text
/// beside the `${VAR}`, is refused without being shown; one set in code
/// passes, read from a file or not.
#[test]
fn a_credential_written_out_or_with_a_default_is_refused_without_being_shown() {
    let pasted = "sk-ant-api03-0123456789abcdef";
    let anthropic = |key: &str| {
        filled(
            &format!("model: {{ api_key: {key} }}\nprompt: {{ system: Hi. }}"),
            &[],
        )
    };

    let literal = refused(&anthropic(&format!("'{pasted}'")), None);
    assert_eq!(
        literal,
        "model.api_key (line 1) is refused: it holds no ${VAR}, and what it holds isn't shown; \
         write api_key: ${ANTHROPIC_API_KEY}, and rotate the key if this was one"
    );
    assert!(!literal.contains(pasted));
    let mut by_override = RawConfig::from_str("prompt: { system: Hi. }", Format::Yaml).unwrap();
    by_override.set(&format!("model.api_key={pasted}")).unwrap();
    let by_override = by_override
        .substituted(|_: &str| None)
        .unwrap()
        .config()
        .unwrap();
    assert_eq!(
        refused(&by_override, None),
        "model.api_key (an override) is refused: it holds no ${VAR}, and what it holds isn't \
         shown; write --set 'model.api_key=${MY_KEY}', and rotate the key if this was one"
    );
    assert_eq!(
        refused(&anthropic("ANTHROPIC_API_KEY"), None),
        "model.api_key (line 1): \"ANTHROPIC_API_KEY\" is refused: it's a variable's name, not a \
         ${VAR}; write api_key: ${ANTHROPIC_API_KEY}"
    );
    // An access key id and a hex token are capitals and digits too, and
    // neither is a name: each is refused as a literal is, unshown.
    for shaped_like_a_name in ["AKIAIOSFODNN7EXAMPLE", "7F3A9C2E1B4D6A8F0C5E7B9D1A3F5C7E"] {
        let unshown = refused(&anthropic(&format!("'{shaped_like_a_name}'")), None);
        assert_eq!(
            unshown,
            "model.api_key (line 1) is refused: it holds no ${VAR}, and what it holds isn't shown; \
             write api_key: ${ANTHROPIC_API_KEY}, and rotate the key if this was one",
            "{shaped_like_a_name}"
        );
        assert!(!unshown.contains(shaped_like_a_name), "{unshown}");
    }
    let mut null_then_code = anthropic("null");
    null_then_code.model.api_key = Some(Secret::new("DEADBEEF0123456789ABCDEF"));
    passes(&null_then_code, None);
    assert_eq!(
        refused(&anthropic("'${K:-sk-default}'"), None),
        "model.api_key (line 1) is refused: a credential's default can only be empty, and what \
         follows :- isn't shown; write ${K:-}"
    );
    assert_eq!(
        refused(&anthropic("'$${K}'"), None),
        "model.api_key (line 1) is refused: it holds no ${VAR}, and what it holds isn't shown; \
         write api_key: ${ANTHROPIC_API_KEY}, and rotate the key if this was one",
        "an escape fills nothing"
    );
    for held in [&[][..], &[("K", KEY)][..]] {
        let beside = refused(
            &filled(
                &format!(
                    "model: {{ api_key: 'sk-ant-{pasted}${{K}}' }}\nprompt: {{ system: Hi. }}"
                ),
                held,
            ),
            None,
        );
        assert_eq!(
            beside,
            "model.api_key (line 1) is refused: it isn't one ${VAR} and nothing else, and what it \
             holds isn't shown; write api_key: ${K}, and rotate the key if this was one",
            "{held:?}"
        );
        assert!(!beside.contains("sk-ant"), "{beside}");
    }

    let mut after_reading = anthropic("'${K}'");
    after_reading.model.api_key = Some(Secret::new(pasted));
    passes(&after_reading, None);
}

/// R37, with R18 and the one-variable rule, for a tool's credential: a
/// literal, a bare name or text beside the `${VAR}` is refused without being
/// shown, and one set in code passes.
#[test]
fn a_tool_s_credential_written_out_or_beside_its_variable_is_refused_without_being_shown() {
    let scratch = lablet_test_support::Scratch::new("prepare-tool-literals");
    let pasted = "ghp_0123456789abcdef";

    let given = fake(
        &scratch,
        &format!("    enabled: [bash]\n    credentials: {{ GITHUB_TOKEN: '{pasted}' }}\n"),
        &[],
    );
    let literal = refused(&given, base(&["PATH"]));
    assert_eq!(
        literal,
        "tools.builtin.credentials.GITHUB_TOKEN (line 7) is refused: it holds no ${VAR}, and \
         what it holds isn't shown; write GITHUB_TOKEN: ${GITHUB_TOKEN}, and rotate it if this \
         was the token"
    );
    assert!(!literal.contains(pasted));
    let named = fake(
        &scratch,
        "    enabled: [bash]\n    credentials: { GITHUB_TOKEN: GITHUB_TOKEN }\n",
        &[],
    );
    assert_eq!(
        refused(&named, base(&["PATH"])),
        "tools.builtin.credentials.GITHUB_TOKEN (line 7): \"GITHUB_TOKEN\" is refused: it's a \
         variable's name, not a ${VAR}; write GITHUB_TOKEN: ${GITHUB_TOKEN}"
    );
    let access_key_id = fake(
        &scratch,
        "    enabled: [bash]\n    credentials: { AWS: AKIAIOSFODNN7EXAMPLE }\n",
        &[],
    );
    let unshown = refused(&access_key_id, base(&["PATH"]));
    assert_eq!(
        unshown,
        "tools.builtin.credentials.AWS (line 7) is refused: it holds no ${VAR}, and what it \
         holds isn't shown; write AWS: ${AWS}, and rotate it if this was the token",
        "capitals and digits with no `_` are no variable's name"
    );
    assert!(!unshown.contains("AKIA"), "{unshown}");
    let beside = fake(
        &scratch,
        "    enabled: [bash]\n    credentials: { GITHUB_TOKEN: 'ghp_${GH}' }\n",
        &[("GH", "0123456789abcdef")],
    );
    let beside = refused(&beside, base(&["PATH"]));
    assert_eq!(
        beside,
        "tools.builtin.credentials.GITHUB_TOKEN (line 7) is refused: it isn't one ${VAR} and \
         nothing else, and what it holds isn't shown; write GITHUB_TOKEN: ${GH}, and rotate it if \
         this was the token"
    );
    assert!(!beside.contains("ghp_"), "{beside}");

    let mut in_code = fake(&scratch, "    enabled: [bash]\n", &[]);
    in_code
        .tools
        .builtin
        .credentials
        .insert("GITHUB_TOKEN".to_owned(), Secret::new(pasted));
    passes(&in_code, base(&["PATH"]));
}

/// R38, R39 and R40: `tools.builtin.env` gives nothing knowingly, so an
/// entry filled from an own or given variable is refused with the fix, a
/// name both lists set is refused, and an entry that names its own variable
/// is refused where `bash` inherits it already, each by its key and line.
#[test]
fn an_env_entry_that_names_a_credential_or_its_own_variable_is_refused() {
    let scratch = lablet_test_support::Scratch::new("prepare-env");
    let held = [
        ("ANTHROPIC_API_KEY", KEY),
        ("GITHUB_TOKEN", "ghp_0123456789abcdef"),
        ("OTLP_TOKEN", "otlp-0123456789abcdef"),
        ("PORT", "8080"),
    ];
    let with = |tools: &str| fake(&scratch, tools, &held);

    let key_config = filled(
        &format!(
            "model: {{ provider: fake, script: '{}', api_key: '${{ANTHROPIC_API_KEY}}' }}\nprompt: {{ system: Hi. }}\ntools:\n  builtin:\n    root: '{}'\n    enabled: [bash]\n    env: {{ T: '${{ANTHROPIC_API_KEY}}' }}\n",
            scratch
                .write(
                    "script.yaml",
                    "- response: { content: [{ text: Done. }], finish: end_turn }\n"
                )
                .display(),
            scratch.at("work").display()
        ),
        &held,
    );
    assert_eq!(
        refused(&key_config, base(&["PATH"])),
        "tools.builtin.env.T (line 7) is refused: `ANTHROPIC_API_KEY` fills `model.api_key`, and \
         no process inherits an own credential; to give bash the key knowingly, list T under \
         `tools.builtin.credentials`"
    );

    let header_config = filled(
        &format!(
            "model: {{ provider: fake, script: '{}' }}\nprompt: {{ system: Hi. }}\ntools:\n  builtin:\n    root: '{}'\n    enabled: [bash]\n    env: {{ T: 'Bearer ${{OTLP_TOKEN}}' }}\ntelemetry: {{ otlp: {{ headers: {{ Authorization: 'Bearer ${{OTLP_TOKEN}}' }} }} }}\n",
            scratch.at("script.yaml").display(),
            scratch.at("work").display()
        ),
        &held,
    );
    assert_eq!(
        refused(&header_config, base(&["PATH"])),
        "tools.builtin.env.T (line 7) is refused: `OTLP_TOKEN` fills \
         `telemetry.otlp.headers.Authorization`, and no process inherits an own credential; to \
         give bash the value knowingly, list T under `tools.builtin.credentials`"
    );

    let given = with(
        "    enabled: [bash]\n    env: { T: '${GITHUB_TOKEN}' }\n    credentials: { GITHUB_TOKEN: '${GITHUB_TOKEN}' }\n",
    );
    assert_eq!(
        refused(&given, base(&["PATH"])),
        "tools.builtin.env.T (line 7) is refused: `GITHUB_TOKEN` is given under `credentials` \
         (line 8); list T there instead, as T: ${GITHUB_TOKEN}"
    );

    let both = with(
        "    enabled: [bash]\n    env: { FOO: bar }\n    credentials: { FOO: '${GITHUB_TOKEN}' }\n",
    );
    assert_eq!(
        refused(&both, base(&["PATH"])),
        "tools.builtin.credentials.FOO (line 8) is refused: `tools.builtin.env` sets FOO too \
         (line 7); keep one"
    );

    let same = with("    enabled: [bash]\n    env: { GITHUB_TOKEN: '${GITHUB_TOKEN}' }\n");
    assert_eq!(
        refused(&same, base(&["PATH", "GITHUB_TOKEN"])),
        "tools.builtin.env.GITHUB_TOKEN (line 7) is refused: bash inherits `GITHUB_TOKEN` as set \
         already, uncut; remove the line, or move it under `tools.builtin.credentials` to have it \
         cut"
    );
    passes(&same, base(&["PATH"]));
    let port = with("    enabled: [bash]\n    env: { PORT: '${PORT}' }\n");
    passes(&port, base(&["PATH"]));
    let renamed = with("    enabled: [bash]\n    env: { GH: '${GITHUB_TOKEN}' }\n");
    passes(&renamed, base(&["PATH", "GITHUB_TOKEN"]));
}

/// A key as long as a key is, with one `-` percent-encoded.
const ESCAPED_KEY: &str = "sk%2Dant-0123456789abcdef0123456789";

/// A token as long as a token is, given to `bash` in the tests below.
const TOKEN: &str = "ghp_0123456789abcdef";

/// A password as long as one is, in a URL's user information below.
const GW_PASSWORD: &str = "pw-0123456789abcdef";

/// A header's token, in the tests below.
const OTLP_TOKEN: &str = "otlp-0123456789abcdef";

/// A config of the fake provider with `bash`, its key filled from `key`, one
/// `env` entry `COPY` written as `env`, and `more` after it, each
/// `${VAR}` filled from a table that holds the key, the token, the password
/// and the header's token each under two names, a value no credential
/// holds, and a value under the cut's floor under two names.
fn with_env_value(
    scratch: &lablet_test_support::Scratch,
    key: &str,
    env: &str,
    more: &str,
) -> Config {
    let script = scratch.write(
        "script.yaml",
        "- response: { content: [{ text: Done. }], finish: end_turn }\n",
    );
    scratch.create_dir("work");
    filled(
        &format!(
            "model: {{ provider: fake, script: '{}', api_key: '${{{key}}}' }}\nprompt: {{ system: Hi. }}\ntools:\n  builtin:\n    root: '{}'\n    enabled: [bash]\n    env: {{ COPY: {env} }}\n{more}",
            script.display(),
            scratch.at("work").display()
        ),
        &[
            ("ANTHROPIC_API_KEY", KEY),
            ("OTHER", KEY),
            ("ESCAPED", ESCAPED_KEY),
            ("GITHUB_TOKEN", TOKEN),
            ("GH_COPY", TOKEN),
            ("OTLP_TOKEN", OTLP_TOKEN),
            ("HEADER_COPY", OTLP_TOKEN),
            ("GW_USER", "gw-user"),
            ("GW_PASS", GW_PASSWORD),
            ("PASS_COPY", GW_PASSWORD),
            ("SOMETHING_ELSE", "benign-0123456789abcdef"),
            ("SHORT", "sk-short-012"),
            ("SHORT_COPY", "sk-short-012"),
        ],
    )
}

/// An `env` entry whose filled value holds an own value is refused by the
/// field whose value it holds, with nothing of the entry shown: whether
/// another variable gave it, whole, within a text, percent-encoded or as a
/// default, or the file wrote it out, whole or split, and for the key, a
/// header's value and a URL's user information alike.
#[test]
fn an_env_entry_whose_value_holds_an_own_value_is_refused_by_the_field() {
    let scratch = lablet_test_support::Scratch::new("prepare-env-own-value");
    let with = |env: &str, more: &str| with_env_value(&scratch, "ANTHROPIC_API_KEY", env, more);
    let own = "tools.builtin.env.COPY (line 7) is refused: its value holds the one \
               `model.api_key` has, and no process inherits an own credential; to give bash the \
               key knowingly, list COPY under `tools.builtin.credentials`";

    for entry in ["'${OTHER}'", "'x${OTHER}y'", "'${ESCAPED}'"] {
        assert_eq!(refused(&with(entry, ""), base(&["PATH"])), own, "{entry}");
    }
    let literal = refused(&with(&format!("'{KEY}'"), ""), base(&["PATH"]));
    assert_eq!(literal, own);
    assert!(!literal.contains(KEY));
    // A `${VAR}` or two defaults that split the key leave it in no piece
    // of the written text, which is shown no more than the whole.
    for split in [
        "'sk-ant-0123456789${E_UNSET:-}abcdef0123456789'",
        "'${E_UNSET:-sk-ant-0123456789}${F_UNSET:-abcdef0123456789}'",
    ] {
        let refusal = refused(&with(split, ""), base(&["PATH"]));
        assert_eq!(refusal, own, "{split}");
        assert!(!refusal.contains("0123456789"), "{refusal}");
    }
    // A default that holds the key is written text, and isn't shown either.
    let default = refused(
        &with(&format!("'${{NOT_SET:-{KEY}}}'"), ""),
        base(&["PATH"]),
    );
    assert_eq!(default, own);
    assert!(!default.contains(KEY));
    // A default that holds the key percent-encoded holds it as surely, and
    // the written text shows it as it does the key.
    let encoded_default = refused(
        &with(&format!("'${{NOT_SET:-{ESCAPED_KEY}}}'"), ""),
        base(&["PATH"]),
    );
    assert_eq!(encoded_default, own);
    assert!(!encoded_default.contains(ESCAPED_KEY));

    let header = refused(
        &with(
            "'Bearer ${HEADER_COPY}'",
            "telemetry: { otlp: { headers: { Authorization: 'Bearer ${OTLP_TOKEN}' } } }\n",
        ),
        base(&["PATH"]),
    );
    assert_eq!(
        header,
        "tools.builtin.env.COPY (line 7) is refused: its value holds the one \
         `telemetry.otlp.headers.Authorization` has, and no process inherits an own credential; to \
         give bash the value knowingly, list COPY under `tools.builtin.credentials`"
    );
    assert!(!header.contains(OTLP_TOKEN));
    let url = refused(
        &with(
            "'${PASS_COPY}'",
            "telemetry: { otlp: { endpoint: 'https://${GW_USER}:${GW_PASS}@gw/v1' } }\n",
        ),
        base(&["PATH"]),
    );
    assert_eq!(
        url,
        "tools.builtin.env.COPY (line 7) is refused: its value holds the one \
         `telemetry.otlp.endpoint` has, and no process inherits an own credential; to give bash \
         the value knowingly, list COPY under `tools.builtin.credentials`"
    );
    assert!(!url.contains(GW_PASSWORD));
}

/// An `env` entry whose filled value holds a given value is refused by the
/// `credentials` entry whose value it holds and its line, with nothing of
/// the entry shown, and with the fix where one set variable gave it whole.
/// A value no credential holds passes, and one under the cut's floor isn't
/// matched.
#[test]
fn an_env_entry_whose_value_holds_a_given_value_is_refused_and_one_holding_none_passes() {
    let scratch = lablet_test_support::Scratch::new("prepare-env-given-value");
    let credentials = "    credentials: { GH: '${GITHUB_TOKEN}' }\n";

    for entry in ["'${GH_COPY}'", "'${GH_COPY:-}'"] {
        let given = refused(
            &with_env_value(&scratch, "ANTHROPIC_API_KEY", entry, credentials),
            base(&["PATH"]),
        );
        assert_eq!(
            given,
            "tools.builtin.env.COPY (line 7) is refused: its value holds the one \
             `tools.builtin.credentials.GH` has (line 8); list COPY there instead, as COPY: \
             ${GH_COPY}",
            "{entry}"
        );
    }
    let literal = refused(
        &with_env_value(
            &scratch,
            "ANTHROPIC_API_KEY",
            &format!("'{TOKEN}'"),
            credentials,
        ),
        base(&["PATH"]),
    );
    assert_eq!(
        literal,
        "tools.builtin.env.COPY (line 7) is refused: its value holds the one \
         `tools.builtin.credentials.GH` has (line 8); list COPY there instead"
    );
    assert!(!literal.contains(TOKEN));
    // A default that holds the token came from no variable, so the fix
    // names none.
    let default = refused(
        &with_env_value(
            &scratch,
            "ANTHROPIC_API_KEY",
            &format!("'${{NOT_SET:-{TOKEN}}}'"),
            credentials,
        ),
        base(&["PATH"]),
    );
    assert_eq!(default, literal);
    // A `${VAR}` or two defaults that split the token gave none of it, so
    // the fix names none of them.
    for split in [
        "'ghp_0123456789${E_UNSET:-}abcdef'",
        "'${E_UNSET:-ghp_0123456789}${F_UNSET:-abcdef}'",
    ] {
        let refusal = refused(
            &with_env_value(&scratch, "ANTHROPIC_API_KEY", split, credentials),
            base(&["PATH"]),
        );
        assert_eq!(refusal, literal, "{split}");
    }

    passes(
        &with_env_value(&scratch, "ANTHROPIC_API_KEY", "'${SOMETHING_ELSE}'", ""),
        base(&["PATH"]),
    );
    passes(
        &with_env_value(&scratch, "SHORT", "'${SHORT_COPY}'", ""),
        base(&["PATH"]),
    );
}

/// R38 and R40 show nothing of the entry they refuse: what's written
/// beside the `${VAR}` it names, or in its default, may be the credential
/// itself, whether the variable is own, given or one `bash` inherits.
#[test]
fn an_env_entry_refused_by_the_variable_it_names_shows_nothing_of_its_text() {
    let scratch = lablet_test_support::Scratch::new("prepare-env-variable-unshown");
    let with = |env: &str, more: &str| with_env_value(&scratch, "ANTHROPIC_API_KEY", env, more);
    let unshown = |refusal: String| {
        assert!(!refusal.contains("0123456789"), "{refusal}");
        refusal
    };

    for entry in [
        format!("'${{ANTHROPIC_API_KEY:-{KEY}}}'"),
        format!("'${{ANTHROPIC_API_KEY}}{KEY}'"),
        "'sk-ant-0123456789${ANTHROPIC_API_KEY:-}abcdef0123456789'".to_owned(),
    ] {
        assert_eq!(
            unshown(refused(&with(&entry, ""), base(&["PATH"]))),
            "tools.builtin.env.COPY (line 7) is refused: `ANTHROPIC_API_KEY` fills \
             `model.api_key`, and no process inherits an own credential; to give bash the key \
             knowingly, list COPY under `tools.builtin.credentials`",
            "{entry}"
        );
    }
    assert_eq!(
        unshown(refused(
            &with(
                &format!("'${{GITHUB_TOKEN:-{TOKEN}}}'"),
                "    credentials: { GH: '${GITHUB_TOKEN}' }\n"
            ),
            base(&["PATH"])
        )),
        "tools.builtin.env.COPY (line 7) is refused: `GITHUB_TOKEN` is given under `credentials` \
         (line 8); list COPY there instead, as COPY: ${GITHUB_TOKEN}"
    );
    assert_eq!(
        unshown(refused(
            &with(
                &format!("'Bearer ${{OTLP_TOKEN:-{OTLP_TOKEN}}}'"),
                "telemetry: { otlp: { headers: { Authorization: 'Bearer ${OTLP_TOKEN}' } } }\n"
            ),
            base(&["PATH"])
        )),
        "tools.builtin.env.COPY (line 7) is refused: `OTLP_TOKEN` fills \
         `telemetry.otlp.headers.Authorization`, and no process inherits an own credential; to \
         give bash the value knowingly, list COPY under `tools.builtin.credentials`"
    );
    let inherited = fake(
        &scratch,
        &format!(
            "    enabled: [bash]\n    env: {{ GITHUB_TOKEN: '${{GITHUB_TOKEN:-{TOKEN}}}' }}\n"
        ),
        &[("GITHUB_TOKEN", TOKEN)],
    );
    assert_eq!(
        unshown(refused(&inherited, base(&["PATH", "GITHUB_TOKEN"]))),
        "tools.builtin.env.GITHUB_TOKEN (line 7) is refused: bash inherits `GITHUB_TOKEN` as set \
         already, uncut; remove the line, or move it under `tools.builtin.credentials` to have it \
         cut"
    );
}

/// R50: `bash` needs an environment handed in, and the file tools don't.
#[test]
fn bash_is_refused_without_an_environment_handed_in_and_the_file_tools_are_not() {
    let scratch = lablet_test_support::Scratch::new("prepare-base");

    assert_eq!(
        refused(
            &fake(&scratch, "    enabled: [read_file, bash]\n", &[]),
            None
        ),
        "tools.builtin.enabled[0] (line 6): \"bash\" is refused: no process environment was \
         handed in; hand in what commands need, PATH among them, as \
         .with_process_environment([(\"PATH\", \"/usr/bin:/bin\")])"
    );
    passes(&fake(&scratch, "    enabled: [read_file]\n", &[]), None);
    passes(
        &fake(&scratch, "    enabled: [bash]\n", &[]),
        Some(Vec::new()),
    );
}

/// The lines the diagnostic log got while `read` ran.
fn warned(read: impl FnOnce()) -> Vec<String> {
    let written = Arc::new(Mutex::new(Vec::<u8>::new()));
    let to = Arc::clone(&written);
    let subscriber = tracing_subscriber::fmt()
        .with_writer(move || Written(Arc::clone(&to)))
        .without_time()
        .with_target(false)
        .with_level(false)
        .with_ansi(false)
        .finish();
    tracing::subscriber::with_default(subscriber, read);
    String::from_utf8(written.lock().unwrap().clone())
        .unwrap()
        .lines()
        .map(str::to_owned)
        .collect()
}

#[derive(Clone)]
struct Written(Arc<Mutex<Vec<u8>>>);

impl std::io::Write for Written {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// W66: an own credential listed under `credentials` passes, with a warning
/// that `bash` has it.
#[test]
fn an_own_credential_given_under_credentials_passes_with_a_warning() {
    let scratch = lablet_test_support::Scratch::new("prepare-q5");
    scratch.create_dir("work");
    let script = scratch.write(
        "script.yaml",
        "- response: { content: [{ text: Done. }], finish: end_turn }\n",
    );
    let config = filled(
        &format!(
            "model: {{ provider: fake, script: '{}', api_key: '${{ANTHROPIC_API_KEY}}' }}\nprompt: {{ system: Hi. }}\ntools:\n  builtin:\n    root: '{}'\n    enabled: [bash]\n    credentials: {{ ANTHROPIC_API_KEY: '${{ANTHROPIC_API_KEY}}' }}\n",
            script.display(),
            scratch.at("work").display()
        ),
        &[("ANTHROPIC_API_KEY", KEY)],
    );

    let warnings = warned(|| {
        prepare(&config, base(&["PATH"])).map(drop).unwrap();
    });

    assert_eq!(
        warnings,
        [
            "tools.builtin.credentials.ANTHROPIC_API_KEY (line 7): it's lablet's own key, given \
             to bash; a task that needs the key gets it only when the config says so"
        ]
    );
    assert!(!warnings[0].contains(KEY));
}

/// `Checked` names what `tools.builtin.env` passes, a renamed copy among
/// them, and attributes each copy the drop by value took to its credential.
#[test]
fn checked_names_the_env_entries_and_the_copies_of_a_credential() {
    let scratch = lablet_test_support::Scratch::new("prepare-checked");
    let config = fake(
        &scratch,
        "    enabled: [bash]\n    env: { PATH: /usr/bin, GH: '${GITHUB_TOKEN}' }\n",
        &[("GITHUB_TOKEN", "ghp_0123456789abcdef")],
    );
    let mut config = config;
    config.model.api_key = Some(Secret::new(KEY));
    let secrets = secrets::derived(&config, None);
    let environment = Environment {
        inherited: vec!["HOME".to_owned()],
        by_name: BTreeSet::new(),
        by_value: ["MY_COPY".to_owned()].into(),
        copies: vec![(KeyPath::of("model.api_key"), "MY_COPY".to_owned())],
        context: vec!["TRACEPARENT".to_owned()],
    };

    let checked = Checked::new(&config, Vec::new(), secrets, Some(environment));

    assert_eq!(
        checked.own()[0].as_own(),
        "model.api_key (set in code), or MY_COPY, which holds it; no process inherits it"
    );
    let env: Vec<String> = checked.env().iter().map(ToString::to_string).collect();
    assert_eq!(
        env,
        [
            "GH (line 7) reaches bash uncut; if it's a credential, move it under \
             tools.builtin.credentials",
            "PATH (line 7) -> bash",
        ]
    );
    assert_eq!(checked.inherited(), ["HOME"]);
    assert_eq!(checked.digest(), config.digest());
    assert!(!format!("{checked:?}").contains(KEY));
}

/// R62: a header value with no `${VAR}` in it is refused without being
/// shown, in the map form by its name and in the text form by its pair, and
/// a text beside a `${VAR}` or a text that is one `${VAR}` passes. A header
/// set in code passes as a credential set in code does.
#[test]
fn a_header_value_with_no_variable_is_refused_and_never_shown() {
    let scratch = lablet_test_support::Scratch::new("prepare-headers");
    let token = "literal-0123456789abcdef";
    let held = [
        ("T", "tok-0123456789abcdef"),
        ("H", "authorization=Bearer%20x"),
    ];
    let with = |telemetry: &str| {
        filled(
            &format!(
                "model: {{ provider: fake, script: '{}' }}\nprompt: {{ system: Hi. }}\ntelemetry:\n  otlp:\n    headers: {telemetry}\n",
                scratch
                    .write(
                        "script.yaml",
                        "- response: { content: [{ text: Done. }], finish: end_turn }\n"
                    )
                    .display()
            ),
            &held,
        )
    };

    let literal = refused(
        &with(&format!(
            "{{ Authorization: 'Bearer ${{T}}', X-Literal: {token} }}"
        )),
        None,
    );
    assert_eq!(
        literal,
        "telemetry.otlp.headers.X-Literal (line 5) is refused: it holds no ${VAR}, and what it \
         holds isn't shown; write X-Literal: ${X_LITERAL}, and rotate it if this was a token"
    );
    assert!(!literal.contains(token));
    let pair = refused(
        &with(&format!("'authorization=Bearer%20${{T}},x-tenant={token}'")),
        None,
    );
    assert_eq!(
        pair,
        "telemetry.otlp.headers (line 5) is refused: its pair `x-tenant` holds no ${VAR}, and what \
         it holds isn't shown; write x-tenant=${X_TENANT}, and rotate it if this was a token"
    );
    assert!(!pair.contains(token));
    let text = refused(&with(&format!("'authorization=Bearer%20{token}'")), None);
    assert_eq!(
        text,
        "telemetry.otlp.headers (line 5) is refused: it holds no ${VAR}, and what it holds isn't \
         shown; write each value as a ${VAR}, as authorization=Bearer%20${TOKEN}, and rotate it \
         if this was a token"
    );
    assert_eq!(
        refused(&with("'${H:-x=default}'"), None),
        "telemetry.otlp.headers (line 5) is refused: a credential's default can only be empty, \
         and what follows :- isn't shown; write ${H:-}"
    );

    passes(&with("{ Authorization: 'Bearer ${T}' }"), None);
    passes(&with("'${H}'"), None);
    passes(&with("'${H:-}'"), None);
    passes(&with("'authorization=Bearer%20${T},x-tenant=${T}'"), None);
    passes(&with("{}"), None);
    let mut in_code = with("null");
    in_code.telemetry.otlp.headers = Some([("X-Code", Secret::new(token))].into());
    passes(&in_code, None);
}

/// R51: a relative path in a config that no directory was given for is
/// refused by its key, and `Config::relative_to` places it.
#[test]
fn a_relative_path_is_refused_until_the_config_is_given_a_directory() {
    let scratch = lablet_test_support::Scratch::new("prepare-paths");
    scratch.write(
        "script.yaml",
        "- response: { content: [{ text: Done. }], finish: end_turn }\n",
    );
    scratch.write("prompts/sys.md", "Hi.");
    let text = format!(
        "model: {{ provider: fake, script: '{}' }}\nprompt: {{ system_file: prompts/sys.md }}\n",
        scratch.at("script.yaml").display()
    );

    assert_eq!(
        refused(&filled(&text, &[]), None),
        "prompt.system_file (line 2): \"prompts/sys.md\" is refused: a config made in code or from \
         text has no directory; make the path absolute, or call .relative_to(dir)"
    );
    passes(&filled(&text, &[]).relative_to(scratch.path()), None);

    let path = scratch.write("lablet.yaml", &text);
    let from_file = lablet_config::Config::from_path(&path).unwrap();
    assert_eq!(
        refused(&from_file, None),
        "prompt.system_file (line 2): \"prompts/sys.md\" is refused: a library resolves no \
         relative path; make the path absolute, or call .relative_to(dir) with the file's \
         directory"
    );
    let mut in_code = Config::default();
    in_code.model.provider = Provider::Fake;
    in_code.model.script = Some("run.yaml".into());
    in_code.prompt.system = Some("Hi.".to_owned());
    assert_eq!(
        refused(&in_code, None),
        "model.script: \"run.yaml\" is refused: a config made in code or from text has no \
         directory; make the path absolute, or call .relative_to(dir)"
    );
    let dash = filled(
        &format!(
            "model: {{ provider: fake, script: '{}' }}\nprompt: {{ system: Hi. }}\ntelemetry: {{ file: {{ path: '-' }} }}\n",
            scratch.at("script.yaml").display()
        ),
        &[],
    );
    passes(&dash, None);
}

/// A gateway's password, which every test of a URL below writes out or
/// fills, and none may show.
const PASSWORD: &str = "gwsecretpassword1234567";

/// A config of the OpenAI provider with `url` at `key`, one of
/// `model.base_url`, `telemetry.otlp.endpoint` and, for anything else, an
/// HTTP server's `url`.
fn url_at(key: &str, url: &str) -> String {
    let model = "model: { provider: openai, api_key: '${K}' }\nprompt: { system: Hi. }";
    match key {
        "model.base_url" => format!(
            "model: {{ provider: openai, api_key: '${{K}}', base_url: '{url}' }}\nprompt: {{ system: Hi. }}"
        ),
        "telemetry.otlp.endpoint" => {
            format!("{model}\ntelemetry: {{ otlp: {{ endpoint: '{url}' }} }}")
        }
        _ => format!("{model}\ntools: {{ mcp: [{{ name: s, transport: http, url: '{url}' }}] }}"),
    }
}

/// R17's URL form and R63, one rule over the URL as the text wrote it: at
/// `model.base_url`, `telemetry.otlp.endpoint` and an HTTP server's `url`,
/// user information written out, in a default among it, is refused
/// unshown, by its line; one a variable gives from outside the written
/// text is refused by the variable's name; one made of `${VAR}`s alone
/// passes.
#[test]
fn a_url_s_user_or_password_written_out_is_refused_unshown_by_its_line() {
    let held = [
        ("U", "gwuser"),
        ("P", PASSWORD),
        ("GW", "https://gwuser:gwsecretpassword1234567@gw.example/v1"),
        ("K", KEY),
    ];
    // A default is the text written whether or not its variable is set.
    let without_gw: Vec<(&str, &str)> = held
        .iter()
        .copied()
        .filter(|(name, _)| *name != "GW")
        .collect();
    for (key, line) in [
        ("model.base_url", 1),
        ("telemetry.otlp.endpoint", 3),
        ("tools.mcp[0].url", 3),
    ] {
        let written_out = format!(
            "{key} (line {line}) is refused: its user or password isn't a ${{VAR}}, and what it \
             holds isn't shown; write https://${{GW_USER}}:${{GW_PASS}}@gw/v1, and rotate it if \
             this was one"
        );
        for url in [
            format!("https://gwuser:{PASSWORD}@gw.example/v1"),
            format!("https://${{U}}:{PASSWORD}@gw.example/v1"),
            format!("${{GW:-https://d:{PASSWORD}@gw.example/v1}}"),
        ] {
            for set in [&held[..], &without_gw] {
                let refusal = refused(&filled(&url_at(key, &url), set), None);
                assert_eq!(refusal, written_out, "{url}");
                assert!(!refusal.contains(PASSWORD), "{refusal}");
            }
        }
        for url in [
            "https://${U}@gw.example/v1",
            "https://${U}:${P}@gw.example/v1",
        ] {
            // A server goes on to the refusal of what this lablet has no
            // adapter for, past every rule of the URL.
            match prepare(&filled(&url_at(key, url), &held), None) {
                Ok(_) | Err(BuildError::Unsupported { .. }) => {}
                Err(error) => panic!("{url}: {error}"),
            }
        }
        let whole = refused(&filled(&url_at(key, "${GW}"), &held), None);
        assert_eq!(
            whole,
            format!(
                "{key} (line {line}) is refused: `GW` holds a user or password, which isn't \
                 shown; write each as its own ${{VAR}}, as https://${{GW_USER}}:${{GW_PASS}}@gw/v1"
            )
        );
        assert!(!whole.contains(PASSWORD), "{whole}");
    }
}

/// R17's URL form for an override, whose fix is the override's own, with a
/// server named by its number; and a URL set in code, after the text wrote
/// none or `null`, passes, since a host has no other way to give it.
#[test]
fn a_url_s_password_an_override_writes_is_refused_and_one_set_in_code_passes() {
    let mut by_override = RawConfig::from_str(
        &url_at("model.base_url", "https://gw.example/v1"),
        Format::Yaml,
    )
    .unwrap();
    by_override
        .set(&format!(
            "model.base_url=https://gwuser:{PASSWORD}@gw.example/v1"
        ))
        .unwrap();
    let mut server =
        RawConfig::from_str(&url_at("tools.mcp", "https://gw.example/v1"), Format::Yaml).unwrap();
    server
        .set(&format!(
            "tools.mcp.0.url=https://gwuser:{PASSWORD}@gw.example/v1"
        ))
        .unwrap();
    for (raw, key) in [(by_override, "model.base_url"), (server, "tools.mcp.0.url")] {
        let config = raw
            .substituted(|name: &str| (name == "K").then(|| OsString::from(KEY)))
            .unwrap()
            .config()
            .unwrap();
        let refusal = refused(&config, None);
        assert_eq!(
            refusal,
            format!(
                "{} (an override) is refused: its user or password isn't a ${{VAR}}, and what it \
                 holds isn't shown; write --set '{key}=https://${{GW_USER}}:${{GW_PASS}}@gw/v1', \
                 and rotate it if this was one",
                key.replace(".0.", "[0].")
            )
        );
        assert!(!refusal.contains(PASSWORD), "{refusal}");
    }

    let in_code = format!("https://gwuser:{PASSWORD}@gw.example/v1");
    for written in ["", ", base_url: null"] {
        let mut config = filled(
            &format!(
                "model: {{ provider: openai, api_key: '${{K}}'{written} }}\nprompt: {{ system: Hi. }}\ntelemetry: {{ otlp: {{ endpoint: null }} }}"
            ),
            &[("K", KEY)],
        );
        config.model.base_url = Some(in_code.clone());
        config.telemetry.otlp.endpoint = Some(in_code.clone());
        passes(&config, None);
    }
}

/// Q6: an own or given `${VAR}` fills no setting but its own field, a URL's
/// user information or a `credentials` entry; anywhere else its value would
/// reach the transcript, telemetry and the digest as it is. The refusal
/// shows the text as written, and nothing the variable holds.
#[test]
fn a_credential_s_variable_written_outside_its_fields_is_refused() {
    let scratch = lablet_test_support::Scratch::new("prepare-q6");
    let held = [
        ("K", KEY),
        ("G", "ghp_0123456789abcdef"),
        ("U", "gwuser"),
        ("P", PASSWORD),
    ];
    let with = |model: &str, rest: &str| {
        filled(
            &format!(
                "model: {{ provider: fake, script: '{}', api_key: '${{K}}'{model} }}\nprompt: {{ system: Hi. }}\ntools:\n  builtin:\n    root: '{}'\n    enabled: [bash]\n    credentials: {{ GH: '${{G}}' }}\n{rest}",
                scratch
                    .write(
                        "script.yaml",
                        "- response: { content: [{ text: Done. }], finish: end_turn }\n"
                    )
                    .display(),
                scratch.at("work").display()
            ),
            &held,
        )
    };
    scratch.create_dir("work");

    let name = refused(&with(", name: 'm-${K}'", ""), base(&["PATH"]));
    assert_eq!(
        name,
        "model.name (line 1): \"m-${K}\" is refused: `K` fills `model.api_key`, and a \
         credential's variable fills no other setting; use another variable here"
    );
    let resource = refused(
        &with("", "telemetry: { resource: 'service.name=svc,tok=${G}' }\n"),
        base(&["PATH"]),
    );
    assert_eq!(
        resource,
        "telemetry.resource (line 8): \"service.name=svc,tok=${G}\" is refused: `G` is given \
         under `credentials` (line 7), and a credential's variable fills no other setting; use \
         another variable here"
    );
    let host = refused(
        &with(
            "",
            "telemetry: { otlp: { endpoint: 'http://${U}:${P}@${K}.example:4318' } }\n",
        ),
        base(&["PATH"]),
    );
    assert!(
        host.starts_with(
            "telemetry.otlp.endpoint (line 8): its value is refused: `K` fills `model.api_key`"
        ),
        "{host}"
    );
    // A stdio server's `env` is a setting like any other, though no server
    // starts before its adapter is built.
    let server = refused(
        &with(
            "",
            "  mcp: [{ name: s, transport: stdio, command: srv, env: { TOKEN: '${G}' } }]\n",
        ),
        base(&["PATH"]),
    );
    assert_eq!(
        server,
        "tools.mcp[0].env.TOKEN (line 8): \"${G}\" is refused: `G` is given under `credentials` \
         (line 7), and a credential's variable fills no other setting; use another variable here"
    );
    for refusal in [&name, &resource, &host, &server] {
        assert!(
            !refusal.contains(KEY) && !refusal.contains("ghp_"),
            "{refusal}"
        );
    }

    // A URL's user information, a header and a `credentials` entry are the
    // fields a credential's variable fills, Q5's own key given among them.
    passes(
        &with(
            "",
            "    env: { PORT: '8080' }\ntelemetry: { otlp: { endpoint: 'http://${U}:${P}@127.0.0.1:9', headers: { authorization: 'Bearer ${K}' } } }\n",
        ),
        base(&["PATH"]),
    );
}

/// A URL whose written user information is `${VAR}`s, filled with a user
/// or password holding an unencoded `/`, `\`, `?`, `#` or `@`, doesn't read
/// as the text wrote it, so nothing would be own, cut or left out of what
/// `check` shows: it's refused unshown, by its key, naming the variable.
#[test]
fn a_url_whose_filled_user_information_does_not_parse_is_refused_unshown() {
    for (key, line) in [
        ("model.base_url", 1),
        ("telemetry.otlp.endpoint", 3),
        ("tools.mcp[0].url", 3),
    ] {
        let expected = |variable: &str| {
            format!(
                "{key} (line {line}) is refused: its user or password `{variable}` holds `/`, \
                 `?`, `#`, `\\` or `@`, which a URL can't read, and isn't shown; percent-encode it"
            )
        };
        for (user, password, variable) in [
            ("gwuser", "pa/ss-gwsecretpassword", "P"),
            ("gwuser", "pa?ss-gwsecretpassword", "P"),
            ("gwuser", "pa#ss-gwsecretpassword", "P"),
            ("gwuser", "pa@ss-gwsecretpassword", "P"),
            ("gwuser", "pa\\ss-gwsecretpassword", "P"),
            ("gw@user/x", "gwsecretpassword1234567", "U"),
            ("CORP\\gwuser", "gwsecretpassword1234567", "U"),
        ] {
            let held = [("U", user), ("P", password), ("K", KEY)];
            let refusal = refused(
                &filled(&url_at(key, "https://${U}:${P}@gw.example/v1"), &held),
                None,
            );
            assert_eq!(refusal, expected(variable), "{user}:{password}");
            assert!(!refusal.contains("gwsecret"), "{refusal}");
        }
        let encoded = [
            ("U", "gwuser"),
            ("P", "pa%2Fss-gwsecretpassword"),
            ("K", KEY),
        ];
        match prepare(
            &filled(&url_at(key, "https://${U}:${P}@gw.example/v1"), &encoded),
            None,
        ) {
            Ok(_) | Err(BuildError::Unsupported { .. }) => {}
            Err(error) => panic!("{key}: {error}"),
        }
        // Empty defaults leave `:@`, user information that parses.
        match prepare(
            &filled(
                &url_at(key, "https://${U:-}:${P:-}@gw.example/v1"),
                &[("K", KEY)],
            ),
            None,
        ) {
            Ok(_) | Err(BuildError::Unsupported { .. }) => {}
            Err(error) => panic!("{key}: {error}"),
        }
    }
}

/// W66 by value: a `credentials` entry filled from another variable that
/// holds an own credential's value gives `bash` that value as surely as
/// the same variable would, so it's warned of and `own:` says who gets it.
#[test]
fn a_copy_of_an_own_credential_given_to_bash_is_warned_of_and_named() {
    let scratch = lablet_test_support::Scratch::new("prepare-q5-copy");
    scratch.create_dir("work");
    let script = scratch.write(
        "script.yaml",
        "- response: { content: [{ text: Done. }], finish: end_turn }\n",
    );
    let config = filled(
        &format!(
            "model: {{ provider: fake, script: '{}', api_key: '${{K}}' }}\nprompt: {{ system: Hi. }}\ntools:\n  builtin:\n    root: '{}'\n    enabled: [bash]\n    credentials: {{ ALSO: '${{KEYCOPY}}' }}\n",
            script.display(),
            scratch.at("work").display()
        ),
        &[("K", KEY), ("KEYCOPY", KEY)],
    );

    let warnings = warned(|| {
        prepare(&config, base(&["PATH"])).map(drop).unwrap();
    });
    let checked = Checked::new(&config, Vec::new(), secrets::derived(&config, None), None);

    assert_eq!(
        warnings,
        [
            "tools.builtin.credentials.ALSO (line 7): it's lablet's own key, given to bash; a \
             task that needs the key gets it only when the config says so"
        ]
    );
    assert_eq!(
        checked.own()[0].as_own(),
        "K (line 1) -> model.api_key; given to bash by credentials (line 7) as ALSO"
    );
}

/// R62's fix is the form the header was written in: an override's is an
/// override, and a pair's name holding a `${VAR}` suggests a variable named
/// from the rest of it.
#[test]
fn a_header_s_fix_is_written_as_the_header_was() {
    let mut raw = RawConfig::from_str(
        "model: { provider: openai, api_key: '${K}' }\nprompt: { system: Hi. }",
        Format::Yaml,
    )
    .unwrap();
    raw.set("telemetry.otlp.headers.a=literalsecretvalue1234")
        .unwrap();
    let config = raw
        .substituted(|name: &str| (name == "K").then(|| OsString::from(KEY)))
        .unwrap()
        .config()
        .unwrap();
    let refusal = refused(&config, None);
    assert_eq!(
        refusal,
        "telemetry.otlp.headers.a (an override) is refused: it holds no ${VAR}, and what it holds \
         isn't shown; write --set 'telemetry.otlp.headers.a=${A}', and rotate it if this was a \
         token"
    );

    let text = filled(
        "model: { provider: openai, api_key: '${K}' }\nprompt: { system: Hi. }\ntelemetry: { otlp: { headers: 'x-${USR}=literalsecretvalue1234,y=${Y}' } }",
        &[("USR", "u"), ("Y", "y-0123456789abcdef"), ("K", KEY)],
    );
    let refusal = refused(&text, None);
    assert!(
        refusal.contains("write x-${USR}=${X}, and rotate"),
        "{refusal}"
    );
    assert!(!refusal.contains("literalsecret"), "{refusal}");

    // A name that's a `${VAR}` alone leaves nothing to name a variable
    // from.
    let named = filled(
        "model: { provider: openai, api_key: '${K}' }\nprompt: { system: Hi. }\ntelemetry: { otlp: { headers: '${N}=literalsecretvalue1234,y=${Y}' } }",
        &[("N", "x-n"), ("Y", "y-0123456789abcdef"), ("K", KEY)],
    );
    let refusal = refused(&named, None);
    assert!(
        refusal.contains("write ${N}=${HEADER_VALUE}, and rotate"),
        "{refusal}"
    );

    // An override of the text replaces it whole, so its fix keeps the
    // other pairs, each value as a `${VAR}`.
    let mut raw = RawConfig::from_str(
        "model: { provider: openai, api_key: '${K}' }\nprompt: { system: Hi. }",
        Format::Yaml,
    )
    .unwrap();
    raw.set("telemetry.otlp.headers=a=literalsecretvalue1234,b=Bearer ${B},c=otherliteral99,junk")
        .unwrap();
    let config = raw
        .substituted(|name: &str| match name {
            "K" => Some(OsString::from(KEY)),
            "B" => Some(OsString::from("b-0123456789abcdef")),
            _ => None,
        })
        .unwrap()
        .config()
        .unwrap();
    let refusal = refused(&config, None);
    assert_eq!(
        refusal,
        "telemetry.otlp.headers (an override) is refused: its pair `a` holds no ${VAR}, and what \
         it holds isn't shown; write --set 'telemetry.otlp.headers=a=${A},b=Bearer ${B},c=${C}', \
         and rotate it if this was a token"
    );
    for literal in ["literalsecret", "otherliteral", "junk", "b-0123"] {
        assert!(!refusal.contains(literal), "{refusal}");
    }
}

/// A URL as written, the variables that fill it, and the refusal expected.
type Case<'a> = (&'a str, &'a [(&'a str, &'a str)], String);

/// A URL's `${VAR}` that gives it an `@` outside the user information the
/// text wrote may be a user or password the URL can't read, or reads where
/// the text wrote none: a whole-URL variable whose password holds `/`, a
/// host after the written `@` and a path. Each is refused naming the
/// variable and nothing it holds; the plain forms pass, their user and
/// password own, and a password is no part of the digest.
#[test]
fn a_url_variable_that_gives_an_at_the_text_did_not_write_is_refused_unshown() {
    const SECRET: &str = "pa/ss-abcdefghijklmnopq";
    for (key, line) in [
        ("model.base_url", 1),
        ("telemetry.otlp.endpoint", 3),
        ("tools.mcp[0].url", 3),
    ] {
        let at = |variable: &str| {
            format!(
                "{key} (line {line}) is refused: `{variable}` holds an `@`, which may be a user or \
                 password a URL can't read, and isn't shown; percent-encode it, or write an `@` \
                 in a path as %40"
            )
        };
        let whole = format!("http://user-abcdefghijklmnop:{SECRET}@127.0.0.1:9");
        let cases: [Case<'_>; 4] = [
            ("${URL}", &[("URL", &whole)], at("URL")),
            (
                "http://${USR}:${PW}@${H}:9",
                &[
                    ("USR", "user-abcdefghijklmnop"),
                    ("PW", "xyzabcdefghijklmnopq@evil.example/"),
                    ("H", "127.0.0.1"),
                ],
                format!(
                    "{key} (line {line}) is refused: its user or password `PW` holds `/`, `?`, \
                     `#`, `\\` or `@`, which a URL can't read, and isn't shown; percent-encode it"
                ),
            ),
            (
                "http://${USR}:${PW}@${H}:9",
                &[
                    ("USR", "user-abcdefghijklmnop"),
                    ("PW", "xyzabcdefghijklmnopq"),
                    ("H", "xyzabcdefghijklmnopq@127.0.0.1"),
                ],
                at("H"),
            ),
            (
                "http://127.0.0.1:9/v1/${PATH_PART}",
                &[("PATH_PART", "xyzabcdefghijklmnopq@x")],
                at("PATH_PART"),
            ),
        ];
        for (url, variables, expected) in cases {
            let mut held = variables.to_vec();
            held.push(("K", KEY));
            let refusal = refused(&filled(&url_at(key, url), &held), None);
            assert_eq!(refusal, expected, "{url}");
            for (_, value) in variables {
                assert!(!refusal.contains(value), "{refusal}");
            }
            assert!(!refusal.contains("abcdefghijklmnop"), "{refusal}");
        }

        for (url, held) in [
            ("${URL}", &[("URL", "http://127.0.0.1:9/v1")][..]),
            (
                "http://127.0.0.1:9/v1/${PATH_PART}",
                &[("PATH_PART", "a%40b")][..],
            ),
            (
                "http://${USR}:${PW}@${H}:9",
                &[("USR", "u"), ("PW", "p%2F%40"), ("H", "127.0.0.1")][..],
            ),
        ] {
            let mut held = held.to_vec();
            held.push(("K", KEY));
            match prepare(&filled(&url_at(key, url), &held), None) {
                Ok(_) | Err(BuildError::Unsupported { .. }) => {}
                Err(error) => panic!("{key} {url}: {error}"),
            }
        }
        let plain = |password: &str| {
            filled(
                &url_at(key, "https://${U}:${P}@gw.example/v1"),
                &[("U", "gwuser"), ("P", password), ("K", KEY)],
            )
        };
        let derived = secrets::derived(&plain(PASSWORD), None);
        let own: Vec<&str> = derived
            .own
            .iter()
            .filter(|credential| credential.field.to_string() == key)
            .filter_map(|credential| credential.variable())
            .collect();
        assert_eq!(own, ["U", "P"], "{key}");
        assert_eq!(
            plain(PASSWORD).digest(),
            plain("anothergwpassword7654321").digest(),
            "{key}"
        );
    }
}

/// A variable written in a URL's user information and again in its host,
/// path or query fills more than its own field: what it fills there is
/// shown, so it's refused as a credential's variable outside its fields is.
#[test]
fn a_user_information_variable_written_again_elsewhere_in_its_url_is_refused() {
    for (key, line) in [
        ("model.base_url", 1),
        ("telemetry.otlp.endpoint", 3),
        ("tools.mcp[0].url", 3),
    ] {
        for url in [
            "http://${USR}:${PW}@127.0.0.1:9/v1/${PW}",
            "http://${USR}:${PW}@127.0.0.1:9/v1?token=${PW:-}",
            "http://${USR}:${PW}@${PW}.example:9/v1",
        ] {
            let refusal = refused(
                &filled(
                    &url_at(key, url),
                    &[("USR", "gwuser"), ("PW", PASSWORD), ("K", KEY)],
                ),
                None,
            );
            assert!(
                refusal.starts_with(&format!(
                    "{key} (line {line}): its value is refused: `PW` fills `{key}`'s user or \
                     password, and a credential's variable fills no other setting; use another \
                     variable here"
                )),
                "{url}: {refusal}"
            );
            assert!(!refusal.contains(PASSWORD), "{refusal}");
        }
    }
}

/// An `@` past a URL's authority is one no reading takes as a user or
/// password, so nothing would see it written out or cut it: as the text
/// writes it, out, in a default whose variable is unset, empty or set, or
/// in a whole-URL default, it's refused unshown by its line or as an
/// override, before every other rule of the URL; and a filled URL that
/// holds one the rule of each variable passes, as a scheme's variable
/// holding a `/` gives, is refused naming each of its variables in order.
/// An `@` percent-encoded in a path passes.
#[test]
fn a_url_that_holds_an_at_past_its_authority_is_refused_unshown() {
    const SECRET: &str = "pa/ss-abcdefghijk";
    let fix = "percent-encode a `/`, `?`, `#`, `\\` or `@` in a user or password, or write an `@` \
               in a path as %40";
    for (key, line) in [
        ("model.base_url", 1),
        ("telemetry.otlp.endpoint", 3),
        ("tools.mcp[0].url", 3),
    ] {
        let written = |place: &str| {
            format!(
                "{key} ({place}) is refused: its text holds an `@` a URL can't read as a user or \
                 password, and isn't shown; {fix}; rotate it if this was one"
            )
        };
        let user = ("U", "user-abcdefghijk");
        for url in [
            format!("http://user-abcdefghijk:{SECRET}@127.0.0.1:9"),
            format!("http://${{U}}:${{P:-{SECRET}}}@127.0.0.1:9"),
            format!("${{URL:-http://u:{SECRET}@127.0.0.1:9}}"),
        ] {
            for held in [
                &[user, ("K", KEY)][..],
                &[user, ("P", ""), ("K", KEY)],
                &[
                    user,
                    ("P", "plain"),
                    ("URL", "http://127.0.0.1:9"),
                    ("K", KEY),
                ],
            ] {
                let refusal = refused(&filled(&url_at(key, &url), held), None);
                assert_eq!(refusal, written(&format!("line {line}")), "{url}");
                assert!(!refusal.contains("abcdefghijk"), "{refusal}");
            }
        }
        let mut raw =
            RawConfig::from_str(&url_at(key, "http://127.0.0.1:9"), Format::Yaml).unwrap();
        let set = key.replace('[', ".").replace(']', "");
        raw.set(&format!("{set}=http://u:{SECRET}@127.0.0.1:9"))
            .unwrap();
        let overridden = raw
            .substituted(|name: &str| (name == "K").then(|| OsString::from(KEY)))
            .unwrap()
            .config()
            .unwrap();
        assert_eq!(refused(&overridden, None), written("an override"));

        let scheme = refused(
            &filled(
                &url_at(key, "${S}://${U}:${P}@127.0.0.1:9"),
                &[
                    ("S", "http://x/"),
                    user,
                    ("P", "sekrit-abcdefghijk"),
                    ("K", KEY),
                ],
            ),
            None,
        );
        assert_eq!(
            scheme,
            format!(
                "{key} (line {line}) is refused: `S`, `U` and `P` give it an `@` a URL can't \
                 read as a user or password, and what they hold isn't shown; {fix}"
            )
        );
        assert!(!scheme.contains("abcdefghijk"), "{scheme}");
        let whole = refused(
            &filled(
                &url_at(key, "${URL}"),
                &[
                    ("URL", "http://u:pa/ss-abcdefghijk@127.0.0.1:9"),
                    ("K", KEY),
                ],
            ),
            None,
        );
        assert!(whole.contains("is refused: `URL` holds an `@`"), "{whole}");
        assert!(!whole.contains("abcdefghijk"), "{whole}");

        match prepare(
            &filled(&url_at(key, "http://127.0.0.1:9/v1/a%40b"), &[("K", KEY)]),
            None,
        ) {
            Ok(_) | Err(BuildError::Unsupported { .. }) => {}
            Err(error) => panic!("{key}: {error}"),
        }
    }
}

/// The target `prepare` builds for `config`, an `anthropic` one.
fn target_of(config: &Config) -> Target {
    let wiring = prepare(config, None)
        .unwrap()
        .finish(Telemetry::default())
        .unwrap()
        .wiring()
        .unwrap();
    match wiring.provider {
        Ready::Anthropic(target) => target,
        Ready::Openai | Ready::Fake(_) => panic!("the config selects anthropic"),
    }
}

/// The url crate reads a `\` in an `https` authority as a `/`, so a
/// gateway's user given as `CORP\jdoe` would have made `corp` the host the
/// key went to, with the password in the path. The check reads the `\` as
/// the adapter does: a variable that gives one to the user information is
/// refused by its name and nothing it holds, as a URL set in code that
/// holds one is, and `%5C` reaches the gateway as a user. Both left empty,
/// the user and the password are none to the check and to the target.
#[test]
fn a_gateway_user_holding_a_backslash_is_refused_where_the_adapter_would_read_a_host() {
    let gateway = |url: &str, held: &[(&str, &str)]| {
        let mut held = held.to_vec();
        held.push(("K", KEY));
        filled(
            &format!(
                "model: {{ api_key: '${{K}}', base_url: '{url}' }}\nprompt: {{ system: Hi. }}"
            ),
            &held,
        )
    };
    let url = "https://${GW_USER}:${GW_PASS}@gateway.corp.example/anthropic";

    let refusal = refused(
        &gateway(url, &[("GW_USER", "CORP\\jdoe"), ("GW_PASS", PASSWORD)]),
        None,
    );
    assert_eq!(
        refusal,
        "model.base_url (line 1) is refused: its user or password `GW_USER` holds `/`, `?`, `#`, \
         `\\` or `@`, which a URL can't read, and isn't shown; percent-encode it"
    );
    assert!(
        !refusal.contains("jdoe") && !refusal.contains(PASSWORD),
        "{refusal}"
    );

    let mut in_code = Config::default();
    in_code.prompt.system = Some("Hi.".to_owned());
    in_code.model.api_key = Some(Secret::new(KEY));
    in_code.model.base_url = Some(format!(
        "https://CORP\\jdoe:{PASSWORD}@gateway.corp.example/anthropic"
    ));
    let refusal = refused(&in_code, None);
    assert_eq!(
        refusal,
        "model.base_url is refused: it holds an `@` a URL can't read as a user or password, and \
         isn't shown; percent-encode a `/`, `?`, `#`, `\\` or `@` in a user or password, or write \
         an `@` in a path as %40"
    );
    assert!(!refusal.contains(PASSWORD), "{refusal}");

    let encoded = target_of(&gateway(
        url,
        &[("GW_USER", "CORP%5Cjdoe"), ("GW_PASS", PASSWORD)],
    ));
    assert_eq!(encoded.endpoint().host, "gateway.corp.example");
    assert!(encoded.carries_credentials());
    let empty = target_of(&gateway(
        "https://${GW_USER:-}:${GW_PASS:-}@gateway.corp.example/anthropic",
        &[],
    ));
    assert_eq!(empty.endpoint().host, "gateway.corp.example");
    assert!(!empty.carries_credentials());
}

/// The belt behind the reader: were the config's reading of
/// `model.base_url` and the target's to part, on whether it holds a user
/// or a password or on the host and port a request goes to, the check
/// refuses the URL rather than send through a target it misread. The two
/// read alike once a `\` ends the authority, which the last case holds,
/// but where a `\` follows `://`: the config's reader finds an empty
/// authority there, and the parser skips it as it does a `/` and sends to
/// the host after it. That is the one parting `prepare` meets, so the rest
/// are made here with the target of another URL; the parser's own
/// spellings of a host, a port and an empty user and password part nothing.
#[test]
fn the_config_s_reading_of_the_base_url_and_the_target_s_are_held_alike() {
    for url in ["https://\\gw.example/v1", "http://\\\\gw.example"] {
        let fix = "the URL isn't read as it's written, and isn't shown; write its host straight \
                   after `://`, and percent-encode a `/`, `?`, `#`, `\\` or `@` in a user or \
                   password, or write an `@` in a path as %40";
        let written = filled(
            &format!(
                "model: {{ api_key: '${{K}}', base_url: '{url}' }}\nprompt: {{ system: Hi. }}"
            ),
            &[("K", KEY)],
        );
        assert_eq!(
            refused(&written, None),
            format!("model.base_url (line 1) is refused: {fix}"),
            "{url}"
        );
        let mut in_code = Config::default();
        in_code.prompt.system = Some("Hi.".to_owned());
        in_code.model.api_key = Some(Secret::new(KEY));
        in_code.model.base_url = Some(url.to_owned());
        assert_eq!(
            refused(&in_code, None),
            format!("model.base_url is refused: {fix}"),
            "{url}"
        );
    }

    let target = |url: &str| Target::new("m", Key(KEY), Some(url), true).unwrap();
    for (read, sent) in [
        ("https://u:p@gw.example/v1", "https://gw.example/v1"),
        ("https://gw.example/v1", "https://u:p@gw.example/v1"),
        ("https://u:p@gw.example/v1", "https://u:p@other.example/v1"),
        (
            "https://u:p@gw.example:8443/v1",
            "https://u:p@gw.example/v1",
        ),
    ] {
        assert!(!read_alike(read, &target(sent)), "{read} read as {sent}");
    }
    for url in [
        "https://u:p@gw.example/v1",
        "https://u:p@GW.Example:8443/v1",
        "https://u:p@[::1]:8443/v1",
        "http://127.1:9/v1",
        "https://bücher.example/v1",
        "https://u@gw.example",
        "https://:p@gw.example",
        "https://:@gw.example/v1",
        "https://@gw.example/v1",
        "https://CORP%5Cjdoe:pw@gw.example/v1",
        "https://CORP\\jdoe:pw@gw.example/v1",
    ] {
        assert!(read_alike(url, &target(url)), "{url}");
    }
}

/// A system prompt of whitespace alone would go to the provider as a
/// system block that says nothing, where an empty one is a run with no
/// system prompt. So one that holds only whitespace is refused where the
/// config states it, inline or in the file it names, and an empty one
/// passes, as does a prompt with whitespace around it.
#[test]
fn a_system_prompt_of_whitespace_alone_is_refused_inline_and_in_its_file() {
    let scratch = lablet_test_support::Scratch::new("prepare-blank-system");
    let script = scratch.write(
        "script.yaml",
        "- response: { content: [{ text: Done. }], finish: end_turn }\n",
    );
    let config = |prompt: &str| {
        filled(
            &format!(
                "model: {{ provider: fake, script: '{}' }}\nprompt: {prompt}",
                script.display()
            ),
            &[],
        )
    };
    for blank in ["' '", "\"\\n\\t\""] {
        assert_eq!(
            refused(&config(&format!("{{ system: {blank} }}")), None),
            format!(
                "prompt.system (line 2): {:?} is refused: it holds only whitespace, which would \
                 be sent as a system prompt that says nothing; write a prompt, or state \
                 `prompt.system` empty for none",
                if blank == "' '" { " " } else { "\n\t" }
            )
        );
    }
    let file = scratch.write("system.md", "\n");
    assert_eq!(
        refused(
            &config(&format!("{{ system_file: '{}' }}", file.display())),
            None
        ),
        format!(
            "prompt.system_file (line 2): {:?} is refused: the file holds only whitespace, which \
             would be sent as a system prompt that says nothing; write a prompt in the file, or \
             leave it empty for none",
            file.display().to_string()
        )
    );

    passes(&config("{ system: '' }"), None);
    passes(&config("{ system: ' Hi. ' }"), None);
    let empty = scratch.write("empty.md", "");
    passes(
        &config(&format!("{{ system_file: '{}' }}", empty.display())),
        None,
    );
}
