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

/// Build the button ZIP: `icons/*` then `manifest.json`.
pub fn button_zip(entries: &BTreeMap<u8, ButtonEntry>, images: &[ButtonImage]) -> Vec<u8> {
    let mut files: Vec<(String, &[u8])> = images
        .iter()
        .map(|i| (format!("icons/{}", i.name), i.png.as_slice()))
        .collect();
    let manifest = manifest_json(entries);
    files.push(("manifest.json".into(), manifest.as_bytes()));
    build_zip(&files)
}

/// ZIP for the deck's "!" flash on one key (it draws the symbol itself).
pub fn alert_zip(index: u8) -> Vec<u8> {
    let manifest = format!(
        r#"{{"{}":{{"KeyType":1,"Progress":100,"State":1}}}}"#,
        crate::manifest::manifest_key(index)
    );
    build_zip(&[("manifest.json".into(), manifest.as_bytes())])
}

fn build_zip(files: &[(String, &[u8])]) -> Vec<u8> {
    let mut zw = ZipWriter::new(Cursor::new(Vec::new()));
    let opts = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
    // Writing to an in-memory buffer cannot fail except on absurd sizes; treat that as a bug.
    for (name, data) in files {
        zw.start_file(name.as_str(), opts).expect("in-memory zip entry");
        zw.write_all(data).expect("in-memory zip write");
    }
    zw.finish().expect("in-memory zip finish").into_inner()
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
