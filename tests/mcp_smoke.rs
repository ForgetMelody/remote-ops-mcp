use std::{
    fs,
    path::PathBuf,
    time::{SystemTime, UNIX_EPOCH},
};

use rmcp::{
    ServiceExt,
    model::{CallToolRequestParams, CallToolResult},
    transport::{ConfigureCommandExt, TokioChildProcess},
};
use serde_json::Value;
use tokio::process::Command;

#[tokio::test]
async fn lists_tools_and_exercises_local_job_and_file_tools() {
    let binary = env!("CARGO_BIN_EXE_remote-ops-mcp");
    let service = ()
        .serve(
            TokioChildProcess::new(Command::new(binary).configure(|cmd| {
                cmd.arg("--config").arg("config.example.toml");
            }))
            .expect("spawn server"),
        )
        .await
        .expect("connect mcp server");

    let tools = service
        .list_tools(Default::default())
        .await
        .expect("list tools");
    let names: Vec<_> = tools.tools.iter().map(|tool| tool.name.as_ref()).collect();
    assert!(names.contains(&"remote_backend_health"));
    assert!(names.contains(&"remote_run"));
    assert!(names.contains(&"remote_start"));
    assert!(names.contains(&"remote_file_sync"));

    let run = service
        .call_tool(CallToolRequestParams {
            meta: None,
            name: "remote_run".into(),
            arguments: serde_json::json!({
                "target": "local",
                "command": "printf remote_ops_test_ok",
                "timeout_s": 5
            })
            .as_object()
            .cloned(),
            task: None,
        })
        .await
        .expect("call remote_run");
    let run = tool_json(run);
    assert_eq!(run["ok"], true);
    assert_eq!(run["data"]["result"]["state"], "exited");
    assert!(
        run["data"]["result"]["chunks"][0]["text"]
            .as_str()
            .unwrap()
            .contains("remote_ops_test_ok")
    );

    let start = service
        .call_tool(CallToolRequestParams {
            meta: None,
            name: "remote_start".into(),
            arguments: serde_json::json!({
                "target": "local",
                "command": "printf remote_ops_job_started; exec sleep 30",
                "initial_wait_s": 1,
                "follow_limit": 1024
            })
            .as_object()
            .cloned(),
            task: None,
        })
        .await
        .expect("call remote_start");
    let start = tool_json(start);
    assert_eq!(start["ok"], true);
    assert!(
        start["data"]["initial"]["chunks"][0]["text"]
            .as_str()
            .unwrap()
            .contains("remote_ops_job_started")
    );
    let job_id = start["data"]["job_id"].as_str().unwrap().to_owned();

    let stop = service
        .call_tool(CallToolRequestParams {
            meta: None,
            name: "remote_stop".into(),
            arguments: serde_json::json!({ "job_id": job_id }).as_object().cloned(),
            task: None,
        })
        .await
        .expect("call remote_stop");
    let stop = tool_json(stop);
    assert_eq!(stop["ok"], true);
    assert_eq!(stop["data"]["state"], "cancelled");

    let temp_deploy = unique_temp_dir();
    fs::create_dir_all(&temp_deploy).expect("create temp deploy");
    let source = temp_deploy.join("source.txt");
    let copied = temp_deploy.join("copied.txt");
    fs::write(&source, "remote ops file sync").expect("write source");

    let sync = service
        .call_tool(CallToolRequestParams {
            meta: None,
            name: "remote_file_sync".into(),
            arguments: serde_json::json!({
                "target": "local",
                "direction": "push",
                "local_path": source,
                "remote_path": copied,
                "timeout_s": 5
            })
            .as_object()
            .cloned(),
            task: None,
        })
        .await
        .expect("call remote_file_sync");
    let sync = tool_json(sync);
    assert_eq!(sync["ok"], true);
    assert_eq!(fs::read_to_string(&copied).unwrap(), "remote ops file sync");

    let stat = service
        .call_tool(CallToolRequestParams {
            meta: None,
            name: "remote_file_stat".into(),
            arguments: serde_json::json!({
                "target": "local",
                "path": copied,
                "timeout_s": 5
            })
            .as_object()
            .cloned(),
            task: None,
        })
        .await
        .expect("call remote_file_stat");
    let stat = tool_json(stat);
    assert_eq!(stat["ok"], true);
    assert_eq!(stat["data"]["exit_code"], 0);

    fs::remove_dir_all(temp_deploy).expect("cleanup temp deploy");
    service.cancel().await.expect("cancel service");
}

fn tool_json(result: CallToolResult) -> Value {
    let text = result.content[0]
        .raw
        .as_text()
        .expect("text content")
        .text
        .as_str();
    serde_json::from_str(text).expect("json envelope")
}

fn unique_temp_dir() -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    std::env::temp_dir().join(format!(
        "remote-ops-mcp-test-{}-{nanos}",
        std::process::id()
    ))
}
