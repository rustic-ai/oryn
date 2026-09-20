//! Bounded, principal-specific worker-agent IPC. These frames carry AgentId,
//! never a synthetic browsing-context PageId.
use oryn_js::worker_wire::*;
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use std::io::{self, Read, Write};

pub const VERSION: u32 = 19;
pub const MAX_FRAME: usize = 32 * 1024 * 1024;
#[derive(Debug, Serialize, Deserialize)]
pub enum ParentFrame {
    Bootstrap {
        version: u32,
        bootstrap: Box<AgentBootstrap>,
    },
    Start,
    Suspension {
        epoch: u64,
        suspended: bool,
    },
    Command(Box<AgentCommand>),
}
#[derive(Debug, Serialize, Deserialize)]
pub enum ChildFrame {
    Prepared {
        version: u32,
        agent: AgentId,
        process_id: u32,
        binary_sha256: String,
        bootstrap_sha256: String,
        sandbox: String,
        build_profile: String,
    },
    Events(Vec<AgentEvent>),
    Suspension {
        epoch: u64,
        suspended: bool,
    },
    Failure(WorkerError),
}
#[derive(Debug, Serialize, Deserialize)]
pub struct ServiceRequest {
    pub id: u64,
    pub request: AgentServiceRequest,
}
#[derive(Debug, Serialize, Deserialize)]
pub struct ServiceResponse {
    pub id: u64,
    pub result: Result<AgentServiceResponse, WorkerError>,
}
/// Sent only after decoding the complete response and receiving its native FD.
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ServiceReceipt {
    pub received_id: u64,
}

pub fn write_frame(writer: &mut impl Write, value: &impl Serialize) -> io::Result<()> {
    crate::agent_thread::wire_size(value, MAX_FRAME)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    let bytes = serde_json::to_vec(value).map_err(io::Error::other)?;
    let size = u32::try_from(bytes.len()).map_err(io::Error::other)?;
    writer.write_all(&size.to_be_bytes())?;
    writer.write_all(&bytes)?;
    writer.flush()
}
pub fn read_frame<T: DeserializeOwned>(reader: &mut impl Read) -> io::Result<T> {
    let mut size = [0u8; 4];
    reader.read_exact(&mut size)?;
    let size = u32::from_be_bytes(size) as usize;
    if size == 0 || size > MAX_FRAME {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "agent IPC frame limit",
        ));
    }
    let mut bytes = vec![0; size];
    reader.read_exact(&mut bytes)?;
    serde_json::from_slice(&bytes).map_err(io::Error::other)
}
pub fn bootstrap_digest(bootstrap: &AgentBootstrap) -> Result<String, WorkerError> {
    use sha2::{Digest, Sha256};
    let bytes = serde_json::to_vec(bootstrap)
        .map_err(|error| WorkerError::new("DataError", error.to_string()))?;
    Ok(format!("{:x}", Sha256::digest(bytes)))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn stream_transfer_descriptors_survive_native_agent_frames() {
        use oryn_js::message_wire::{PortToken, WireHost, WireValue};
        let value = WireValue {
            version: 1,
            bytes: vec![0],
            hosts: vec![WireHost::Stream {
                readable: Some(0),
                writable: Some(1),
            }],
            buffers: vec![],
            ports: vec![
                PortToken {
                    id: 21,
                    secret: [1; 32],
                },
                PortToken {
                    id: 22,
                    secret: [2; 32],
                },
            ],
            shared: vec![],
            shared_queue: None,
            shared_recipient: None,
            wasm_modules: vec![],
        };
        value.validate().unwrap();
        let mut bytes = Vec::new();
        write_frame(
            &mut bytes,
            &ChildFrame::Events(vec![AgentEvent::Message {
                client: None,
                value,
            }]),
        )
        .unwrap();
        let ChildFrame::Events(events) = read_frame::<ChildFrame>(&mut bytes.as_slice()).unwrap()
        else {
            panic!("event frame")
        };
        let AgentEvent::Message { value, .. } = &events[0] else {
            panic!("message frame")
        };
        assert!(matches!(
            value.hosts[0],
            WireHost::Stream {
                readable: Some(0),
                writable: Some(1)
            }
        ));
        value.validate().unwrap();
        let mut malformed = value.clone();
        malformed.hosts.push(WireHost::Stream {
            readable: Some(0),
            writable: None,
        });
        assert!(malformed.validate().is_err());
        let mut malformed = value.clone();
        malformed.hosts.push(WireHost::Port(0));
        assert!(malformed.validate().is_err());
    }
    #[test]
    fn service_snapshot_queue_admission_survives_agent_frame_encoding() {
        let receipt = ServiceRequest {
            id: 24,
            request: AgentServiceRequest::Service(
                oryn_js::service_worker_wire::ServiceRequest::Acknowledge {
                    creator: Creator::Agent(AgentId(4)),
                    operation: 91,
                },
            ),
        };
        let mut bytes = Vec::new();
        write_frame(&mut bytes, &receipt).unwrap();
        let receipt: ServiceRequest = read_frame(&mut bytes.as_slice()).unwrap();
        assert!(matches!(
            receipt.request,
            AgentServiceRequest::Service(
                oryn_js::service_worker_wire::ServiceRequest::Acknowledge { operation: 91, .. }
            )
        ));
        for take_messages in [false, true] {
            let value = ServiceRequest {
                id: 23,
                request: AgentServiceRequest::Service(
                    oryn_js::service_worker_wire::ServiceRequest::Snapshot {
                        creator: Creator::Agent(AgentId(4)),
                        take_messages,
                    },
                ),
            };
            let mut bytes = Vec::new();
            write_frame(&mut bytes, &value).unwrap();
            let decoded: ServiceRequest = read_frame(&mut bytes.as_slice()).unwrap();
            assert_eq!(decoded.id, 23);
            assert!(
                matches!(decoded.request,AgentServiceRequest::Service(oryn_js::service_worker_wire::ServiceRequest::Snapshot{take_messages:actual,..})if actual==take_messages)
            );
        }
    }
    #[test]
    fn frame_limit_rejects_prefix_before_allocating_or_reading_payload() {
        let mut reader = std::io::Cursor::new(((MAX_FRAME + 1) as u32).to_be_bytes());
        assert_eq!(
            read_frame::<ParentFrame>(&mut reader).unwrap_err().kind(),
            io::ErrorKind::InvalidData
        );
        assert_eq!(reader.position(), 4);
    }
    #[test]
    fn agent_service_frame_preserves_native_utf16_and_exact_correlation() {
        let value = ServiceResponse {
            id: 17,
            result: Ok(AgentServiceResponse::Script {
                url: "https://worker.test/a.js".into(),
                source: vec![0xd800, 0x61, 0xdc00],
                referrer_policy: "no-referrer".into(),
            }),
        };
        let mut bytes = Vec::new();
        write_frame(&mut bytes, &value).unwrap();
        let value: ServiceResponse = read_frame(&mut bytes.as_slice()).unwrap();
        assert_eq!(value.id, 17);
        let Ok(AgentServiceResponse::Script { source, .. }) = value.result else {
            panic!("script frame")
        };
        assert_eq!(source, vec![0xd800, 0x61, 0xdc00]);
    }
    #[test]
    fn parent_commands_preserve_wire_shape_and_decode_existing_frames() {
        for (command, expected) in [
            (AgentCommand::Stop, br#"{"Command":"Stop"}"#.as_slice()),
            (
                AgentCommand::CancelTask { id: 42 },
                br#"{"Command":{"CancelTask":{"id":42}}}"#.as_slice(),
            ),
        ] {
            let frame = ParentFrame::Command(Box::new(command));
            let mut encoded = Vec::new();
            write_frame(&mut encoded, &frame).unwrap();
            assert_eq!(&encoded[..4], &(expected.len() as u32).to_be_bytes());
            assert_eq!(&encoded[4..], expected);
            let mut original = (expected.len() as u32).to_be_bytes().to_vec();
            original.extend_from_slice(expected);
            let ParentFrame::Command(decoded) = read_frame(&mut original.as_slice()).unwrap()
            else {
                panic!("command frame");
            };
            let ParentFrame::Command(expected_command) = frame else {
                unreachable!()
            };
            match (*expected_command, *decoded) {
                (AgentCommand::Stop, AgentCommand::Stop) => {}
                (
                    AgentCommand::CancelTask { id: expected },
                    AgentCommand::CancelTask { id: actual },
                ) => assert_eq!(actual, expected),
                _ => panic!("decoded command changed variant"),
            }
        }
    }
}
