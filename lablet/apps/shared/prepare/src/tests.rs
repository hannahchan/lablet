//! What lablet reads of where it runs, against an environment the test
//! states, since a test can't set a variable of its own process: the check
//! of the key's variable, the names of lablet's secrets, and `${VAR}`.

use std::ffi::OsString;

use super::*;

/// What stands for a key that was written where the variable's name
/// belongs. It reads as a name, so only the environment refuses it.
const PASTED: &str = "sk_live_0123456789abcdef";

fn config(text: &str) -> Config {
    Config::from_str(text, Format::Yaml).unwrap()
}

fn model(text: &str) -> Config {
    config(text)
}

/// Whether the key's variable is set, for the config as it runs and as
/// it's written, which here are one.
fn key_set(
    config: &Config,
    held: impl Fn(&str) -> Option<OsString> + Sync,
) -> Result<(), BuildError> {
    key_is_set(&config.model, config, &held)
}

/// An environment that holds `value` under `name` and nothing else.
fn holding(name: &'static str, value: &'static str) -> impl Fn(&str) -> Option<OsString> {
    move |asked| (asked == name).then(|| value.into())
}

fn nothing(_: &str) -> Option<OsString> {
    None
}

fn refusal(model: &Config, held: impl Fn(&str) -> Option<OsString> + Sync) -> String {
    let error = key_set(model, held).unwrap_err();
    assert!(matches!(error, BuildError::KeyVariable { .. }), "{error:?}");
    assert!(!format!("{error:?}").contains(PASTED), "{error:?}");
    error.to_string()
}

#[test]
fn a_variable_that_holds_a_key_is_taken() {
    let named = model(&format!("model: {{ api_key_env: {PASTED} }}"));

    assert_eq!(key_set(&named, holding(PASTED, "a key")), Ok(()));
    assert_eq!(
        key_set(&model(""), holding("ANTHROPIC_API_KEY", "a key")),
        Ok(())
    );
}

#[test]
fn a_variable_that_is_not_set_is_refused_and_a_name_not_written_in_capitals_is_not_shown() {
    for named in [PASTED, "work_key", "Work_Key", "ANTHROPIC_API_KEy"] {
        let config = model(&format!("model: {{ api_key_env: {named} }}"));

        let shown = refusal(&config, holding("ANTHROPIC_API_KEY", "a key"));

        assert_eq!(
            shown,
            "model.api_key_env (line 1) is refused: the variable it names isn't set, and the provider \
             `anthropic` needs a key",
            "{named}"
        );
        assert!(!shown.contains(named), "{named}: {shown}");
    }
}

#[test]
fn a_variable_that_is_empty_is_refused_and_a_name_not_written_in_capitals_is_not_shown() {
    let named = model(&format!("model: {{ api_key_env: {PASTED} }}"));

    assert_eq!(
        refusal(&named, holding(PASTED, "")),
        "model.api_key_env (line 1) is refused: the variable it names is empty, and the provider \
         `anthropic` needs a key"
    );
}

#[test]
fn a_variable_the_config_names_in_capitals_is_named_when_it_is_not_set_or_empty() {
    for named in ["ANTHROPIC_API_KEY", "WORK_KEY_2", "_K"] {
        let config = model(&format!("model: {{ api_key_env: {named} }}"));

        assert_eq!(
            refusal(&config, nothing),
            format!(
                "model.api_key_env (line 1) is refused: `{named}`, the variable it names, isn't set, \
                 and the provider `anthropic` needs a key"
            )
        );
    }
    let empty = model("model: { api_key_env: WORK_KEY }");
    assert_eq!(
        refusal(&empty, holding("WORK_KEY", "")),
        "model.api_key_env (line 1) is refused: `WORK_KEY`, the variable it names, is empty, and the \
         provider `anthropic` needs a key"
    );
}

#[test]
fn the_variable_that_is_read_when_the_config_names_none_is_shown() {
    assert_eq!(
        refusal(&model(""), nothing),
        "model.api_key_env is refused: `ANTHROPIC_API_KEY`, the variable that's read when \
         the config names none, isn't set, and the provider `anthropic` needs a key"
    );
    assert_eq!(
        refusal(&model(""), holding("ANTHROPIC_API_KEY", "")),
        "model.api_key_env is refused: `ANTHROPIC_API_KEY`, the variable that's read when \
         the config names none, is empty, and the provider `anthropic` needs a key"
    );
}

#[test]
fn a_provider_that_needs_no_key_has_no_variable_read() {
    let never = |_: &str| -> Option<OsString> { panic!("the environment was read") };

    for provider in ["openai", "fake"] {
        let named = model(&format!(
            "model: {{ provider: {provider}, api_key_env: {PASTED} }}"
        ));

        assert_eq!(key_set(&named, never), Ok(()), "{provider}");
    }
}
