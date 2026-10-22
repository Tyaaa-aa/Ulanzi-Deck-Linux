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

/// Device command codes.
#[repr(u16)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Command {
    SetButtons = 0x0001,
    SmallWindow = 0x0006,
    Brightness = 0x000a,
    LabelStyle = 0x000b,
    PartialUpdate = 0x000d,
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
