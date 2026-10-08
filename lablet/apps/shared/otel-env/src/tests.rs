//! The specification's parsing, over environments the tests state, with
//! the warnings caught by a `tracing` subscriber of the test's own.

use std::ffi::OsString;
use std::io;
use std::num::NonZeroUsize;
use std::os::unix::ffi::OsStringExt as _;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use super::*;

/// What `read` comes to over an environment that holds `held` and nothing
/// else, and each warning it logged, one line each.
pub(crate) fn warned<T>(
    held: &[(&str, &str)],
    read: impl FnOnce(&Variables<'_>) -> T,
) -> (T, Vec<String>) {
    let held: Vec<(String, OsString)> = held
        .iter()
        .map(|(name, value)| ((*name).to_owned(), OsString::from(value)))
        .collect();
    within(&held, read)
}

fn within<T>(
    held: &[(String, OsString)],
    read: impl FnOnce(&Variables<'_>) -> T,
) -> (T, Vec<String>) {
    let env = |name: &str| {
        held.iter()
            .find(|(variable, _)| variable == name)
            .map(|(_, value)| value.clone())
    };
    let written = Written::default();
    let subscriber = tracing_subscriber::fmt()
        .with_writer(written.clone())
        .with_max_level(tracing::Level::WARN)
        .without_time()
        .with_target(false)
        .with_level(false)
        .with_ansi(false)
        .finish();
    let read = tracing::subscriber::with_default(subscriber, || read(&Variables(&env)));
    let text = String::from_utf8(written.0.lock().unwrap().clone()).unwrap();
    (read, text.lines().map(str::to_owned).collect())
}

/// What the subscriber writes, kept.
#[derive(Debug, Clone, Default)]
struct Written(Arc<Mutex<Vec<u8>>>);

impl io::Write for Written {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for Written {
    type Writer = Self;

    fn make_writer(&'a self) -> Self::Writer {
        self.clone()
    }
}

/// A choice for the tests of lists and names.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Colour {
    Red,
    Blue,
}

impl Choice for Colour {
    fn named(name: &str) -> Option<Self> {
        match name {
            "red" => Some(Self::Red),
            "blue" => Some(Self::Blue),
            _ => None,
        }
    }
}

const VARIABLE: &str = "OTEL_TEST_VARIABLE";

fn get<T: Parse>(value: &str) -> (Option<T>, Vec<String>) {
    warned(&[(VARIABLE, value)], |variables| {
        variables.get::<T>(VARIABLE)
    })
}

#[test]
fn an_empty_variable_is_unset() {
    let unset = warned(&[], |variables| {
        (
            variables.get::<String>(VARIABLE),
            variables.get::<u32>(VARIABLE),
            variables.get::<bool>(VARIABLE),
            variables.get::<Vec<Colour>>(VARIABLE),
            variables.get::<Named<Colour>>(VARIABLE),
        )
    });
    let empty = warned(&[(VARIABLE, "")], |variables| {
        (
            variables.get::<String>(VARIABLE),
            variables.get::<u32>(VARIABLE),
            variables.get::<bool>(VARIABLE),
            variables.get::<Vec<Colour>>(VARIABLE),
            variables.get::<Named<Colour>>(VARIABLE),
        )
    });

    assert_eq!(unset, ((None, None, None, None, None), Vec::new()));
    assert_eq!(empty, unset, "nothing read, and no word of it");
}

#[test]
fn a_value_that_is_not_utf_8_is_ignored_with_a_warning() {
    let held = [(VARIABLE.to_owned(), OsString::from_vec(vec![0xff, b'1']))];

    let (read, warnings) = within(&held, |variables| variables.get::<String>(VARIABLE));

    assert_eq!(read, None);
    assert_eq!(
        warnings,
        ["`OTEL_TEST_VARIABLE` holds what isn't UTF-8, so it's ignored"]
    );
}

#[test]
fn a_whole_number_is_digits_alone_and_anything_else_is_ignored_with_a_warning() {
    assert_eq!(get::<u32>("0"), (Some(0), Vec::new()));
    assert_eq!(get::<u32>("4096"), (Some(4096), Vec::new()));
    for value in ["-1", "+1", " 1", "1.5", "1e3", "a lot", "4294967296"] {
        assert_eq!(
            get::<u32>(value),
            (
                None,
                vec![format!(
                    "`OTEL_TEST_VARIABLE` holds `{value}`, which isn't a whole number, so it's \
                     ignored"
                )]
            ),
            "{value}"
        );
    }
}

#[test]
fn a_size_of_nothing_is_out_of_range_and_ignored_with_a_warning() {
    assert_eq!(get::<NonZeroUsize>("1"), (NonZeroUsize::new(1), Vec::new()));
    assert_eq!(
        get::<NonZeroUsize>("0"),
        (
            None,
            vec![
                "`OTEL_TEST_VARIABLE` holds `0`, which isn't a whole number above zero, so it's \
                 ignored"
                    .to_owned()
            ]
        )
    );
}

#[test]
fn a_timeout_is_whole_milliseconds_and_zero_is_no_limit() {
    assert_eq!(
        get::<Timeout>("1500"),
        (Some(Timeout(Duration::from_millis(1_500))), Vec::new())
    );
    assert_eq!(
        get::<Timeout>("0"),
        (
            Some(Timeout(Duration::from_millis(2_147_483_647))),
            Vec::new()
        )
    );
    assert_eq!(get::<Duration>("0"), (Some(Duration::ZERO), Vec::new()));
    assert_eq!(
        get::<Timeout>("-1"),
        (
            None,
            vec![
                "`OTEL_TEST_VARIABLE` holds `-1`, which isn't a whole number of milliseconds, so \
                 it's ignored"
                    .to_owned()
            ]
        )
    );
}

#[test]
fn a_duration_is_whole_milliseconds() {
    assert_eq!(
        get::<Duration>("1500"),
        (Some(Duration::from_millis(1_500)), Vec::new())
    );
    assert_eq!(
        get::<Duration>("1.5s"),
        (
            None,
            vec![
                "`OTEL_TEST_VARIABLE` holds `1.5s`, which isn't a whole number of milliseconds, so \
                 it's ignored"
                    .to_owned()
            ]
        )
    );
}

#[test]
fn a_boolean_is_true_only_when_it_is_true_in_any_case() {
    for value in ["true", "TRUE", "True"] {
        assert_eq!(get::<bool>(value), (Some(true), Vec::new()), "{value}");
    }
    for value in ["false", "FALSE"] {
        assert_eq!(get::<bool>(value), (Some(false), Vec::new()), "{value}");
    }
    for value in ["1", "yes", " true"] {
        assert_eq!(
            get::<bool>(value),
            (
                None,
                vec![format!(
                    "`OTEL_TEST_VARIABLE` holds `{value}`, which isn't `true` or `false`, so it's \
                     read as `false`"
                )]
            ),
            "{value}"
        );
    }
}

#[test]
fn a_name_is_matched_in_any_case_and_trimmed() {
    assert_eq!(
        get::<Named<Colour>>(" RED "),
        (Some(Named(Colour::Red)), Vec::new())
    );
    assert_eq!(
        get::<Named<Colour>>("green"),
        (
            None,
            vec![
                "`OTEL_TEST_VARIABLE` holds `green`, which isn't one lablet serves, so it's ignored"
                    .to_owned()
            ]
        )
    );
}

#[test]
fn a_list_is_trimmed_matched_in_any_case_and_has_its_duplicates_dropped() {
    assert_eq!(
        get::<Vec<Colour>>(" Blue, red ,BLUE,,"),
        (Some(vec![Colour::Blue, Colour::Red]), Vec::new())
    );
}

#[test]
fn none_alone_is_the_empty_list_and_beside_another_name_it_is_ignored_with_a_warning() {
    assert_eq!(get::<Vec<Colour>>("NONE"), (Some(Vec::new()), Vec::new()));
    assert_eq!(
        get::<Vec<Colour>>("none,red"),
        (
            Some(vec![Colour::Red]),
            vec![
                "`OTEL_TEST_VARIABLE` holds `none`, which is beside another value, so it's ignored"
                    .to_owned()
            ]
        )
    );
}

#[test]
fn a_list_naming_nothing_lablet_serves_is_read_as_unset_with_a_warning_for_each_name() {
    assert_eq!(
        get::<Vec<Colour>>("green,red"),
        (
            Some(vec![Colour::Red]),
            vec![
                "`OTEL_TEST_VARIABLE` holds `green`, which isn't one lablet serves, so it's ignored"
                    .to_owned()
            ]
        )
    );
    assert_eq!(
        get::<Vec<Colour>>("green,mauve"),
        (
            None,
            vec![
                "`OTEL_TEST_VARIABLE` holds `green`, which isn't one lablet serves, so it's ignored"
                    .to_owned(),
                "`OTEL_TEST_VARIABLE` holds `mauve`, which isn't one lablet serves, so it's ignored"
                    .to_owned(),
            ]
        )
    );
}

/// A value that may hold a secret.
#[derive(Debug, PartialEq, Eq)]
struct Secret;

impl Parse for Secret {
    const SECRET: bool = true;

    fn parse(text: &str, ignored: &mut dyn FnMut(&str, &str)) -> Option<Self> {
        ignored(text, "isn't one, so it's ignored");
        None
    }
}

#[test]
fn a_warning_of_a_value_that_may_be_a_secret_names_the_variable_alone() {
    assert_eq!(
        get::<Secret>("hunter2"),
        (
            None,
            vec!["`OTEL_TEST_VARIABLE` holds a value that isn't one, so it's ignored".to_owned()]
        )
    );
}
