use rmcp::{
    ServiceExt,
    model::CallToolRequestParams,
    transport::{ConfigureCommandExt, TokioChildProcess},
};
use serde_json::Value;
use tokio::process::Command;

#[tokio::test]
async fn lists_tools_and_runs_local_command() {
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
    assert!(names.contains(&"remote_file_sync"));

    let result = service
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
    let text = result.content[0]
        .raw
        .as_text()
        .expect("text content")
        .text
        .as_str();
    let value: Value = serde_json::from_str(text).expect("json envelope");
    assert_eq!(value["ok"], true);
    assert_eq!(value["data"]["result"]["state"], "exited");
    assert!(
        value["data"]["result"]["chunks"][0]["text"]
            .as_str()
            .unwrap()
            .contains("remote_ops_test_ok")
    );

    service.cancel().await.expect("cancel service");
}
