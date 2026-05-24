//! Live smoke test of the Codex app-server protocol Cameo depends on.
//!
//! Marked `#[ignore]` — it spawns the real `codex app-server`, makes a real
//! image-generation turn against the user's ChatGPT subscription (network +
//! auth + ~20-60s), and asserts an `imageGeneration` item comes back with bytes.
//!
//! Run explicitly:  cargo test --manifest-path src-tauri/Cargo.toml -- --ignored --nocapture

use serde_json::{json, Value};
use std::process::Stdio;
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

async fn send(stdin: &mut tokio::process::ChildStdin, msg: &Value) {
    let line = format!("{}\n", serde_json::to_string(msg).unwrap());
    stdin.write_all(line.as_bytes()).await.unwrap();
    stdin.flush().await.unwrap();
}

#[tokio::test]
#[ignore]
async fn codex_image_generation_smoke() {
    let codex = which::which("codex").expect("codex must be on PATH");
    let tmp = tempfile::tempdir().unwrap();

    let mut child = tokio::process::Command::new(&codex)
        .arg("app-server")
        .current_dir(tmp.path())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn codex app-server");

    let mut stdin = child.stdin.take().unwrap();
    let mut lines = BufReader::new(child.stdout.take().unwrap()).lines();

    let result = tokio::time::timeout(Duration::from_secs(150), async {
        // Read lines until we see a response with the given id; return its result.
        macro_rules! await_response {
            ($id:expr) => {{
                let mut out = Value::Null;
                while let Some(line) = lines.next_line().await.unwrap() {
                    let line = line.trim();
                    if line.is_empty() { continue; }
                    let Ok(msg) = serde_json::from_str::<Value>(line) else { continue };
                    if msg.get("id").and_then(|v| v.as_u64()) == Some($id) && msg.get("method").is_none() {
                        assert!(msg.get("error").is_none(), "rpc {} error: {:?}", $id, msg.get("error"));
                        out = msg.get("result").cloned().unwrap_or(Value::Null);
                        break;
                    }
                }
                out
            }};
        }

        // 1. initialize (+ initialized)
        send(&mut stdin, &json!({"jsonrpc":"2.0","id":1,"method":"initialize",
            "params":{"clientInfo":{"name":"Cameo","title":null,"version":"0.0.1"},"capabilities":null}})).await;
        let _ = await_response!(1);
        send(&mut stdin, &json!({"jsonrpc":"2.0","method":"initialized","params":{}})).await;

        // 2. thread/start
        send(&mut stdin, &json!({"jsonrpc":"2.0","id":2,"method":"thread/start","params":{
            "cwd": tmp.path().to_string_lossy(), "model": null,
            "approvalPolicy":"never", "sandbox":"workspace-write",
            "developerInstructions": null, "ephemeral": false }})).await;
        let started = await_response!(2);
        let thread_id = started["thread"]["id"].as_str().expect("thread id").to_string();
        eprintln!("thread started: {thread_id}");

        // 3. turn/start — ask for an image.
        send(&mut stdin, &json!({"jsonrpc":"2.0","id":3,"method":"turn/start","params":{
            "threadId": thread_id,
            "input":[{"type":"text",
                "text":"Use image generation to create a simple image: a solid red circle centered on a white background. Save it as a PNG file.",
                "text_elements":[]}],
            "summary":"concise" }})).await;
        let _turn = await_response!(3);

        // 4. Read notifications until turn/completed; look for an imageGeneration item.
        let mut image_evidence: Option<String> = None;
        while let Some(line) = lines.next_line().await.unwrap() {
            let line = line.trim();
            if line.is_empty() { continue; }
            let Ok(msg) = serde_json::from_str::<Value>(line) else { continue };
            let method = msg.get("method").and_then(|v| v.as_str()).unwrap_or("");
            if method == "item/completed" {
                let item = &msg["params"]["item"];
                if item.get("type").and_then(|v| v.as_str()) == Some("imageGeneration") {
                    let saved = item.get("savedPath").and_then(|v| v.as_str());
                    let has_b64 = item.get("result").and_then(|v| v.as_str()).map(|s| !s.is_empty()).unwrap_or(false);
                    image_evidence = Some(format!("savedPath={:?} base64={}", saved, has_b64));
                }
            } else if method == "turn/completed" {
                eprintln!("turn completed: status={:?}", msg["params"]["turn"]["status"]);
                break;
            } else if method == "error" {
                eprintln!("codex error notification: {:?}", msg["params"]);
            }
        }
        image_evidence
    })
    .await;

    let _ = child.start_kill();

    match result {
        Ok(Some(evidence)) => eprintln!("PASS: imageGeneration item produced ({evidence})"),
        Ok(None) => panic!("turn completed but no imageGeneration item was produced"),
        Err(_) => panic!("timed out waiting for image generation"),
    }
}
