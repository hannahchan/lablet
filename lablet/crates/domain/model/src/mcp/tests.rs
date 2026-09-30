use super::*;

fn server(name: &str, version: &str) -> McpServer {
    McpServer {
        name: name.to_owned(),
        version: version.to_owned(),
    }
}

fn docs_and_tickets() -> McpServers {
    McpServers::new(
        McpLifetime::Lablet,
        vec![server("docs", "1.4.0"), server("tickets", "0.9.2")],
    )
    .unwrap()
}

const fn spelling(lifetime: McpLifetime) -> &'static str {
    match lifetime {
        McpLifetime::Run => "run",
        McpLifetime::Lablet => "lablet",
    }
}

#[test]
fn an_mcp_lifetime_prints_its_spelling() {
    for lifetime in McpLifetime::ALL {
        let spelling = spelling(lifetime);
        assert_eq!(lifetime.as_str(), spelling);
        assert_eq!(lifetime.to_string(), spelling);
    }
}

/// The wide event reports the names and the versions as two lists, so the
/// n-th of one has to be the n-th server's of the other.
#[test]
fn the_names_and_the_versions_are_each_server_s_own_in_config_order() {
    let servers = docs_and_tickets();

    assert_eq!(servers.names().collect::<Vec<_>>(), ["docs", "tickets"]);
    assert_eq!(servers.versions().collect::<Vec<_>>(), ["1.4.0", "0.9.2"]);
    assert_eq!(
        servers.servers(),
        [server("docs", "1.4.0"), server("tickets", "0.9.2")]
    );
}

#[test]
fn the_servers_say_how_long_they_live() {
    for lifetime in McpLifetime::ALL {
        let servers = McpServers::new(lifetime, vec![server("docs", "1.4.0")]).unwrap();

        assert_eq!(servers.lifetime(), lifetime);
    }
}

#[test]
fn a_set_of_no_servers_is_refused() {
    assert_eq!(
        McpServers::new(McpLifetime::Run, Vec::new()),
        Err(NoMcpServers)
    );
}
