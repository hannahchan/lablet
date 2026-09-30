//! The check of the key's variable, and what's withheld of lablet's
//! secrets, against an environment the test states: a test can't set a
//! variable of its own process.

use super::*;

/// What stands for a key that was written where the variable's name
/// belongs. It reads as a name, so only the environment refuses it.
const PASTED: &str = "sk_live_0123456789abcdef";

/// A key as long as a key is, and so one that's cut.
const KEY: &str = "sk-ant-0123456789abcdef0123456789";

fn config(text: &str) -> Config {
    Config::from_str(text, Format::Yaml).unwrap()
}

fn model(text: &str) -> Model {
    config(text).model
}

/// An environment that holds `value` under `name` and nothing else.
fn holding(name: &'static str, value: &'static str) -> impl Fn(&str) -> Option<OsString> {
    move |asked| (asked == name).then(|| value.into())
}

fn nothing(_: &str) -> Option<OsString> {
    None
}

fn refusal(model: &Model, held: impl Fn(&str) -> Option<OsString>) -> String {
    let error = key_is_set(model, held).unwrap_err();
    assert!(matches!(error, BuildError::KeyVariable { .. }), "{error:?}");
    assert!(!format!("{error:?}").contains(PASTED), "{error:?}");
    error.to_string()
}

#[test]
fn a_variable_that_holds_a_key_is_taken() {
    let named = model(&format!("model: {{ api_key_env: {PASTED} }}"));

    assert_eq!(key_is_set(&named, holding(PASTED, "a key")), Ok(()));
    assert_eq!(
        key_is_set(&model(""), holding("ANTHROPIC_API_KEY", "a key")),
        Ok(())
    );
}

#[test]
fn a_variable_that_is_not_set_is_refused_and_the_name_the_config_states_is_not_shown() {
    let named = model(&format!("model: {{ api_key_env: {PASTED} }}"));

    assert_eq!(
        refusal(&named, holding("ANTHROPIC_API_KEY", "a key")),
        "model.api_key_env is refused: the variable it names isn't set, and the provider \
         `anthropic` needs a key"
    );
}

#[test]
fn a_variable_that_is_empty_is_refused_and_the_name_the_config_states_is_not_shown() {
    let named = model(&format!("model: {{ api_key_env: {PASTED} }}"));

    assert_eq!(
        refusal(&named, holding(PASTED, "")),
        "model.api_key_env is refused: the variable it names is empty, and the provider \
         `anthropic` needs a key"
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

        assert_eq!(key_is_set(&named, never), Ok(()), "{provider}");
    }
}

#[test]
fn the_variable_lablet_reads_its_key_from_is_withheld_and_what_it_holds_is_cut() {
    let named = config("model: { provider: fake, script: run.yaml, api_key_env: WORK_KEY }");
    let by_default = config("");

    for (config, variable) in [(named, "WORK_KEY"), (by_default, "ANTHROPIC_API_KEY")] {
        let withheld = withheld(&config, holding(variable, KEY));

        assert_eq!(
            withheld,
            Withheld {
                variables: [variable.to_owned()].into(),
                values: Secrets::new([KEY.to_owned()]),
            }
        );
        assert!(!format!("{withheld:?}").contains(KEY), "{withheld:?}");
    }
}

#[test]
fn a_config_that_names_no_key_withholds_nothing() {
    let never = |_: &str| -> Option<OsString> { panic!("the environment was read") };

    let withheld = withheld(
        &config("model: { provider: fake, script: run.yaml }"),
        never,
    );

    assert_eq!(withheld, Withheld::default());
}

#[test]
fn a_key_variable_that_is_not_set_or_holds_a_short_value_is_withheld_with_nothing_to_cut() {
    let named = config("model: { provider: fake, script: run.yaml, api_key_env: WORK_KEY }");
    let expected = Withheld {
        variables: ["WORK_KEY".to_owned()].into(),
        values: Secrets::default(),
    };

    assert_eq!(withheld(&named, nothing), expected);
    assert_eq!(withheld(&named, holding("WORK_KEY", "short")), expected);
}
