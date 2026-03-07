use anyhow::{Context, Result};
use common_types::{AudioFrame, TransportBackend, TransportStats, VadDecision, now_micros};
use std::collections::{BTreeMap, VecDeque};
use std::io::ErrorKind;
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
) -> Result<Box<dyn TransportLink>> {
    match backend {
        TransportBackend::Udp => Ok(Box::new(UdpTransport::bind(
            listen_addr,
            peer_addr,
            jitter_frames.max(1),
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
    stats: TransportStats,
}

impl MockTransport {
    pub fn new(delay_frames: usize) -> Self {
        Self {
            delay_frames,
            queue: VecDeque::new(),
            last_frame: None,
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
                self.last_frame = Some(frame.clone());
                return Ok(frame);
            }
        }

        self.stats.concealed_packets += 1;
        let concealed = self
            .last_frame
            .clone()
            .map(|frame| frame.with_timestamp(now_micros()))
            .unwrap_or_else(|| AudioFrame::zero(0));
        Ok(concealed)
    }

    fn stats(&self) -> TransportStats {
        self.stats
    }
}

pub struct UdpTransport {
    socket: UdpSocket,
    peer_addr: String,
    jitter_capacity: usize,
    jitter_buffer: BTreeMap<u64, AudioFrame>,
    expected_sequence: Option<u64>,
    last_frame: Option<AudioFrame>,
    stats: TransportStats,
    rx_buffer: [u8; 4096],
}

impl UdpTransport {
    pub fn bind(listen_addr: &str, peer_addr: &str, jitter_capacity: usize) -> Result<Self> {
        let socket = UdpSocket::bind(listen_addr)
            .with_context(|| format!("failed to bind UDP socket on {listen_addr}"))?;
        socket
            .set_nonblocking(true)
            .context("failed to set UDP socket nonblocking")?;

        Ok(Self {
            socket,
            peer_addr: peer_addr.to_owned(),
            jitter_capacity,
            jitter_buffer: BTreeMap::new(),
            expected_sequence: None,
            last_frame: None,
            stats: TransportStats::default(),
            rx_buffer: [0_u8; 4096],
        })
    }

    fn drain_socket(&mut self) -> Result<()> {
        loop {
            match self.socket.recv_from(&mut self.rx_buffer) {
                Ok((bytes, _)) => {
                    if let Some(frame) = decode_packet(&self.rx_buffer[..bytes]) {
                        self.stats.received_packets += 1;
                        self.jitter_buffer.insert(frame.sequence, frame);
                    }
                }
                Err(error) if error.kind() == ErrorKind::WouldBlock => break,
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
                self.last_frame = Some(frame.clone());
                return frame;
            }
        }

        self.stats.concealed_packets += 1;
        self.expected_sequence = Some(expected + 1);
        self.last_frame
            .clone()
            .map(|frame| frame.with_sequence(expected).with_timestamp(now_micros()))
            .unwrap_or_else(|| AudioFrame::zero(expected))
    }
}

impl TransportLink for UdpTransport {
    fn send_frame(&mut self, frame: &AudioFrame, vad: Option<VadDecision>) -> Result<()> {
        let packet = encode_packet(frame, vad);
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
        self.stats
    }
}

fn encode_packet(frame: &AudioFrame, vad: Option<VadDecision>) -> Vec<u8> {
    let mut payload = Vec::with_capacity(HEADER_BYTES + frame.samples.len() * 4);
    payload.extend_from_slice(&frame.sequence.to_le_bytes());
    payload.extend_from_slice(&frame.capture_timestamp_us.to_le_bytes());
    payload.extend_from_slice(&frame.sample_rate.to_le_bytes());
    payload.extend_from_slice(&(frame.samples.len() as u16).to_le_bytes());
    payload.push(u8::from(vad.unwrap_or_default().is_speech));
    payload.push(0);

    for sample in &frame.samples {
        payload.extend_from_slice(&sample.to_le_bytes());
    }

    payload
}

fn decode_packet(payload: &[u8]) -> Option<AudioFrame> {
    if payload.len() < HEADER_BYTES {
        return None;
    }

    let sequence = u64::from_le_bytes(payload[0..8].try_into().ok()?);
    let capture_timestamp_us = u64::from_le_bytes(payload[8..16].try_into().ok()?);
    let sample_rate = u32::from_le_bytes(payload[16..20].try_into().ok()?);
    let frame_samples = u16::from_le_bytes(payload[20..22].try_into().ok()?) as usize;
    let sample_bytes = &payload[HEADER_BYTES..];
    if sample_bytes.len() < frame_samples * 4 {
        return None;
    }

    let mut samples = Vec::with_capacity(frame_samples);
    for chunk in sample_bytes.chunks_exact(4).take(frame_samples) {
        samples.push(f32::from_le_bytes(chunk.try_into().ok()?));
    }

    Some(AudioFrame::new(
        sequence,
        capture_timestamp_us,
        sample_rate,
        samples,
    ))
}
