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
/// is refused where `bash` inherits it already.
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
        "tools.builtin.env.T (line 7): \"${ANTHROPIC_API_KEY}\" is refused: `ANTHROPIC_API_KEY` \
         fills `model.api_key`, and no process inherits an own credential; to give bash the key \
         knowingly, list T under `tools.builtin.credentials`"
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
        "tools.builtin.env.T (line 7): \"Bearer ${OTLP_TOKEN}\" is refused: `OTLP_TOKEN` fills \
         `telemetry.otlp.headers.Authorization`, and no process inherits an own credential; to \
         give bash the value knowingly, list T under `tools.builtin.credentials`"
    );

    let given = with(
        "    enabled: [bash]\n    env: { T: '${GITHUB_TOKEN}' }\n    credentials: { GITHUB_TOKEN: '${GITHUB_TOKEN}' }\n",
    );
    assert_eq!(
        refused(&given, base(&["PATH"])),
        "tools.builtin.env.T (line 7): \"${GITHUB_TOKEN}\" is refused: `GITHUB_TOKEN` is given \
         under `credentials` (line 8); list T there instead, as T: ${GITHUB_TOKEN}"
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
        "tools.builtin.env.GITHUB_TOKEN (line 7): \"${GITHUB_TOKEN}\" is refused: bash inherits \
         `GITHUB_TOKEN` as set already, uncut; remove the line, or move it under \
         `tools.builtin.credentials` to have it cut"
    );
    passes(&same, base(&["PATH"]));
    let port = with("    enabled: [bash]\n    env: { PORT: '${PORT}' }\n");
    passes(&port, base(&["PATH"]));
    let renamed = with("    enabled: [bash]\n    env: { GH: '${GITHUB_TOKEN}' }\n");
    passes(&renamed, base(&["PATH", "GITHUB_TOKEN"]));
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
    let secrets = secrets::derived(&config, None, true);
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
