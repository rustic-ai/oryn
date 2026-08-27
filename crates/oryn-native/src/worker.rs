use std::io::{BufRead, Write};

use oryn_common::v2::{Observation, Revision};
use serde::{Deserialize, Serialize};

use crate::{
    Browser, NavigationResult, PageHandle,
    oil::{NativeOilOutput, NativeOilSession},
    runtime::NativeAction,
};

#[derive(Debug, Deserialize, PartialEq, Eq)]
#[serde(tag = "command", rename_all = "snake_case")]
pub enum WorkerRequest {
    Ping,
    Goto { url: String },
    LoadHtml { url: String, html: String },
    Observe,
    Execute { action: NativeActionRequest },
    Oil { input: String },
    Trace,
    Exit,
}

#[derive(Debug, Deserialize, PartialEq, Eq)]
pub struct NativeActionRequest {
    pub target: oryn_common::v2::SemanticRef,
    pub action: oryn_common::v2::SemanticAction,
    pub value: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum WorkerResponse {
    Pong {
        protocol_version: u32,
    },
    Loaded {
        revision: Revision,
    },
    Navigated {
        navigation: NavigationResult,
    },
    Observation {
        observation: Observation,
    },
    Action {
        result: oryn_common::v2::ActionResult,
    },
    Oil {
        outputs: Vec<NativeOilOutput>,
    },
    Trace {
        events: Vec<crate::trace::TraceEvent>,
    },
    Exiting,
    Error {
        message: String,
    },
}

pub async fn serve<R: BufRead, W: Write>(reader: R, mut writer: W) -> std::io::Result<()> {
    let page = Browser::new().new_context().new_page();
    let mut oil = NativeOilSession::new(page.clone());
    for line in reader.lines() {
        let response = match line {
            Ok(line) => dispatch(&page, &mut oil, &line).await,
            Err(error) => WorkerResponse::Error {
                message: error.to_string(),
            },
        };
        let exiting = matches!(response, WorkerResponse::Exiting);
        serde_json::to_writer(&mut writer, &response)?;
        writer.write_all(b"\n")?;
        writer.flush()?;
        if exiting {
            break;
        }
    }
    let _ = page.close().await;
    Ok(())
}

async fn dispatch(page: &PageHandle, oil: &mut NativeOilSession, line: &str) -> WorkerResponse {
    let request = match serde_json::from_str::<WorkerRequest>(line) {
        Ok(request) => request,
        Err(error) => {
            return WorkerResponse::Error {
                message: format!("invalid worker request: {error}"),
            };
        }
    };
    match request {
        WorkerRequest::Ping => WorkerResponse::Pong {
            protocol_version: 1,
        },
        WorkerRequest::Goto { url } => match page.goto(url).await {
            Ok(navigation) => WorkerResponse::Navigated { navigation },
            Err(error) => WorkerResponse::Error {
                message: error.to_string(),
            },
        },
        WorkerRequest::LoadHtml { url, html } => match page.load_html(url, html).await {
            Ok(revision) => WorkerResponse::Loaded { revision },
            Err(error) => WorkerResponse::Error {
                message: error.to_string(),
            },
        },
        WorkerRequest::Observe => match page.observe().await {
            Ok(observation) => WorkerResponse::Observation { observation },
            Err(error) => WorkerResponse::Error {
                message: error.to_string(),
            },
        },
        WorkerRequest::Execute { action } => match page
            .execute(NativeAction {
                target: action.target,
                action: action.action,
                value: action.value,
            })
            .await
        {
            Ok(result) => WorkerResponse::Action { result },
            Err(error) => WorkerResponse::Error {
                message: error.to_string(),
            },
        },
        WorkerRequest::Oil { input } => match oil.execute(&input).await {
            Ok(outputs) => WorkerResponse::Oil { outputs },
            Err(error) => WorkerResponse::Error {
                message: error.to_string(),
            },
        },
        WorkerRequest::Trace => match page.trace().await {
            Ok(events) => WorkerResponse::Trace { events },
            Err(error) => WorkerResponse::Error {
                message: error.to_string(),
            },
        },
        WorkerRequest::Exit => WorkerResponse::Exiting,
    }
}

#[cfg(test)]
mod tests {
    use std::io::{BufReader, Cursor};

    use super::*;

    #[tokio::test(flavor = "current_thread")]
    async fn serves_page_requests_over_json_lines() {
        let input = br#"{"command":"ping"}
{"command":"load_html","url":"https://example.test","html":"<button>Save</button>"}
{"command":"observe"}
{"command":"oil","input":"observe\nclick \"Save\""}
{"command":"trace"}
{"command":"exit"}
"#;
        let mut output = Vec::new();
        serve(BufReader::new(Cursor::new(input)), &mut output)
            .await
            .expect("serve requests");
        let lines: Vec<serde_json::Value> = output
            .split(|byte| *byte == b'\n')
            .filter(|line| !line.is_empty())
            .map(|line| serde_json::from_slice(line).expect("valid response"))
            .collect();

        assert_eq!(lines.len(), 6);
        assert_eq!(lines[0]["status"], "pong");
        assert_eq!(lines[1]["revision"], 1);
        assert_eq!(lines[2]["observation"]["nodes"][0]["name"], "Save");
        assert_eq!(lines[3]["status"], "oil", "{lines:#?}");
        assert_eq!(lines[3]["outputs"][1]["kind"], "action");
        assert_eq!(lines[4]["status"], "trace");
        assert!(lines[4]["events"].as_array().is_some_and(|events| {
            events
                .iter()
                .any(|event| event["kind"]["kind"] == "event_dispatched")
        }));
        assert_eq!(lines[5]["status"], "exiting");
    }
}
