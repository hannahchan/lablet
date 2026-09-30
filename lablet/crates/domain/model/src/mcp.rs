//! The MCP servers that serve a run, and how long they live.

/// How long a run's MCP servers live.
///
/// A server that outlives a run carries what the run left in it to the next,
/// so two runs on one `Lablet` are comparable only when the record says
/// which they had.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum McpLifetime {
    /// The servers are started again for each run.
    Run,
    /// The servers are started once and serve every run of their `Lablet`.
    Lablet,
}

impl McpLifetime {
    /// How a run's record spells it.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Run => "run",
            Self::Lablet => "lablet",
        }
    }
}

display_as_str!(McpLifetime);

every_variant!(McpLifetime::ALL = [Run, Lablet]);

/// One MCP server of a run.
///
/// The name and the version are one value because they're reported as two
/// lists, and two lists held apart can differ in length or in order.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct McpServer {
    /// The server's name in the config.
    pub name: String,
    /// The version the server gave of itself when it started.
    pub version: String,
}

/// The MCP servers of a run that has any, and how long they live.
///
/// The lifetime is held with the servers it's the lifetime of, and there's
/// at least one of those, so a run has both or neither: a lifetime says
/// whether a server's state could have carried over from another run, which
/// it says of no server when there's none. [`McpServers::new`] refuses a set
/// of no servers, and the fields aren't public.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct McpServers {
    lifetime: McpLifetime,
    servers: Vec<McpServer>,
}

/// A run's MCP servers were given as none at all.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("a run's MCP servers are at least one server, and a run without any has none to report")]
pub struct NoMcpServers;

impl McpServers {
    /// The servers of a run, in config order, which live as long as
    /// `lifetime` says.
    ///
    /// # Errors
    ///
    /// Returns [`NoMcpServers`] when `servers` is empty. A run with no MCP
    /// servers holds no `McpServers`, rather than an empty one with a
    /// lifetime that's nothing's.
    pub fn new(lifetime: McpLifetime, servers: Vec<McpServer>) -> Result<Self, NoMcpServers> {
        if servers.is_empty() {
            return Err(NoMcpServers);
        }
        Ok(Self { lifetime, servers })
    }

    /// How long the servers live.
    #[must_use]
    pub const fn lifetime(&self) -> McpLifetime {
        self.lifetime
    }

    /// The servers, in config order; never empty.
    #[must_use]
    pub fn servers(&self) -> &[McpServer] {
        &self.servers
    }

    /// The servers' names, in the order of [`McpServers::versions`].
    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.servers.iter().map(|server| server.name.as_str())
    }

    /// The version each server gave of itself, in the order of
    /// [`McpServers::names`].
    pub fn versions(&self) -> impl Iterator<Item = &str> {
        self.servers.iter().map(|server| server.version.as_str())
    }
}

#[cfg(test)]
mod tests;
