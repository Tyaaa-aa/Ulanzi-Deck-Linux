//! Direct hardware probes for the unknowns in the spec (U1-U5). They talk to the device without the
//! service, so they refuse to run while it, or anything else, holds the deck.

use std::collections::BTreeMap;
use std::io::{BufRead, Cursor};

use std::path::Path;
use std::time::{Duration, Instant};
use tokio::io::AsyncBufReadExt;

use clap::Subcommand;
use resvg::tiny_skia::{self, Paint, Pixmap, Rect, Transform};
use ulanzi_core::render::{Fonts, TextStyle};
use ulanzi_deckd::transport::{DeviceState, Hidraw, Transport, probe};
use ulanzi_proto::{
    ButtonEntry, ButtonImage, Command, LabelStyle, ScreenMode, SmallWindow, brightness, button_zip,
    file_packets, label_style, parse_input, small_window,
};

#[derive(Subcommand, Debug, PartialEq)]
pub enum Probe {
    /// Print every input report for 30 s (U1: which index each control sends).
    Input,
    /// Show test patterns of several sizes on the wide screen (U2).
    Screen,
    /// Try partial key updates and time full uploads (U3).
    Partial,
    /// Listen for a device-info report (U4).
    Info,
    /// Check whether keys left out of an upload keep their image (U5).
    Omit,
    /// Try each wide-screen mode, re-sending the image after each one.
    Modes,
    /// Upload realistic full pages 2 s apart and report if the deck reboots (USB disconnect); no user needed.
    Crash,
}

pub async fn run(what: Probe) -> Result<(), String> {
    let mut dev = open().await?;
    match what {
        Probe::Input => input(&mut dev).await,
        Probe::Screen => screen(&mut dev).await,
        Probe::Partial => partial(&mut dev).await,
        Probe::Info => info(&mut dev).await,
        Probe::Omit => omit(&mut dev).await,
        Probe::Modes => modes(&mut dev).await,
        Probe::Crash => crash(&mut dev).await,
    }
}

async fn open() -> Result<Hidraw, String> {
    if let Ok(conn) = zbus::Connection::session().await
        && let Ok(dbus) = zbus::fdo::DBusProxy::new(&conn).await
        && let Ok(name) = zbus::names::BusName::try_from(ulanzi_deckd::dbus::BUS_NAME)
        && dbus.name_has_owner(name).await.unwrap_or(false)
    {
        return Err(
            "the Ulanzi Deck service is running; stop it first: systemctl --user stop ulanzi-deckd".into(),
        );
    }
    match probe(
        Path::new("/sys/class/hidraw"),
        Path::new("/dev"),
        Path::new("/proc"),
    ) {
        DeviceState::Ready { node } => Hidraw::open(&node).map_err(|s| s.detail()),
        other => Err(other.detail()),
    }
}

fn wait_enter(prompt: &str) {
    println!("{prompt}  [Enter]");
    let _ = std::io::stdin().lock().lines().next();
}

async fn send(dev: &mut Hidraw, packets: &[ulanzi_proto::Packet]) -> Result<(), String> {
    dev.send(packets).await.map_err(|e| e.to_string())
}

async fn input(dev: &mut Hidraw) -> Result<(), String> {
    // The deck may stay silent until the host has talked to it, so greet it like the daemon does.
    send(dev, &[brightness(60), label_style(&LabelStyle::default())]).await?;
    send(dev, &[ulanzi_proto::unlock()]).await?;
    let window = SmallWindow {
        mode: ScreenMode::Clock,
        cpu: 0,
        mem: 0,
        time: "12:00:00".into(),
        gpu: 0,
        clock_24h: true,
        date: String::new(),
    };
    send(dev, &[small_window(&window)]).await?;
    send(
        dev,
        &page(&(0..13).map(|i| (i, solid([40, 70, 120]))).collect::<Vec<_>>()),
    )
    .await?;
    wait_enter("All keys should now be blue. Press Enter here when you are ready to start pressing");
    println!("RECORDING. Press each control. Press Enter here when you are done.");
    let mut stdin = tokio::io::BufReader::new(tokio::io::stdin()).lines();
    loop {
        tokio::select! {
            _ = stdin.next_line() => break,
            r = dev.recv() => {
                let Ok(buf) = r else { break };
                let raw: Vec<String> = buf.iter().take(12).map(|b| format!("{b:02x}")).collect();
                match parse_input(&buf) {
                    Some(e) => println!("{:<12} {:<10} index={:<3} b2={} b3={}  [{}]",
                        e.control.as_str(), format!("{:?}", e.kind), e.raw_index, buf[10], buf[11], raw.join(" ")),
                    None => println!("(other report) [{}]", raw.join(" ")),
                }
            }
        }
    }
    println!("stopped.");
    Ok(())
}

fn png(pix: &Pixmap) -> Vec<u8> {
    pix.encode_png().unwrap_or_default()
}

/// A labelled grid with a red border, so stretching and cropping are obvious.
fn pattern(w: u32, h: u32, fonts: &mut Fonts) -> Vec<u8> {
    let Some(mut pix) = Pixmap::new(w, h) else {
        return Vec::new();
    };
    pix.fill(tiny_skia::Color::from_rgba8(10, 20, 40, 255));
    let mut p = Paint::default();
    p.set_color_rgba8(90, 110, 150, 255);
    for x in (0..w).step_by(28) {
        if let Some(r) = Rect::from_xywh(x as f32, 0.0, 1.0, h as f32) {
            pix.fill_rect(r, &p, Transform::identity(), None);
        }
    }
    for y in (0..h).step_by(28) {
        if let Some(r) = Rect::from_xywh(0.0, y as f32, w as f32, 1.0) {
            pix.fill_rect(r, &p, Transform::identity(), None);
        }
    }
    p.set_color_rgba8(230, 40, 40, 255);
    for r in [
        Rect::from_xywh(0.0, 0.0, w as f32, 4.0),
        Rect::from_xywh(0.0, h as f32 - 4.0, w as f32, 4.0),
        Rect::from_xywh(0.0, 0.0, 4.0, h as f32),
        Rect::from_xywh(w as f32 - 4.0, 0.0, 4.0, h as f32),
    ]
    .into_iter()
    .flatten()
    {
        pix.fill_rect(r, &p, Transform::identity(), None);
    }
    p.set_color_rgba8(255, 255, 255, 255);
    if let Some(c) = tiny_skia::PathBuilder::from_circle(w as f32 / 2.0, h as f32 / 2.0, h as f32 / 3.0) {
        pix.stroke_path(
            &c,
            &p,
            &tiny_skia::Stroke {
                width: 3.0,
                ..Default::default()
            },
            Transform::identity(),
            None,
        );
    }
    fonts.draw(
        &mut pix,
        &format!("{w}x{h}"),
        (12.0, 10.0),
        &TextStyle {
            size: 28.0,
            bold: true,
            rgb: [255, 255, 255],
        },
    );
    png(&pix)
}

fn solid(rgb: [u8; 3]) -> Vec<u8> {
    let img = image::RgbaImage::from_pixel(196, 196, image::Rgba([rgb[0], rgb[1], rgb[2], 255]));
    let mut out = Vec::new();
    let _ =
        image::DynamicImage::ImageRgba8(img).write_to(&mut Cursor::new(&mut out), image::ImageFormat::Png);
    out
}

/// Same content as `button_zip`, but `manifest.json` first and fast compression (homedeck's layout).
fn manifest_first_zip(entries: &BTreeMap<u8, ButtonEntry>, images: &[ButtonImage]) -> Vec<u8> {
    use std::io::Write;
    let mut zw = zip::ZipWriter::new(Cursor::new(Vec::new()));
    let opts = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated)
        .compression_level(Some(1));
    zw.start_file("manifest.json", opts).expect("zip");
    zw.write_all(ulanzi_proto::manifest_json(entries).as_bytes())
        .expect("zip");
    for img in images {
        zw.start_file(format!("icons/{}", img.name), opts).expect("zip");
        zw.write_all(&img.png).expect("zip");
    }
    zw.finish().expect("zip").into_inner()
}

fn zip_for(entries: &BTreeMap<u8, ButtonEntry>, images: &[ButtonImage]) -> Vec<u8> {
    if std::env::var("ULANZI_ZIP").as_deref() == Ok("manifest-first") {
        manifest_first_zip(entries, images)
    } else {
        button_zip(entries, images)
    }
}

fn partial_packets(keys: &[(u8, Vec<u8>)]) -> Vec<ulanzi_proto::Packet> {
    let mut entries = BTreeMap::new();
    let mut images = Vec::new();
    for (i, data) in keys {
        let name = format!("q{i}_{}.png", unique());
        entries.insert(
            *i,
            ButtonEntry {
                state: 0,
                text: None,
                icon: Some(format!("icons/{name}")),
            },
        );
        images.push(ButtonImage {
            name,
            png: data.clone(),
        });
    }
    file_packets(Command::PartialUpdate, &zip_for(&entries, &images))
}

fn unique() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0)
}

fn page(keys: &[(u8, Vec<u8>)]) -> Vec<ulanzi_proto::Packet> {
    let mut entries = BTreeMap::new();
    let mut images = Vec::new();
    for (i, data) in keys {
        // The deck caches images by file name, so every upload gets a fresh one.
        let name = format!("p{i}_{}.png", unique());
        entries.insert(
            *i,
            ButtonEntry {
                state: 0,
                text: None,
                icon: Some(format!("icons/{name}")),
            },
        );
        images.push(ButtonImage {
            name,
            png: data.clone(),
        });
    }
    file_packets(Command::SetButtons, &zip_for(&entries, &images))
}

async fn screen(dev: &mut Hidraw) -> Result<(), String> {
    let mut fonts = Fonts::system();
    send(dev, &[brightness(60)]).await?;
    send(dev, &[ulanzi_proto::unlock()]).await?;
    let window = SmallWindow {
        mode: ScreenMode::Image,
        cpu: 0,
        mem: 0,
        time: "00:00:00".into(),
        gpu: 0,
        clock_24h: true,
        date: String::new(),
    };
    send(dev, &[small_window(&window)]).await?;
    for (w, h) in [(196, 196), (392, 196), (248, 196), (458, 196)] {
        send(dev, &page(&[(13, pattern(w, h, &mut fonts))])).await?;
        wait_enter(&format!(
            "\nNOW SHOWING: {w}x{h}. Take a photo, then press Enter for the next one."
        ));
    }
    println!("\nDone. All screen tests finished.");
    Ok(())
}

async fn partial(dev: &mut Hidraw) -> Result<(), String> {
    send(
        dev,
        &page(&(0..13).map(|i| (i, solid([40, 40, 40]))).collect::<Vec<_>>()),
    )
    .await?;
    wait_enter("all keys should be dark grey now");
    let mut entries = BTreeMap::new();
    entries.insert(
        0u8,
        ButtonEntry {
            state: 0,
            text: None,
            icon: Some("icons/red.png".into()),
        },
    );
    let zip = button_zip(
        &entries,
        &[ButtonImage {
            name: "red.png".into(),
            png: solid([230, 30, 30]),
        }],
    );
    let t = Instant::now();
    send(dev, &file_packets(Command::PartialUpdate, &zip)).await?;
    println!("partial update sent in {:?}", t.elapsed());
    wait_enter("did ONLY key 1 turn red, with the other keys still grey?");
    for n in [1u32, 10, 30] {
        let t = Instant::now();
        for k in 0..n {
            let shade = (k * 7 % 200) as u8 + 30;
            send(
                dev,
                &page(&(0..13).map(|i| (i, solid([shade, 60, 90]))).collect::<Vec<_>>()),
            )
            .await?;
        }
        println!(
            "{n} full uploads in {:?} ({:?} each)",
            t.elapsed(),
            t.elapsed() / n
        );
    }
    wait_enter("did the keys keep changing smoothly without glitches?");
    Ok(())
}

async fn info(dev: &mut Hidraw) -> Result<(), String> {
    send(
        dev,
        &[ulanzi_proto::unlock(), ulanzi_proto::request_device_info()],
    )
    .await?;
    let end = Instant::now() + Duration::from_secs(2);
    while let Ok(Ok(buf)) = tokio::time::timeout_at(end.into(), dev.recv()).await {
        match ulanzi_proto::parse_report(&buf) {
            ulanzi_proto::DeviceReport::DeviceInfo(json) => println!("device info: {json}"),
            other => println!("other report: {other:?}"),
        }
    }
    Ok(())
}

async fn omit(dev: &mut Hidraw) -> Result<(), String> {
    send(
        dev,
        &page(&(0..13).map(|i| (i, solid([200, 40, 40]))).collect::<Vec<_>>()),
    )
    .await?;
    wait_enter("all keys should be red now");
    send(dev, &page(&[(0, solid([40, 80, 220]))])).await?;
    wait_enter("key 1 should be blue: are the other keys still red, or blank?");
    Ok(())
}

async fn modes(dev: &mut Hidraw) -> Result<(), String> {
    let mut fonts = Fonts::system();
    send(dev, &[ulanzi_proto::unlock(), brightness(60)]).await?;
    send(dev, &page(&[(13, pattern(458, 196, &mut fonts))])).await?;
    let tests: [(&str, ScreenMode, &str); 7] = [
        ("1 (picture only, the one we want)", ScreenMode::Image, ""),
        ("2 (clock)", ScreenMode::Clock, ""),
        (
            "3 (clock, date and weekday)",
            ScreenMode::ClockDateWeekday,
            "2026-10-22 Thu",
        ),
        ("4 (clock and weekday)", ScreenMode::ClockWeekday, "Thu"),
        ("5 (clock and date)", ScreenMode::ClockDate, "2026-10-22"),
        ("6 (large clock)", ScreenMode::ClockLarge, ""),
        ("7 (picture only again, after the clocks)", ScreenMode::Image, ""),
    ];
    for (name, mode, date) in tests {
        let w = SmallWindow {
            mode,
            cpu: 11,
            mem: 22,
            time: "12:34:56".into(),
            gpu: 33,
            clock_24h: true,
            date: date.into(),
        };
        send(dev, &[small_window(&w)]).await?;
        wait_enter(&format!(
            "\nTEST {name}: what is on the wide screen? Then press Enter."
        ));
    }
    println!("\nDone. All mode tests finished.");
    Ok(())
}

/// Send, then watch the deck for `secs`: Err if it dropped off USB (read/write error).
async fn survives(dev: &mut Hidraw, packets: &[ulanzi_proto::Packet], secs: u64) -> bool {
    if dev.send(packets).await.is_err() {
        return false;
    }
    let end = Instant::now() + Duration::from_secs(secs);
    loop {
        let keepalive = SmallWindow {
            mode: ScreenMode::Image,
            cpu: 0,
            mem: 0,
            time: "00:00:00".into(),
            gpu: 0,
            clock_24h: true,
            date: String::new(),
        };
        if dev.send(&[small_window(&keepalive)]).await.is_err() {
            return false;
        }
        if let Ok(Err(_)) = tokio::time::timeout(Duration::from_millis(500), dev.recv()).await {
            return false;
        }
        if Instant::now() >= end {
            return true;
        }
    }
}

/// A PNG of random noise: about as large as a photo key image.
fn noise(w: u32, h: u32, seed: u32) -> Vec<u8> {
    let mut x = seed.wrapping_mul(2_654_435_761).max(1);
    let img = image::RgbaImage::from_fn(w, h, |_, _| {
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        image::Rgba([x as u8, (x >> 8) as u8, (x >> 16) as u8, 255])
    });
    let mut out = Vec::new();
    let _ =
        image::DynamicImage::ImageRgba8(img).write_to(&mut Cursor::new(&mut out), image::ImageFormat::Png);
    out
}

async fn crash(dev: &mut Hidraw) -> Result<(), String> {
    send(dev, &[ulanzi_proto::unlock(), brightness(60)]).await?;
    // Sizes like the real page: three ~60 KB photos, the rest small, plus a ~60 KB wide screen.
    let photo = |i: u8| noise(110, 110, i as u32 + 1);
    let page_keys = |seed: u8| -> Vec<(u8, Vec<u8>)> {
        let mut k: Vec<(u8, Vec<u8>)> = (0..13)
            .map(|i| {
                (
                    i,
                    if [4u8, 9, 12].contains(&i) {
                        photo(i + seed)
                    } else {
                        solid([30, 60 + i * 10, 120])
                    },
                )
            })
            .collect();
        k.push((13, noise(150, 100, seed as u32 + 99)));
        k
    };
    let mut steps: Vec<(String, Vec<ulanzi_proto::Packet>, u64)> =
        vec![("realistic full page".into(), page(&page_keys(0)), 2)];
    for n in 0..6u32 {
        let p = partial_packets(&[(6, noise(110, 110, 500 + n)), (13, noise(150, 100, 600 + n))]);
        steps.push((format!("partial #{n}, back to back"), p, 0));
    }
    for n in 0..4u8 {
        steps.push((
            format!("full page #{n}, back to back"),
            page(&page_keys(n * 13)),
            0,
        ));
    }
    steps.push(("final page, watched 8 s".into(), page(&page_keys(77)), 8));
    for (name, packets, wait_after) in steps {
        let kb = packets.len();
        let ok = survives(dev, &packets, wait_after).await;
        println!(
            "{name:<40} {kb:>4} KB  {}",
            if ok {
                "survived"
            } else {
                "DECK REBOOTED (the previous wait was too short)"
            }
        );
        if !ok {
            return Ok(());
        }
    }
    Ok(())
}
