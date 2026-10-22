/// USB vendor ID of the Ulanzi D200 family.
pub const VID: u16 = 0x2207;
/// USB product ID of the Ulanzi D200 family.
pub const PID: u16 = 0x0019;
/// Every report to and from the device is exactly this long.
pub const PACKET_SIZE: usize = 1024;
/// Header: `7c 7c`, u16 BE command, u32 LE length.
pub const HEADER_SIZE: usize = 8;
/// Largest payload that fits in one packet after the header.
pub const MAX_PAYLOAD: usize = PACKET_SIZE - HEADER_SIZE;

pub type Packet = [u8; PACKET_SIZE];

const MAGIC: [u8; 2] = [0x7c, 0x7c];

/// Device command codes. On the wire these are two bytes, a command code (0 = host request) and a
/// protocol ID, which reads the same as this big-endian u16 for everything the host sends.
#[repr(u16)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Command {
    SetButtons = 0x0001,
    DeviceInfo = 0x0003,
    Lock = 0x0004,
    Unlock = 0x0005,
    SmallWindow = 0x0006,
    Screensaver = 0x0009,
    Brightness = 0x000a,
    LabelStyle = 0x000b,
    PartialUpdate = 0x000d,
    SleepTime = 0x001a,
    Alert = 0x00e0,
}

/// Build one packet. `payload` is truncated to [`MAX_PAYLOAD`] bytes; the rest is zero.
/// `declared_len` is the length written in the header (the whole file size for multi-packet uploads).
pub fn packet(cmd: Command, payload: &[u8], declared_len: u32) -> Packet {
    let mut p = [0u8; PACKET_SIZE];
    p[..2].copy_from_slice(&MAGIC);
    p[2..4].copy_from_slice(&(cmd as u16).to_be_bytes());
    p[4..8].copy_from_slice(&declared_len.to_le_bytes());
    let n = payload.len().min(MAX_PAYLOAD);
    p[HEADER_SIZE..HEADER_SIZE + n].copy_from_slice(&payload[..n]);
    p
}

/// Packet that carries a short payload whose declared length is its own length.
pub(crate) fn simple(cmd: Command, payload: &[u8]) -> Packet {
    packet(cmd, payload, payload.len().min(MAX_PAYLOAD) as u32)
}

/// What is actually written to hidraw: report ID 0, then the packet. The kernel strips a leading 0,
/// so the deck receives exactly the 1024 packet bytes, including continuation chunks that start with
/// 0x00 (without the prefix those lost their first byte).
pub fn with_report_id(p: &Packet) -> [u8; PACKET_SIZE + 1] {
    let mut out = [0u8; PACKET_SIZE + 1];
    out[1..].copy_from_slice(p);
    out
}
