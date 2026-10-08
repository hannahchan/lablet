use std::time::Duration;

use lablet_model::{ToolConcurrency, ToolSource};
use lablet_run::{ToolErrorKind, ToolExecutor};
use lablet_tools_builtin::{BuiltinTools, Settings, SettingsError, Tool};
use serde_json::json;

use crate::harness::{Root, ask, link, said};

#[tokio::test]
async fn an_executor_serves_no_tool_unless_it_is_built_with_one() {
    let scratch = Root::new("executor-none");
    let by_default = BuiltinTools::default();
    let with_none = BuiltinTools::new(
        Settings {
            enabled: [].into(),
            ..scratch.settings()
        },
        crate::harness::untraced(),
    )
    .unwrap();

    for tools in [by_default, with_none] {
        assert_eq!(tools.specs().await.unwrap(), []);
        for tool in Tool::ALL {
            let error = ask(&tools, tool.name(), json!({ "command": "pwd" }))
                .await
                .unwrap_err();
            assert_eq!(error.kind, ToolErrorKind::Unknown);
            assert_eq!(
                error.message(),
                format!("no built-in tool named {tool} is enabled")
            );
        }
    }
}

#[tokio::test]
async fn an_executor_serves_the_tools_it_was_built_with_and_no_other() {
    let scratch = Root::new("executor-some");
    scratch.holds("plan.txt", "the plan");
    let tools = BuiltinTools::new(
        Settings {
            enabled: [Tool::ReadFile].into(),
            ..scratch.settings()
        },
        crate::harness::untraced(),
    )
    .unwrap();

    let offered: Vec<String> = tools
        .specs()
        .await
        .unwrap()
        .into_iter()
        .map(|spec| spec.name.into())
        .collect();
    let read = said(&tools, "read_file", json!({ "path": "plan.txt" })).await;
    let ran = ask(&tools, "bash", json!({ "command": "echo ran > ran" })).await;
    let wrote = ask(
        &tools,
        "write_file",
        json!({ "path": "written", "content": "" }),
    )
    .await;

    assert_eq!(offered, ["read_file"]);
    assert_eq!(read, "the plan");
    assert_eq!(ran.unwrap_err().kind, ToolErrorKind::Unknown);
    assert_eq!(wrote.unwrap_err().kind, ToolErrorKind::Unknown);
    assert!(!scratch.root().join("ran").exists());
    assert!(!scratch.root().join("written").exists());
}

#[tokio::test]
async fn the_tools_are_offered_in_one_order_each_with_what_a_model_needs_to_call_it() {
    let scratch = Root::new("executor-specs");
    let tools = BuiltinTools::new(
        Settings {
            enabled: [Tool::WriteFile, Tool::ReadFile, Tool::Bash].into(),
            timeout: Duration::from_secs(120),
            ..scratch.settings()
        },
        crate::harness::untraced(),
    )
    .unwrap();

    let specs = tools.specs().await.unwrap();

    let offered: Vec<_> = specs
        .iter()
        .map(|spec| {
            (
                spec.name.as_str(),
                spec.concurrency,
                spec.input_schema["required"].clone(),
            )
        })
        .collect();
    assert_eq!(
        offered,
        [
            ("bash", ToolConcurrency::Exclusive, json!(["command"])),
            ("read_file", ToolConcurrency::Shared, json!(["path"])),
            (
                "write_file",
                ToolConcurrency::Exclusive,
                json!(["path", "content"])
            ),
        ]
    );
    let root = scratch.root().display().to_string();
    for spec in &specs {
        assert_eq!(spec.source, ToolSource::Builtin);
        assert_eq!(spec.input_schema["type"], "object");
        assert_eq!(spec.input_schema["additionalProperties"], false);
        let properties = spec.input_schema["properties"].as_object().unwrap();
        for (argument, schema) in properties {
            assert!(
                schema["description"]
                    .as_str()
                    .is_some_and(|said| !said.is_empty()),
                "{argument} of {} is described",
                spec.name
            );
        }
        for required in spec.input_schema["required"].as_array().unwrap() {
            assert!(properties.contains_key(required.as_str().unwrap()));
        }
        assert!(
            spec.description
                .ends_with(" A call that takes longer than 120s is stopped."),
            "{}",
            spec.description
        );
        // The digest of a run's tools is taken from the specs, and two runs
        // under two roots are to share it.
        assert!(!spec.description.contains(&root), "{}", spec.description);
    }
    let arguments: Vec<Vec<&String>> = specs
        .iter()
        .map(|spec| {
            spec.input_schema["properties"]
                .as_object()
                .unwrap()
                .keys()
                .collect()
        })
        .collect();
    assert_eq!(
        arguments,
        [
            vec!["command"],
            vec!["limit", "offset", "path"],
            vec!["content", "path"]
        ]
    );
}

#[tokio::test]
async fn a_root_that_is_no_directory_that_exists_is_refused() {
    let scratch = Root::new("executor-root");
    let file = scratch.holds("plan.txt", "the plan");
    let missing = scratch.root().join("missing");

    let of_a_file = BuiltinTools::new(
        Settings {
            root: file.clone(),
            ..scratch.settings()
        },
        crate::harness::untraced(),
    )
    .unwrap_err();
    let of_nothing = BuiltinTools::new(
        Settings {
            root: missing.clone(),
            ..scratch.settings()
        },
        crate::harness::untraced(),
    )
    .unwrap_err();

    assert_eq!(
        of_a_file,
        SettingsError::RootIsNoDirectory {
            root: file.display().to_string()
        }
    );
    assert_eq!(
        of_nothing.to_string(),
        format!(
            "the root {} can't be used: No such file or directory (os error 2)",
            missing.display()
        )
    );
}

#[tokio::test]
async fn a_root_that_is_a_link_is_the_directory_the_link_leads_to() {
    let scratch = Root::new("executor-linked-root");
    let file = scratch.holds("plan.txt", "the plan");
    let linked = scratch.outside("linked");
    link(&scratch.root(), &linked);
    let tools = BuiltinTools::new(
        Settings {
            root: linked.clone(),
            ..scratch.settings()
        },
        crate::harness::untraced(),
    )
    .unwrap();

    let from_the_root = said(&tools, "read_file", json!({ "path": "plan.txt" })).await;
    let by_the_link = said(
        &tools,
        "read_file",
        json!({ "path": linked.join("plan.txt") }),
    )
    .await;
    let by_the_directory = said(&tools, "read_file", json!({ "path": file })).await;
    let started_in = said(&tools, "bash", json!({ "command": "pwd" })).await;

    assert_eq!(from_the_root, "the plan");
    assert_eq!(by_the_link, "the plan");
    assert_eq!(by_the_directory, "the plan");
    assert_eq!(
        started_in,
        format!("{}\nexit code: 0", scratch.root().display())
    );
}

#[tokio::test]
async fn a_variable_no_command_can_start_with_is_refused_when_the_executor_is_built() {
    let scratch = Root::new("executor-variable");

    let refused = BuiltinTools::new(
        Settings {
            env: [("KEY=VALUE".to_owned(), "1".to_owned())].into(),
            ..scratch.settings()
        },
        crate::harness::untraced(),
    )
    .unwrap_err();

    assert_eq!(
        refused.to_string(),
        "the variable \"KEY=VALUE\" can't be set: its name holds `=` or a NUL"
    );
}

#[tokio::test]
async fn an_executor_is_shown_as_its_tools_and_its_timeout() {
    let scratch = Root::new("executor-shown");
    let tools = BuiltinTools::new(
        Settings {
            enabled: [Tool::ReadFile, Tool::Bash].into(),
            env: [(
                "NPM_TOKEN".to_owned(),
                "a value no log is to hold".to_owned(),
            )]
            .into(),
            withheld: ["ANTHROPIC_API_KEY".to_owned()].into(),
            ..scratch.settings()
        },
        crate::harness::untraced(),
    )
    .unwrap();

    assert_eq!(
        format!("{tools:?}"),
        "BuiltinTools { tools: [Bash, ReadFile], timeout: 30s }"
    );
    assert_eq!(
        format!("{:?}", BuiltinTools::default()),
        "BuiltinTools { tools: [], timeout: 0ns }"
    );
}
