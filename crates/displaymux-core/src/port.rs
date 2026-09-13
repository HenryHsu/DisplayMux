use crate::{DiscoveredPeer, DisplayInput, DisplayMuxError, MonitorDescriptor, MonitorId};

pub trait MonitorControl {
    fn enumerate(&self) -> Result<Vec<MonitorDescriptor>, DisplayMuxError>;

    fn read_input(&self, monitor: &MonitorId) -> Result<DisplayInput, DisplayMuxError>;

    fn supported_inputs(&self, monitor: &MonitorId) -> Result<Vec<DisplayInput>, DisplayMuxError>;

    fn write_input(&self, monitor: &MonitorId, input: DisplayInput) -> Result<(), DisplayMuxError>;
}

pub trait PeerDiscovery {
    fn peers(&self) -> Result<Vec<DiscoveredPeer>, DisplayMuxError>;
}
