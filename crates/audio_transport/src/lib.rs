use anyhow::{Context, Result};
use common_types::{
    AudioFrame, NodeIdentity, NodeRole, SessionMode, TransportBackend, TransportLossWindow,
    TransportStats, VadDecision, now_micros,
};
use std::collections::{BTreeMap, VecDeque};
use std::io::{Error, ErrorKind};
use std::net::UdpSocket;

const HEADER_BYTES: usize = 24;

pub trait TransportLink: Send {
    fn send_frame(&mut self, frame: &AudioFrame, vad: Option<VadDecision>) -> Result<()>;
    fn recv_or_conceal(&mut self) -> Result<AudioFrame>;
    fn stats(&self) -> TransportStats;
}

pub fn build_transport(
    backend: TransportBackend,
    listen_addr: &str,
    peer_addr: &str,
    jitter_frames: usize,
    frame_ms: usize,
    mock_delay_ms: u16,
    local_identity: NodeIdentity,
    expected_peer_identity: NodeIdentity,
) -> Result<Box<dyn TransportLink>> {
    match backend {
        TransportBackend::Udp => Ok(Box::new(UdpTransport::bind(
            listen_addr,
            peer_addr,
            jitter_frames.max(1),
            local_identity,
            expected_peer_identity,
        )?)),
        TransportBackend::Mock => Ok(Box::new(MockTransport::new(
            (mock_delay_ms as usize / frame_ms.max(1)).max(1),
        ))),
    }
}

pub struct MockTransport {
    delay_frames: usize,
    queue: VecDeque<AudioFrame>,
    last_frame: Option<AudioFrame>,
    loss_window: TransportLossWindow,
    stats: TransportStats,
}

impl MockTransport {
    pub fn new(delay_frames: usize) -> Self {
        Self {
            delay_frames,
            queue: VecDeque::new(),
            last_frame: None,
            loss_window: TransportLossWindow::default(),
            stats: TransportStats::default(),
        }
    }
}

impl TransportLink for MockTransport {
    fn send_frame(&mut self, frame: &AudioFrame, _vad: Option<VadDecision>) -> Result<()> {
        self.stats.sent_packets += 1;
        self.queue.push_back(frame.clone());
        Ok(())
    }

    fn recv_or_conceal(&mut self) -> Result<AudioFrame> {
        if self.queue.len() > self.delay_frames {
            if let Some(frame) = self.queue.pop_front() {
                self.stats.received_packets += 1;
                self.loss_window.record_received();
                self.last_frame = Some(frame.clone());
                return Ok(frame);
            }
        }

        self.stats.concealed_packets += 1;
        self.loss_window.record_concealed();
        let concealed = self
            .last_frame
            .clone()
            .map(|frame| frame.with_timestamp(now_micros()))
            .unwrap_or_else(|| AudioFrame::zero(0));
        Ok(concealed)
    }

    fn stats(&self) -> TransportStats {
        self.stats.with_loss_window(&self.loss_window)
    }
}

pub struct UdpTransport {
    socket: UdpSocket,
    peer_addr: String,
    local_identity: NodeIdentity,
    expected_peer_identity: NodeIdentity,
    jitter_capacity: usize,
    jitter_buffer: BTreeMap<u64, AudioFrame>,
    expected_sequence: Option<u64>,
    last_frame: Option<AudioFrame>,
    loss_window: TransportLossWindow,
    stats: TransportStats,
    rx_buffer: [u8; 4096],
}

impl UdpTransport {
    pub fn bind(
        listen_addr: &str,
        peer_addr: &str,
        jitter_capacity: usize,
        local_identity: NodeIdentity,
        expected_peer_identity: NodeIdentity,
    ) -> Result<Self> {
        let socket = UdpSocket::bind(listen_addr)
            .with_context(|| format!("failed to bind UDP socket on {listen_addr}"))?;
        socket
            .set_nonblocking(true)
            .context("failed to set UDP socket nonblocking")?;

        Ok(Self {
            socket,
            peer_addr: peer_addr.to_owned(),
            local_identity,
            expected_peer_identity,
            jitter_capacity,
            jitter_buffer: BTreeMap::new(),
            expected_sequence: None,
            last_frame: None,
            loss_window: TransportLossWindow::default(),
            stats: TransportStats::default(),
            rx_buffer: [0_u8; 4096],
        })
    }

    fn drain_socket(&mut self) -> Result<()> {
        loop {
            match self.socket.recv_from(&mut self.rx_buffer) {
                Ok((bytes, _)) => {
                    if let Some((frame, identity)) = decode_packet(&self.rx_buffer[..bytes]) {
                        if identity != self.expected_peer_identity {
                            anyhow::bail!(
                                "received packet from incompatible peer role={:?} mode={:?}; expected role={:?} mode={:?}",
                                identity.role,
                                identity.session_mode,
                                self.expected_peer_identity.role,
                                self.expected_peer_identity.session_mode
                            );
                        }
                        self.stats.received_packets += 1;
                        self.jitter_buffer.insert(frame.sequence, frame);
                    }
                }
                Err(error) if error.kind() == ErrorKind::WouldBlock => break,
                Err(error) if udp_receive_error_is_transient(&error) => break,
                Err(error) => return Err(error).context("UDP receive failed"),
            }
        }

        Ok(())
    }

    fn pop_next_frame(&mut self) -> AudioFrame {
        if self.expected_sequence.is_none() {
            self.expected_sequence = self.jitter_buffer.keys().next().copied().or(Some(0));
        }

        let expected = self.expected_sequence.unwrap_or_default();

        if let Some(frame) = self.jitter_buffer.remove(&expected) {
            self.expected_sequence = Some(expected + 1);
            self.loss_window.record_received();
            self.last_frame = Some(frame.clone());
            return frame;
        }

        if self.jitter_buffer.len() > self.jitter_capacity {
            if let Some((&sequence, _)) = self.jitter_buffer.iter().next() {
                let frame = self
                    .jitter_buffer
                    .remove(&sequence)
                    .unwrap_or_else(|| AudioFrame::zero(sequence));
                if sequence > expected {
                    self.stats.dropped_packets += sequence - expected;
                }
                self.expected_sequence = Some(sequence + 1);
                self.loss_window.record_received();
                self.last_frame = Some(frame.clone());
                return frame;
            }
        }

        self.stats.concealed_packets += 1;
        self.loss_window.record_concealed();
        self.expected_sequence = Some(expected + 1);
        self.last_frame
            .clone()
            .map(|frame| frame.with_sequence(expected).with_timestamp(now_micros()))
            .unwrap_or_else(|| AudioFrame::zero(expected))
    }
}

impl TransportLink for UdpTransport {
    fn send_frame(&mut self, frame: &AudioFrame, vad: Option<VadDecision>) -> Result<()> {
        let packet = encode_packet(frame, vad, self.local_identity);
        self.socket
            .send_to(&packet, &self.peer_addr)
            .with_context(|| format!("failed to send UDP packet to {}", self.peer_addr))?;
        self.stats.sent_packets += 1;
        Ok(())
    }

    fn recv_or_conceal(&mut self) -> Result<AudioFrame> {
        self.drain_socket()?;
        Ok(self.pop_next_frame())
    }

    fn stats(&self) -> TransportStats {
        self.stats.with_loss_window(&self.loss_window)
    }
}

fn encode_packet(frame: &AudioFrame, vad: Option<VadDecision>, identity: NodeIdentity) -> Vec<u8> {
    let mut payload = Vec::with_capacity(HEADER_BYTES + frame.samples.len() * 4);
    payload.extend_from_slice(&frame.sequence.to_le_bytes());
    payload.extend_from_slice(&frame.capture_timestamp_us.to_le_bytes());
    payload.extend_from_slice(&frame.sample_rate.to_le_bytes());
    payload.extend_from_slice(&(frame.samples.len() as u16).to_le_bytes());
    payload.push(u8::from(vad.unwrap_or_default().is_speech));
    payload.push(encode_identity(identity));

    for sample in &frame.samples {
        payload.extend_from_slice(&sample.to_le_bytes());
    }

    payload
}

fn decode_packet(payload: &[u8]) -> Option<(AudioFrame, NodeIdentity)> {
    if payload.len() < HEADER_BYTES {
        return None;
    }

    let sequence = u64::from_le_bytes(payload[0..8].try_into().ok()?);
    let capture_timestamp_us = u64::from_le_bytes(payload[8..16].try_into().ok()?);
    let sample_rate = u32::from_le_bytes(payload[16..20].try_into().ok()?);
    let frame_samples = u16::from_le_bytes(payload[20..22].try_into().ok()?) as usize;
    let identity = decode_identity(payload[23])?;
    let sample_bytes = &payload[HEADER_BYTES..];
    if sample_bytes.len() < frame_samples * 4 {
        return None;
    }

    let mut samples = Vec::with_capacity(frame_samples);
    for chunk in sample_bytes.chunks_exact(4).take(frame_samples) {
        samples.push(f32::from_le_bytes(chunk.try_into().ok()?));
    }

    Some((
        AudioFrame::new(sequence, capture_timestamp_us, sample_rate, samples),
        identity,
    ))
}

fn encode_identity(identity: NodeIdentity) -> u8 {
    ((session_mode_code(identity.session_mode) & 0x0F) << 4) | (node_role_code(identity.role) & 0x0F)
}

fn decode_identity(encoded: u8) -> Option<NodeIdentity> {
    Some(NodeIdentity {
        session_mode: session_mode_from_code((encoded >> 4) & 0x0F)?,
        role: node_role_from_code(encoded & 0x0F)?,
    })
}

fn node_role_code(role: NodeRole) -> u8 {
    match role {
        NodeRole::Master => 1,
        NodeRole::Slave => 2,
        NodeRole::Peer => 3,
    }
}

fn node_role_from_code(code: u8) -> Option<NodeRole> {
    match code {
        1 => Some(NodeRole::Master),
        2 => Some(NodeRole::Slave),
        3 => Some(NodeRole::Peer),
        _ => None,
    }
}

fn session_mode_code(mode: SessionMode) -> u8 {
    match mode {
        SessionMode::MasterSlave => 1,
        SessionMode::Peer => 2,
        SessionMode::Both => 3,
    }
}

fn session_mode_from_code(code: u8) -> Option<SessionMode> {
    match code {
        1 => Some(SessionMode::MasterSlave),
        2 => Some(SessionMode::Peer),
        3 => Some(SessionMode::Both),
        _ => None,
    }
}

fn udp_receive_error_is_transient(error: &Error) -> bool {
    matches!(
        error.kind(),
        ErrorKind::ConnectionReset
            | ErrorKind::ConnectionAborted
            | ErrorKind::ConnectionRefused
            | ErrorKind::HostUnreachable
            | ErrorKind::NetworkUnreachable
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use common_types::{TRANSPORT_LOSS_RATE_WINDOW_FRAMES, now_micros};

    #[test]
    fn udp_receive_error_is_transient_for_expected_windows_peer_absence_kinds() {
        for kind in [
            ErrorKind::ConnectionReset,
            ErrorKind::ConnectionAborted,
            ErrorKind::ConnectionRefused,
            ErrorKind::HostUnreachable,
            ErrorKind::NetworkUnreachable,
        ] {
            let error = Error::new(kind, "transient");
            assert!(
                udp_receive_error_is_transient(&error),
                "expected {kind:?} to be treated as transient"
            );
        }
    }

    #[test]
    fn udp_receive_error_is_not_transient_for_unexpected_kinds() {
        for kind in [ErrorKind::TimedOut, ErrorKind::PermissionDenied] {
            let error = Error::new(kind, "fatal");
            assert!(
                !udp_receive_error_is_transient(&error),
                "expected {kind:?} to remain fatal"
            );
        }
    }

    #[test]
    fn mock_transport_loss_rate_uses_recent_2400_frame_window() {
        let mut transport = MockTransport::new(1);
        let frame = AudioFrame::new(1, now_micros(), 48_000, vec![0.0; 480]);

        for _ in 0..3_000 {
            transport
                .send_frame(&frame, None)
                .expect("mock send should succeed");
            transport
                .recv_or_conceal()
                .expect("mock receive should succeed");
        }

        for _ in 0..1_200 {
            transport
                .recv_or_conceal()
                .expect("mock conceal should succeed");
        }

        let stats = transport.stats();
        assert_eq!(
            stats.window_received_packets + stats.window_concealed_packets,
            TRANSPORT_LOSS_RATE_WINDOW_FRAMES as u32
        );
        assert!(
            (stats.loss_rate() - 0.5).abs() < 0.001,
            "expected recent-window loss rate near 50%, got {}",
            stats.loss_rate()
        );
    }

    #[test]
    fn packet_identity_round_trip_preserves_mode_and_role() {
        let frame = AudioFrame::new(7, now_micros(), 48_000, vec![0.0; 480]);
        let identity = NodeIdentity {
            role: NodeRole::Slave,
            session_mode: SessionMode::MasterSlave,
        };

        let packet = encode_packet(&frame, None, identity);
        let (_, decoded_identity) = decode_packet(&packet).expect("packet should decode");
        assert_eq!(decoded_identity, identity);
    }
}
