use lablet::{BuildError, ConfigError, Unsupported};

use super::Refusal;

#[test]
fn a_refused_config_is_a_config_message() {
    let refusal = Refusal::from(ConfigError::Missing {
        key: "model.script".to_owned(),
        reason: "the provider `fake` plays it".to_owned(),
    });
    assert_eq!(
        refusal.to_string(),
        "config: model.script isn't set: the provider `fake` plays it"
    );
}

#[test]
fn a_build_that_failed_is_a_config_message_whatever_failed() {
    let refusals = [
        BuildError::Config(ConfigError::UnknownFormat {
            path: "lablet.toml".to_owned(),
        }),
        BuildError::KeyVariable {
            place: None,
            reason: "`ANTHROPIC_API_KEY` isn't set".to_owned(),
        },
        BuildError::Unsupported {
            kind: Unsupported::Anthropic,
            phase: "7",
        },
        BuildError::Tools {
            reason: "two tools are named bash".to_owned(),
        },
    ];
    for error in refusals {
        let shown = error.to_string();
        assert_eq!(Refusal::from(error).to_string(), format!("config: {shown}"));
    }
}

#[test]
fn what_isnt_built_says_so_with_no_class() {
    assert_eq!(
        Refusal::NotBuilt("lablet schema").to_string(),
        "lablet schema isn't built yet"
    );
}
