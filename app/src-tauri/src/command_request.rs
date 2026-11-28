use reqwest::{Client, StatusCode, redirect::Policy};
use serde::{Deserialize, Serialize};
use std::time::Duration;

const COMMAND_REQUEST_ENDPOINT: &str = "https://tryvoxi.vercel.app/api/command-request";
const WRONG_RESULT_ENDPOINT: &str = "https://tryvoxi.vercel.app/api/wrong-result";
const REPORT_FAILED: &str = "Couldn't send the report. Try again.";
const SEND_FAILED: &str = "Couldn't send the request. Try again.";

#[derive(Serialize)]
struct Request<'a> {
    command: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    note: Option<&'a str>,
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct WrongResult {
    command: String,
    outcome: String,
    actions: Vec<ReportedAction>,
    note: String,
    app_version: String,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct ReportedAction {
    kind: String,
    count: u64,
}

#[derive(Deserialize)]
struct Response {
    ok: bool,
    error: Option<String>,
}

#[derive(Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub(crate) enum RequestResult {
    Sent,
    RateLimited,
    Failed { message: String },
}

// An async Tauri command runs on its runtime, outside the UI thread.
#[tauri::command]
pub(crate) async fn send_command_request(command: String, note: Option<String>) -> RequestResult {
    let request = Request { command: &command, note: note.as_deref() };
    send(COMMAND_REQUEST_ENDPOINT, &request, SEND_FAILED).await.unwrap_or_else(|_| RequestResult::Failed { message: SEND_FAILED.into() })
}

#[tauri::command]
pub(crate) async fn send_wrong_result(report: WrongResult) -> RequestResult {
    send(WRONG_RESULT_ENDPOINT, &report, REPORT_FAILED).await.unwrap_or_else(|_| RequestResult::Failed { message: REPORT_FAILED.into() })
}

async fn send(endpoint: &str, request: &impl Serialize, failed: &str) -> reqwest::Result<RequestResult> {
    // The updater uses the same TLS provider and may not have run yet.
    let _ = rustls::crypto::ring::default_provider().install_default();
    let response = Client::builder()
        .https_only(true)
        .redirect(Policy::none())
        .timeout(Duration::from_secs(10))
        .build()?
        .post(endpoint)
        .json(request)
        .send()
        .await?;

    let status = response.status();
    if status == StatusCode::TOO_MANY_REQUESTS {
        return Ok(RequestResult::RateLimited);
    }
    if matches!(status, StatusCode::OK | StatusCode::BAD_REQUEST | StatusCode::SERVICE_UNAVAILABLE) {
        let reply = response.json::<Response>().await?;
        if status == StatusCode::OK && reply.ok {
            return Ok(RequestResult::Sent);
        }
        if let Some(message) = reply.error {
            return Ok(RequestResult::Failed { message });
        }
    }
    Ok(RequestResult::Failed { message: failed.into() })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{json, to_value};

    #[test]
    fn payload_contains_only_the_edited_command_and_note() {
        let request = Request { command: "Group files by month", note: Some("Keep existing folders") };
        assert_eq!(
            to_value(request).unwrap(),
            json!({
                "command": "Group files by month",
                "note": "Keep existing folders",
            })
        );
    }

    #[test]
    fn missing_note_is_omitted() {
        let request = Request { command: "Group files by month", note: None };
        assert_eq!(to_value(request).unwrap(), json!({ "command": "Group files by month" }));
    }
}
