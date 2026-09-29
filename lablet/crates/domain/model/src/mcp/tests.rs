use serde_json::json;

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

#[test]
fn an_mcp_lifetime_prints_what_it_serialises_as() {
    for (lifetime, spelling) in [(McpLifetime::Run, "run"), (McpLifetime::Lablet, "lablet")] {
        assert_eq!(lifetime.to_string(), spelling);
        assert_eq!(serde_json::to_value(lifetime).unwrap(), json!(spelling));
        assert_eq!(
            serde_json::from_value::<McpLifetime>(json!(spelling)).unwrap(),
            lifetime
        );
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
    for lifetime in [McpLifetime::Run, McpLifetime::Lablet] {
        let servers = McpServers::new(lifetime, vec![server("docs", "1.4.0")]).unwrap();

        assert_eq!(servers.lifetime(), lifetime);
    }
}

#[test]
fn a_set_of_no_servers_is_refused_whichever_way_it_comes() {
    assert_eq!(
        McpServers::new(McpLifetime::Run, Vec::new()),
        Err(NoMcpServers)
    );

    let error = serde_json::from_value::<McpServers>(json!({ "lifetime": "run", "servers": [] }))
        .unwrap_err();
    assert_eq!(error.to_string(), NoMcpServers.to_string());
}

#[test]
fn the_servers_have_one_json_form() {
    let servers = docs_and_tickets();
    let expected = json!({
        "lifetime": "lablet",
        "servers": [
            { "name": "docs", "version": "1.4.0" },
            { "name": "tickets", "version": "0.9.2" },
        ],
    });

    assert_eq!(serde_json::to_value(&servers).unwrap(), expected);
    assert_eq!(
        serde_json::from_value::<McpServers>(expected).unwrap(),
        servers
    );
}

/// A version with no name beside it, or a lifetime with no servers, was what
/// the lists held apart could say. Each is a document that doesn't read.
#[test]
fn a_server_without_its_version_and_a_lifetime_without_servers_do_not_read() {
    for document in [
        json!({ "lifetime": "run", "servers": [{ "name": "docs" }] }),
        json!({ "lifetime": "run", "servers": [{ "version": "1.4.0" }] }),
        json!({ "lifetime": "run" }),
        json!({ "servers": [{ "name": "docs", "version": "1.4.0" }] }),
        json!({ "lifetime": "run", "servers": [{ "name": "docs", "version": "1.4.0" }], "versions": ["1.4.0"] }),
        json!({ "lifetime": "run", "servers": [{ "name": "docs", "version": "1.4.0", "build": 7 }] }),
    ] {
        assert!(
            serde_json::from_value::<McpServers>(document.clone()).is_err(),
            "{document}"
        );
    }
}
