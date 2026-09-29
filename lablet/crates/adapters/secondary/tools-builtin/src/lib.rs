//! Secondary adapter: a `ToolExecutor` for `bash`, `read_file`, and
//! `write_file` (`task_complete` lives in the loop, not here).
//!
//! The tools are a convenience for a task that needs a shell or files. They
//! reproduce no harness's tool suite, and an executor serves only the ones
//! it's built with: [`BuiltinTools::default`] serves none.
//!
//! # The root
//!
//! [`Settings::root`] is where `bash` starts and what the file tools stay
//! under. A path the model gives `read_file` or `write_file` starts at the
//! root unless it's absolute, every symbolic link on it is followed, and
//! what it then leads to has to be under the root, or the call is refused
//! with an error result. `write_file` may name a file that doesn't exist
//! yet, in directories that don't either: the part of the path that exists
//! is resolved, and a symbolic link whose target is missing is refused,
//! since writing to it would make the target wherever the link says.
//!
//! The root bounds the file tools and nothing more. `bash` is as free as the
//! environment lablet runs in, and that environment is what isolates it.
//!
//! # `bash`
//!
//! ```json
//! { "command": "cargo test --quiet" }
//! ```
//!
//! Each command is `bash -c` in a new process, which starts in the root and
//! leads a process group of its own. So no working directory and no variable
//! carries from one command to the next. `bash` is found on the `PATH` the
//! command starts with.
//!
//! A command starts with the variables of [`ENVIRONMENT`] that lablet's own
//! environment holds and with what [`Settings::env`] adds, and with nothing
//! else of lablet's, so the model can't read an API key with `env`. That
//! holds of the command's environment and of nothing else: a command can
//! read lablet's own through the process table, as `ps eww -p $PPID` and
//! `/proc/<pid>/environ` do.
//!
//! The result is what the command wrote, standard output and standard error
//! in the order they were written, then one line with the exit code, as
//! `exit code: 0`. The line is the output's closing line, so the model is
//! sent it whatever the run's output cap leaves out of what the command
//! wrote. A command that exits with another code is a result like any other
//! and not an error: a failing test is something a command reports. A call
//! returns once the shell has exited and nothing it started holds its output
//! open.
//!
//! A command that runs for the shorter of the call's deadline and
//! [`Settings::timeout`] is killed with its whole process group, and the
//! call returns `timeout` once the group has gone. A process that left the
//! group, as one started with `setsid` does, is beyond the kill: it's
//! neither stopped nor waited for. A group counts as gone when none of its
//! processes is left in the process table. A process that was killed stays
//! there until its parent takes it out, and a process whose parent was
//! killed with it is left to the system's first process. Where that's lablet
//! itself, as in a container started without an init, nothing takes it out,
//! and the call returns `failed` after five seconds rather than say that
//! something stopped which it can't see to have stopped.
//!
//! # `read_file`
//!
//! ```json
//! { "path": "src/parser.rs", "offset": 200, "limit": 100 }
//! ```
//!
//! The result is the text of the file, and bytes that aren't UTF-8 are read
//! as U+FFFD. `offset` is how many lines to skip and `limit` how many to
//! return after them, so a file longer than the run's output cap can be
//! read in parts. Both may be left out. A file that doesn't exist, that
//! can't be read, or that's a directory is an error result.
//!
//! # `write_file`
//!
//! ```json
//! { "path": "notes/plan.md", "content": "1. Read the parser.\n" }
//! ```
//!
//! Makes the file, with the directories on the way to it that are missing,
//! or replaces what it held. The result says how many bytes were written.
//!
//! # What a call keeps
//!
//! Every tool feeds its text to a [`lablet_model::KeptOutput`] made from the
//! call's `keep`, as the text arrives, so the executor holds no more than the
//! run's output cap can use however much a command writes or a file holds,
//! and the size it reports is still the size of all of it.
//!
//! Windows isn't supported.

mod bash;
mod executor;
mod read_file;
mod root;
mod settings;
mod text;
mod tool;
mod write_file;

pub use executor::BuiltinTools;
pub use settings::{ENVIRONMENT, Settings, SettingsError, Tool};
