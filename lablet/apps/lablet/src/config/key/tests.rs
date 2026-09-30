use serde_json::json;

use super::*;

#[test]
fn a_path_is_written_with_dots_between_keys_and_a_list_place_in_brackets() {
    let path = KeyPath::of("tools.mcp").index(1).key("env").key("TOKEN");

    assert_eq!(path.to_string(), "tools.mcp[1].env.TOKEN");
    assert_eq!(KeyPath::of("run").to_string(), "run");
    assert_eq!(KeyPath::default().index(0).to_string(), "[0]");
}

#[test]
fn a_path_finds_what_a_tree_holds_there_and_nothing_where_it_holds_nothing() {
    let tree = json!({ "tools": { "allow": ["bash", "read_file"] } });

    assert_eq!(
        KeyPath::of("tools.allow").index(1).find(&tree),
        Some(&json!("read_file"))
    );
    assert_eq!(KeyPath::of("tools.allow").index(2).find(&tree), None);
    assert_eq!(KeyPath::of("tools.deny").find(&tree), None);
    assert_eq!(KeyPath::default().find(&tree), Some(&tree));
}

#[test]
fn a_path_is_within_itself_and_its_sections_and_nothing_else() {
    let root = KeyPath::of("tools.builtin.root");

    assert!(root.is_within(&root));
    assert!(root.is_within(&KeyPath::of("tools.builtin")));
    assert!(root.is_within(&KeyPath::default()));
    assert!(!root.is_within(&KeyPath::of("tools.builtin.env")));
    assert!(!KeyPath::of("tools").is_within(&root));
}

#[test]
fn a_place_says_the_line_or_that_an_override_set_it() {
    assert_eq!(Place::Line(12).to_string(), "line 12");
    assert_eq!(Place::Override.to_string(), "an override");
}

#[test]
fn places_answer_for_the_settings_they_hold_and_equal_any_other() {
    let run = KeyPath::of("run");
    let places = Places::new(BTreeMap::from([(run.clone(), Place::Line(2))]));

    assert_eq!(places.of(&run), Some(Place::Line(2)));
    assert_eq!(places.of(&KeyPath::of("model")), None);
    assert_eq!(Places::default().of(&run), None);
    assert_eq!(places, Places::default());
}
