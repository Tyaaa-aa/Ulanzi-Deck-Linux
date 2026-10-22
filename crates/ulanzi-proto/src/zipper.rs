use std::collections::BTreeMap;
use std::io::{Cursor, Write};

use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipWriter};

use crate::manifest::{ButtonEntry, manifest_json};
use crate::packet::{Command, MAX_PAYLOAD, PACKET_SIZE, Packet, packet};

/// A PNG stored under `icons/<name>` in the button ZIP.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ButtonImage {
    pub name: String,
    pub png: Vec<u8>,
}

const MAX_PADDING_ATTEMPTS: usize = 30;

/// Build the button ZIP, varying the leading `dummy.txt` until no `0x00`/`0x7c` byte lands on a
/// packet boundary (a firmware transfer bug). Gives up after 30 attempts and returns the last layout.
pub fn button_zip(entries: &BTreeMap<u8, ButtonEntry>, images: &[ButtonImage]) -> Vec<u8> {
    let manifest = manifest_json(entries);
    let mut last = Vec::new();
    for attempt in 0..=MAX_PADDING_ATTEMPTS {
        last = build_zip(&padding(attempt), images, &manifest);
        if zip_is_clean(&last) {
            return last;
        }
    }
    tracing::warn!("no clean button ZIP layout after {MAX_PADDING_ATTEMPTS} attempts; sending anyway");
    last
}

fn padding(attempt: usize) -> String {
    if attempt == 0 {
        return "init".into();
    }
    (0..8 + 8 * attempt)
        .map(|i| (b'a' + ((i * 7 + attempt) % 26) as u8) as char)
        .collect()
}

fn build_zip(dummy: &str, images: &[ButtonImage], manifest: &str) -> Vec<u8> {
    let mut zw = ZipWriter::new(Cursor::new(Vec::new()));
    let deflated = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
    // The padding file is stored uncompressed so each attempt shifts every later byte by exactly
    // its extra length; compressed, the repetitive padding would barely change the layout.
    let stored = SimpleFileOptions::default().compression_method(CompressionMethod::Stored);
    // Writing to an in-memory buffer cannot fail except on absurd sizes; treat that as a bug.
    let mut add = |name: &str, data: &[u8], opts: SimpleFileOptions| {
        zw.start_file(name, opts).expect("in-memory zip entry");
        zw.write_all(data).expect("in-memory zip write");
    };
    add("dummy.txt", dummy.as_bytes(), stored);
    for img in images {
        add(&format!("icons/{}", img.name), &img.png, deflated);
    }
    add("manifest.json", manifest.as_bytes(), deflated);
    zw.finish().expect("in-memory zip finish").into_inner()
}

/// True when no byte at offset `1016 + k*1024` is `0x00` or `0x7c`.
pub fn zip_is_clean(zip: &[u8]) -> bool {
    (MAX_PAYLOAD..zip.len())
        .step_by(PACKET_SIZE)
        .all(|i| zip[i] != 0x00 && zip[i] != 0x7c)
}

/// Split a file upload into packets: a header packet carrying the first 1016 bytes and the total
/// size, then raw zero-padded 1024-byte chunks.
pub fn file_packets(cmd: Command, data: &[u8]) -> Vec<Packet> {
    let first = &data[..data.len().min(MAX_PAYLOAD)];
    let mut out = vec![packet(cmd, first, data.len() as u32)];
    for chunk in data.get(MAX_PAYLOAD..).unwrap_or_default().chunks(PACKET_SIZE) {
        let mut p = [0u8; PACKET_SIZE];
        p[..chunk.len()].copy_from_slice(chunk);
        out.push(p);
    }
    out
}
