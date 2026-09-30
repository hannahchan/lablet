use std::path::{Path, PathBuf};

use lablet::config::Provider;
use lablet::{Config, Format};
use lablet_provider_fake::{Script, ScriptFormat, ScriptSource};
use lablet_test_support::Scratch;

use super::{CONFIG, SCRIPT, Starter, write, written};
use crate::cli::refusal::Refusal;

const STARTERS: [(Starter, Provider); 3] = [
    (Starter::Anthropic, Provider::Anthropic),
    (Starter::Openai, Provider::Openai),
    (Starter::Fake, Provider::Fake),
];

fn config_of(starter: Starter) -> Config {
    let [config, ..] = starter.files() else {
        panic!("{starter:?} has no files");
    };
    assert_eq!(config.name, CONFIG);
    Config::from_str(config.text, Format::Yaml).unwrap()
}

#[test]
fn each_starter_is_a_config_the_library_reads_that_selects_its_provider() {
    for (starter, provider) in STARTERS {
        let config = config_of(starter);
        assert_eq!(config.model.provider, provider, "{starter:?}");
        assert!(
            config
                .prompt
                .system
                .is_some_and(|system| !system.is_empty()),
            "{starter:?} has no system prompt"
        );
    }
}

#[test]
fn the_starters_that_need_a_key_name_its_variable_and_hold_no_key() {
    for (starter, variable) in [
        (Starter::Anthropic, "ANTHROPIC_API_KEY"),
        (Starter::Openai, "OPENAI_API_KEY"),
    ] {
        let config = config_of(starter);
        assert_eq!(config.model.api_key_env.as_deref(), Some(variable));
        assert_eq!(
            starter.files().len(),
            1,
            "{starter:?} writes its config alone"
        );
    }
}

#[test]
fn the_fake_starter_plays_the_script_it_writes_beside_its_config() {
    let config = config_of(Starter::Fake);
    assert_eq!(config.model.script.as_deref(), Some(Path::new(SCRIPT.name)));
    assert_eq!(Starter::Fake.files().get(1), Some(&SCRIPT));
    Script::read(ScriptSource {
        name: SCRIPT.name,
        text: SCRIPT.text,
        format: ScriptFormat::Yaml,
    })
    .unwrap();
}

#[test]
fn init_writes_the_starters_files_into_a_directory_it_makes() {
    let scratch = Scratch::new("init-writes");
    let directory = scratch.at("trials/first");

    let paths = write(Starter::Fake, &directory).unwrap();

    assert_eq!(paths, [directory.join(CONFIG), directory.join(SCRIPT.name)]);
    for (path, file) in paths.iter().zip(Starter::Fake.files()) {
        assert_eq!(std::fs::read_to_string(path).unwrap(), file.text);
    }
}

#[test]
fn init_writes_nothing_when_a_file_it_would_write_is_there() {
    let scratch = Scratch::new("init-refuses");
    let kept = scratch.write(
        SCRIPT.name,
        "- response: { content: [], finish: end_turn }\n",
    );

    let refused = write(Starter::Fake, scratch.path());

    assert_eq!(
        refused,
        Err(Refusal::config(format!(
            "{} is there already, and init writes over nothing",
            kept.display()
        )))
    );
    assert!(!scratch.at(CONFIG).exists(), "the config was written");
    assert_eq!(
        std::fs::read_to_string(&kept).unwrap(),
        "- response: { content: [], finish: end_turn }\n"
    );
}

#[test]
fn init_that_cant_make_the_directory_says_which() {
    let scratch = Scratch::new("init-no-directory");
    let file = scratch.write("taken", "");
    let directory = file.join("configs");

    let Err(refusal) = write(Starter::Anthropic, &directory) else {
        panic!("a directory was made under a file");
    };
    let message = refusal.to_string();
    let named = format!("config: {} can't be made: ", directory.display());
    assert!(message.starts_with(&named), "{message}");
}

#[test]
fn init_says_what_it_wrote_and_how_to_run_it_from_where_its_config_is() {
    let here = [
        PathBuf::from("./lablet.yaml"),
        PathBuf::from("./lablet-script.yaml"),
    ];
    assert_eq!(
        written(&here, Path::new(".")),
        "wrote ./lablet.yaml and ./lablet-script.yaml; \
         run it with: lablet run --config lablet.yaml --prompt \"Say hello.\""
    );
    assert_eq!(
        written(&[PathBuf::from("trial/lablet.yaml")], Path::new("trial")),
        "wrote trial/lablet.yaml; \
         run it with: cd trial && lablet run --config lablet.yaml --prompt \"Say hello.\""
    );
}
