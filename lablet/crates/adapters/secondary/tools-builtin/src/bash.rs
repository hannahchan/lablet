//! `bash`: one command, in a new process that leads a process group of its
//! own.

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::os::fd::OwnedFd;
use std::process::{ExitStatus, Stdio};
use std::time::Duration;

use lablet_model::ToolConcurrency;
use lablet_run::{ToolError, ToolOutput};
use nix::errno::Errno;
use nix::sys::signal::{Signal, killpg};
use nix::unistd::Pid;
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::io::AsyncReadExt as _;
use tokio::net::unix::pipe::Receiver;
use tokio::process::{Child, Command};

use crate::Tool;
use crate::root::Root;
use crate::text::Text;
use crate::tool::{BuiltIn, Offer, Terms, failed};

/// The shell, found on the `PATH` a command starts with.
const SHELL: &str = "bash";

/// How much of what a command wrote is read at a time.
const PIECE_BYTES: usize = 16 * 1024;

/// How long a group that was killed is given to go. A process that was
/// killed goes at once, and whoever it was left to takes it from the process
/// table moments later, so a group that's still there after this is one
/// nobody takes: lablet runs as the first process of a container that has no
/// init.
const GONE_WITHIN: Duration = Duration::from_secs(5);

/// How long passes between two looks at a group that was killed.
const LOOKS_EVERY: Duration = Duration::from_millis(1);

const DESCRIPTION: &str = "Runs a command with `bash -c` and returns what it wrote: standard \
output and standard error together, in the order they were written, then one line with the \
exit code, as `exit code: 0`. A command that exits with another code is a result like any \
other. Every command is a new process that starts in the run's root directory, so a working \
directory, a variable or a function that one command set isn't there for the next: write \
`cd dir && command` as one command. The call returns when the command has exited and nothing \
it started holds its output open, so send the output of a process that's to stay in the \
background somewhere else, as in `server > server.log 2>&1 &`. A command that runs for as \
long as a call may take is killed, with every process it started.";

/// Runs commands in the root, with the environment it's given and nothing
/// else of lablet's.
pub(crate) struct Bash {
    pub(crate) root: Root,
    pub(crate) environment: BTreeMap<OsString, OsString>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Arguments {
    command: String,
}

/// The process group of one command.
///
/// A group that's dropped while its command runs is killed, so a call that
/// was given up leaves nothing running.
struct Group {
    id: Pid,
    running: bool,
}

/// One command that was started: its shell, the group the shell leads, and
/// the end of the pipe that everything in the group writes to.
struct Started {
    shell: Child,
    group: Group,
    wrote: Receiver,
}

impl Group {
    /// Kills every process of the group. A group that has gone already has
    /// nothing to kill, which is what was wanted.
    fn kill(&self) {
        let _ = killpg(self.id, Signal::SIGKILL);
    }

    /// Whether any process of the group is left, one that has exited and
    /// that nobody has taken from the process table among them.
    fn is_there(&self) -> bool {
        killpg(self.id, None) != Err(Errno::ESRCH)
    }

    /// Waits for the group to go, and says whether it went.
    async fn went(&mut self) -> bool {
        let went = went(|| self.is_there()).await;
        self.running = !went;
        went
    }
}

/// Waits for as long as `is_there` says that something is, and no longer
/// than [`GONE_WITHIN`], and says whether it went.
///
/// What looks is a parameter so that a test can say what's seen, and when.
async fn went(is_there: impl Fn() -> bool) -> bool {
    let gone = async {
        while is_there() {
            tokio::time::sleep(LOOKS_EVERY).await;
        }
    };
    tokio::time::timeout(GONE_WITHIN, gone).await.is_ok()
}

/// How a call ends whose command ran for as long as it may and was killed,
/// once the shell was `waited` for.
///
/// It's a timeout only when everything the command started `went`, because
/// a timeout says that the work has stopped.
async fn ended_by_the_kill(
    terms: Terms<'_>,
    waited: std::io::Result<ExitStatus>,
    went: impl Future<Output = bool>,
) -> ToolError {
    let killed = format!("the command was killed after {:?}", terms.limit);
    match waited {
        Ok(_) if went.await => terms.ran_out(Tool::Bash),
        Ok(_) => failed(format!(
            "{killed}, and {GONE_WITHIN:?} later a process it started hadn't gone"
        )),
        Err(error) => failed(format!(
            "{killed}, and the shell couldn't be waited for: {error}"
        )),
    }
}

impl Drop for Group {
    fn drop(&mut self) {
        if self.running {
            self.kill();
        }
    }
}

impl Bash {
    fn start(&self, command: &str) -> std::io::Result<Started> {
        // One pipe for both streams, so what the command wrote is read in
        // the order it was written. The `Command` holds lablet's copies of
        // the end that's written to, and is dropped with this function: the
        // pipe then ends when the last process of the group lets go of it.
        let (wrote, writes) = std::io::pipe()?;
        let shell = Command::new(SHELL)
            .arg("-c")
            .arg(command)
            .current_dir(self.root.path())
            .env_clear()
            .envs(&self.environment)
            .stdin(Stdio::null())
            .stdout(writes.try_clone()?)
            .stderr(writes)
            .process_group(0)
            .spawn()?;
        let id = shell
            .id()
            .and_then(|id| i32::try_from(id).ok())
            .ok_or_else(|| std::io::Error::other("the shell has no process id"))?;
        Ok(Started {
            shell,
            group: Group {
                id: Pid::from_raw(id),
                running: true,
            },
            wrote: Receiver::from_owned_fd(OwnedFd::from(wrote))?,
        })
    }
}

/// Feeds `text` what the command writes until nothing holds the pipe open,
/// then waits for the shell.
async fn read_to_the_end(
    wrote: &mut Receiver,
    shell: &mut Child,
    text: &mut Text,
) -> std::io::Result<ExitStatus> {
    let mut piece = vec![0; PIECE_BYTES];
    loop {
        let read = wrote.read(&mut piece).await?;
        if read == 0 {
            break;
        }
        text.feed(&piece[..read]);
    }
    shell.wait().await
}

/// The line that says how the shell ended.
fn ended(status: ExitStatus) -> String {
    status.code().map_or_else(
        // Killed by a signal, which the status names.
        || status.to_string(),
        |code| format!("exit code: {code}"),
    )
}

#[async_trait::async_trait]
impl BuiltIn for Bash {
    fn tool(&self) -> Tool {
        Tool::Bash
    }

    fn offer(&self) -> Offer {
        Offer {
            description: DESCRIPTION,
            input_schema: json!({
                "type": "object",
                "properties": {
                    "command": {
                        "type": "string",
                        "description": "The command line, as it would be typed at a bash prompt."
                    }
                },
                "required": ["command"],
                "additionalProperties": false
            }),
            concurrency: ToolConcurrency::Exclusive,
        }
    }

    async fn run(&self, input: Value, terms: Terms<'_>) -> Result<ToolOutput, ToolError> {
        let Arguments { command } = match terms.arguments(Tool::Bash, input) {
            Ok(arguments) => arguments,
            Err(refusal) => return Ok(*refusal),
        };
        let Started {
            mut shell,
            mut group,
            mut wrote,
        } = self
            .start(&command)
            .map_err(|error| failed(format!("{SHELL} couldn't be started: {error}")))?;
        let mut text = Text::new(terms.keep, terms.secrets);

        let reading = read_to_the_end(&mut wrote, &mut shell, &mut text);
        let Ok(read) = tokio::time::timeout(terms.limit, reading).await else {
            group.kill();
            let waited = shell.wait().await;
            return Err(ended_by_the_kill(terms, waited, group.went()).await);
        };
        let status =
            read.map_err(|error| failed(format!("what the command wrote wasn't read: {error}")))?;
        // The shell has exited and was waited for, so its id is free for
        // another process to have. What it left running is left running.
        group.running = false;

        text.close(&ended(status));
        Ok(ToolOutput {
            output: text.kept(),
            is_error: false,
            mcp: None,
        })
    }
}

#[cfg(test)]
mod tests;
