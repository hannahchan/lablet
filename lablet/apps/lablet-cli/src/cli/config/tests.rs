use lablet_test_support::Scratch;

use super::read;

const CONFIG: &str = "run:\n  max_turns: 3\nmodel:\n  provider: fake\n";

fn overrides(set: &[&str]) -> Vec<String> {
    set.iter().map(|&set| set.to_owned()).collect()
}

#[test]
fn a_config_with_no_override_is_the_file_s() {
    let scratch = Scratch::new("config-plain");
    let path = scratch.write("lablet.yaml", CONFIG);

    let config = read(&path, &[]).unwrap();

    assert_eq!(config.run.max_turns, std::num::NonZeroU32::new(3));
    assert_eq!(config.source(), Some(path.as_path()));
}

#[test]
fn each_override_is_stated_over_the_file_and_a_later_one_over_an_earlier() {
    let scratch = Scratch::new("config-overrides");
    let path = scratch.write("lablet.yaml", CONFIG);

    let config = read(
        &path,
        &overrides(&["run.max_turns=5", "model.name=other", "run.max_turns=7"]),
    )
    .unwrap();

    assert_eq!(config.run.max_turns, std::num::NonZeroU32::new(7));
    assert_eq!(config.model.name, "other");
}

#[test]
fn an_override_that_cant_be_stated_is_a_config_message_that_names_its_key() {
    let scratch = Scratch::new("config-override-refused");
    let path = scratch.write("lablet.yaml", CONFIG);

    let refused = read(&path, &overrides(&["run.max_turns.cap=5"])).unwrap_err();

    assert_eq!(
        refused.to_string(),
        "config: run.max_turns.cap: the override is refused: run.max_turns holds a value, and \
         a value has no keys"
    );
}

#[test]
fn an_override_the_config_refuses_is_named_as_set_by_an_override() {
    let scratch = Scratch::new("config-override-invalid");
    let path = scratch.write("lablet.yaml", CONFIG);

    let refused = read(&path, &overrides(&["run.completion=implicit"])).unwrap_err();

    assert!(
        refused
            .to_string()
            .starts_with("config: run.completion (an override): \"implicit\" is refused: "),
        "{refused}"
    );
}

#[test]
fn a_config_that_cant_be_read_is_a_config_message() {
    let scratch = Scratch::new("config-missing");

    let refused = read(&scratch.at("lablet.yaml"), &[]).unwrap_err();

    assert!(
        refused.to_string().starts_with(&format!(
            "config: the config {} couldn't be read: ",
            scratch.at("lablet.yaml").display()
        )),
        "{refused}"
    );
}
