mod domain;
mod error;
mod network;
mod port;
mod service;

#[cfg(target_os = "macos")]
pub mod macos;
#[cfg(target_os = "windows")]
pub mod windows;

pub use domain::{
    DestinationHost, DiscoveredPeer, DisplayInput, DisplayMuxProfile, MonitorDescriptor,
    MonitorFingerprint, MonitorId, MonitorResolution, SwitchMode, SwitchOutcome,
};
pub use error::DisplayMuxError;
pub use network::{
    AgentAction, AgentClient, AgentRequest, AgentResponse, AgentServer, MacAddress,
    MdnsPeerDiscovery, PeerEndpoint, WakeTarget, DEFAULT_AGENT_PORT,
};
pub use port::{MonitorControl, PeerDiscovery};
pub use service::DisplayMuxService;
