use super::*;

#[test]
fn two_directories_made_under_one_name_are_two_directories() {
    let first = Scratch::new("twice");
    let second = Scratch::new("twice");

    assert_ne!(first.path(), second.path());
    first.write("mine.txt", "first");
    assert!(!second.at("mine.txt").exists());
}

#[test]
fn the_directory_and_everything_in_it_are_removed_when_it_is_dropped() {
    let scratch = Scratch::new("dropped");
    scratch.write("nested/deeper/file.txt", "text");
    let path = scratch.path().to_owned();

    drop(scratch);

    assert!(!path.exists(), "{} is still there", path.display());
}

#[test]
fn the_path_is_the_one_the_system_resolves_the_directory_to() {
    let outer = Scratch::new("outer");
    outer.create_dir("real");
    let real = outer.at("real");
    std::os::unix::fs::symlink(&real, outer.at("linked")).unwrap();

    let scratch = Scratch::under(&outer.at("linked"), "inner");

    assert!(scratch.path().is_dir());
    assert!(
        scratch.path().starts_with(&real),
        "{} is not under {}",
        scratch.path().display(),
        real.display()
    );
}

#[test]
fn what_is_written_is_written_where_the_path_says_with_the_directories_on_the_way() {
    let scratch = Scratch::new("write");

    let path = scratch.write("a/b/notes.md", "on it goes");

    assert_eq!(path, scratch.path().join("a/b/notes.md"));
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "on it goes");
}

#[test]
fn a_directory_is_made_where_the_path_says_with_the_directories_on_the_way() {
    let scratch = Scratch::new("create-dir");

    scratch.create_dir("work/nested");

    assert!(scratch.path().join("work/nested").is_dir());
}

#[test]
fn a_path_in_the_directory_is_named_and_nothing_is_made_there() {
    let scratch = Scratch::new("at");

    let path = scratch.at("missing/file.txt");

    assert_eq!(path, scratch.path().join("missing/file.txt"));
    assert!(!scratch.at("missing").exists());
}
