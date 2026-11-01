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
    /// Does the deck play a GIF by itself, how fast can one key change, and how late does a press
    /// show on the key?
    Gif,
    /// How much GIF can the deck take: 13 at once, large files, labels over a GIF?
    GifLimits,
    /// Throwaway effects: Fire on every key, then hard cuts against fades on one key.
    Fade,
    /// Is Fire's question mark about the file, or about how much the deck holds at once?
    FireLimit,
    /// Ten versions of Fire on ten keys: which ones does the deck play?
    FireVariants,
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
        Probe::Gif => gif(&mut dev).await,
        Probe::GifLimits => gif_limits(&mut dev).await,
        Probe::Fade => fade(&mut dev).await,
        Probe::FireLimit => fire_limit(&mut dev).await,
        Probe::FireVariants => fire_variants(&mut dev).await,
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

/// A 196×196 looping GIF: a white bar crossing a coloured key, twelve frames of 100 ms.
fn test_gif(rgb: [u8; 3]) -> Vec<u8> {
    use image::codecs::gif::{GifEncoder, Repeat};
    use image::{Delay, Frame, Rgba, RgbaImage};
    let mut out = Vec::new();
    {
        let mut enc = GifEncoder::new(&mut out);
        enc.set_repeat(Repeat::Infinite).expect("gif");
        for f in 0..12u32 {
            let img = RgbaImage::from_fn(196, 196, |x, _| {
                if x / 16 == f {
                    Rgba([255, 255, 255, 255])
                } else {
                    Rgba([rgb[0], rgb[1], rgb[2], 255])
                }
            });
            let delay = Delay::from_numer_denom_ms(100, 1);
            enc.encode_frame(Frame::from_parts(img, 0, 0, delay))
                .expect("gif");
        }
    }
    out
}

/// A 196×196 looping GIF of a thick white bar crossing random coloured squares `block` pixels wide.
/// Noise compresses badly, so smaller blocks and more frames make a larger file; the bar shows
/// whether it plays.
fn noise_gif(block: u32, frames: u32) -> Vec<u8> {
    use image::codecs::gif::{GifEncoder, Repeat};
    use image::{Delay, Frame, Rgba, RgbaImage};
    let mut seed = 0x2545_f491u32;
    let mut next = move || {
        seed ^= seed << 13;
        seed ^= seed >> 17;
        seed ^= seed << 5;
        seed
    };
    let mut out = Vec::new();
    {
        let mut enc = GifEncoder::new(&mut out);
        enc.set_repeat(Repeat::Infinite).expect("gif");
        for f in 0..frames {
            let cells: Vec<u32> = (0..196 * 196).map(|_| next()).collect();
            let bar = 160 * f / frames;
            let img = RgbaImage::from_fn(196, 196, |x, y| {
                let v = cells[((y / block) * 196 + x / block) as usize].to_le_bytes();
                if x >= bar && x < bar + 36 {
                    Rgba([255, 255, 255, 255])
                } else {
                    Rgba([v[0], v[1], v[2], 255])
                }
            });
            let delay = Delay::from_numer_denom_ms(100, 1);
            enc.encode_frame(Frame::from_parts(img, 0, 0, delay))
                .expect("gif");
        }
    }
    out
}

/// Keys 0..13 each playing the GIF `gif(key)`, with `label` under each when given.
fn gif_page(
    gif: impl Fn(u8) -> Vec<u8>,
    label: Option<&str>,
) -> (BTreeMap<u8, ButtonEntry>, Vec<ButtonImage>) {
    let mut entries = BTreeMap::new();
    let mut images = Vec::new();
    for i in 0..13u8 {
        let name = format!("g{i}_{}.gif", unique());
        entries.insert(
            i,
            ButtonEntry {
                state: 0,
                text: label.map(String::from),
                icon: Some(format!("icons/{name}")),
            },
        );
        images.push(ButtonImage { name, png: gif(i) });
    }
    (entries, images)
}

async fn gif_limits(dev: &mut Hidraw) -> Result<(), String> {
    wait_enter("Look at the deck. Press Enter for the first test.");
    send(dev, &[ulanzi_proto::unlock()]).await?;
    let grey = page(&(0..13).map(|i| (i, solid([40, 40, 40]))).collect::<Vec<_>>());

    // Same file size with different frame counts, so a limit can be pinned on one or the other.
    println!("TEST 1 of 2: one key at a time. Answer each with: bar moves / frozen / blank / grey");
    for (block, frames) in [(4, 12), (4, 24), (4, 40), (2, 24), (2, 48), (32, 120), (32, 240)] {
        if !survives(dev, &grey, 2).await {
            println!("DECK REBOOTED");
            return Ok(());
        }
        let name = format!("big_{}.gif", unique());
        let mut entries = BTreeMap::new();
        entries.insert(
            0u8,
            ButtonEntry {
                state: 0,
                text: None,
                icon: Some(format!("icons/{name}")),
            },
        );
        let images = [ButtonImage {
            name,
            png: noise_gif(block, frames),
        }];
        let kb = images[0].png.len() / 1024;
        println!("{kb} KB, {frames} frames: sending");
        let t = Instant::now();
        let zip = zip_for(&entries, &images);
        if !survives(dev, &file_packets(Command::PartialUpdate, &zip), 4).await {
            println!("{kb} KB: DECK REBOOTED");
            return Ok(());
        }
        println!("  sent and waited in {:?}", t.elapsed());
        wait_enter(&format!("{kb} KB, {frames} frames: what does key 1 show?"));
    }

    let colour = |i: u8| [40 + i * 15, 200 - i * 12, 90 + i * 10];
    let (entries, images) = gif_page(|i| test_gif(colour(i)), Some("Label"));
    let zip = zip_for(&entries, &images);
    println!("TEST 2 of 2: the small GIFs with the deck's own label");
    if !survives(dev, &file_packets(Command::SetButtons, &zip), 6).await {
        println!("DECK REBOOTED");
        return Ok(());
    }
    wait_enter("Does \"Label\" show on the keys? Does the bar keep moving? Then press Enter to finish");
    Ok(())
}

/// One frame of a white bar crossing a dark key, as PNG.
fn bar_frame(f: u32, frames: u32) -> Vec<u8> {
    let bar = 196 * (f % frames) / frames;
    let img = image::RgbaImage::from_fn(196, 196, |x, _| {
        if x >= bar && x < bar + 16 {
            image::Rgba([255, 255, 255, 255])
        } else {
            image::Rgba([20, 40, 80, 255])
        }
    });
    let mut out = Vec::new();
    let _ =
        image::DynamicImage::ImageRgba8(img).write_to(&mut Cursor::new(&mut out), image::ImageFormat::Png);
    out
}

async fn gif(dev: &mut Hidraw) -> Result<(), String> {
    let window = SmallWindow {
        mode: ScreenMode::Clock,
        cpu: 0,
        mem: 0,
        time: "12:00:00".into(),
        gpu: 0,
        clock_24h: true,
        date: String::new(),
    };
    send(dev, &[ulanzi_proto::unlock(), small_window(&window)]).await?;
    send(
        dev,
        &page(&(0..13).map(|i| (i, solid([40, 40, 40]))).collect::<Vec<_>>()),
    )
    .await?;
    wait_enter("TEST 1 of 4: all 13 keys should be dark grey.");

    // A GIF in a whole-page upload, in a partial update, and under a .png name.
    let mut entries = BTreeMap::new();
    let mut images = Vec::new();
    for (i, name, rgb) in [
        (0u8, "gif_page.gif", [200, 40, 40]),
        (1, "gif_named.png", [40, 160, 60]),
    ] {
        entries.insert(
            i,
            ButtonEntry {
                state: 0,
                text: None,
                icon: Some(format!("icons/{name}")),
            },
        );
        images.push(ButtonImage {
            name: name.into(),
            png: test_gif(rgb),
        });
    }
    let bytes: usize = images.iter().map(|i| i.png.len()).sum();
    println!("two GIFs, {bytes} bytes");
    send(
        dev,
        &file_packets(Command::SetButtons, &zip_for(&entries, &images)),
    )
    .await?;
    let mut partial = BTreeMap::new();
    partial.insert(
        2u8,
        ButtonEntry {
            state: 0,
            text: None,
            icon: Some("icons/gif_partial.gif".into()),
        },
    );
    let img = [ButtonImage {
        name: "gif_partial.gif".into(),
        png: test_gif([40, 80, 200]),
    }];
    send(
        dev,
        &file_packets(Command::PartialUpdate, &zip_for(&partial, &img)),
    )
    .await?;
    wait_enter(
        "TEST 2 of 4: look at the first three keys (red, green, blue). Note for EACH one: \
         a moving white bar, a still picture, or blank. Then press Enter",
    );

    send(
        dev,
        &page(&(0..13).map(|i| (i, solid([40, 40, 40]))).collect::<Vec<_>>()),
    )
    .await?;
    wait_enter("TEST 3 of 4: key 1 will show a white bar crossing it, as fast as the link allows.");
    let frames: Vec<Vec<u8>> = (0..24).map(|f| bar_frame(f, 24)).collect();
    println!("frame size {} bytes", frames[0].len());
    let t = Instant::now();
    for (n, f) in frames.iter().cycle().take(72).enumerate() {
        send(dev, &partial_packets(&[(0, f.clone())])).await?;
        if n % 24 == 23 {
            println!("{} frames in {:?}", n + 1, t.elapsed());
        }
    }
    println!(
        "72 frames in {:?}: {:.1} frames a second written",
        t.elapsed(),
        72.0 / t.elapsed().as_secs_f32()
    );
    wait_enter("Did the bar glide, stutter, or freeze? Did any key blink or go black? Then press Enter");

    send(
        dev,
        &page(&(0..13).map(|i| (i, solid([40, 40, 40]))).collect::<Vec<_>>()),
    )
    .await?;
    println!(
        "TEST 4 of 4: press and release key 1 several times. It turns white while held. \
         Note how late that feels. Press Enter here when done."
    );
    let mut stdin = tokio::io::BufReader::new(tokio::io::stdin()).lines();
    loop {
        tokio::select! {
            _ = stdin.next_line() => break,
            r = dev.recv() => {
                let Ok(buf) = r else { break };
                let Some(e) = parse_input(&buf) else { continue };
                if !matches!(e.control, ulanzi_proto::Control::Key(_)) {
                    continue;
                }
                let colour = match e.kind {
                    ulanzi_proto::InputKind::Press => [255, 255, 255],
                    ulanzi_proto::InputKind::Release => [40, 40, 40],
                    _ => continue,
                };
                let t = Instant::now();
                send(dev, &partial_packets(&[(e.raw_index, solid(colour))])).await?;
                println!("{:?} written in {:?}", e.kind, t.elapsed());
            }
        }
    }
    Ok(())
}

/// Throwaway effects for `probe fade`. Each pixel is worked out from where it is and how far through
/// the loop it is, so every loop closes on itself.
#[derive(Clone, Copy, PartialEq, Debug)]
enum Fx {
    Fire,
    Pulse,
    Chase,
}

const FX_FRAMES: u32 = 24;
const FX_FADE_FRAMES: u32 = 6;
const FX_DELAY_MS: u32 = 66;

fn fx_pixel(fx: Fx, x: u32, y: u32, t: f32) -> [f32; 3] {
    use std::f32::consts::TAU;
    let (fx_x, fx_y) = (x as f32, y as f32);
    let (l, r, top, b) = (fx_x, 195.0 - fx_x, fx_y, 195.0 - fx_y);
    let d = l.min(r).min(top).min(b);
    // How far round the border this pixel is, 0 to 1.
    let u = if d == top {
        fx_x / 196.0 / 4.0
    } else if d == r {
        (1.0 + fx_y / 196.0) / 4.0
    } else if d == b {
        (2.0 + (195.0 - fx_x) / 196.0) / 4.0
    } else {
        (3.0 + (195.0 - fx_y) / 196.0) / 4.0
    };
    let icon = x.abs_diff(98) < 28 && y.abs_diff(98) < 28;
    let bg = if icon { [0.9, 0.9, 0.9] } else { [0.07, 0.07, 0.09] };
    let (colour, a) = match fx {
        Fx::Fire => {
            let h = 0.55
                + 0.25 * (TAU * (5.0 * u + 2.0 * t)).sin()
                + 0.15 * (TAU * (11.0 * u - 3.0 * t) + 1.3).sin()
                + 0.10 * (TAU * (23.0 * u + 5.0 * t) + 0.7).sin();
            let v = (1.0 - d / (46.0 * h.max(0.1))).clamp(0.0, 1.0);
            let ramp = [
                (v * 2.2).min(1.0),
                (v * 1.6 - 0.35).clamp(0.0, 1.0),
                (v * 2.0 - 1.4).clamp(0.0, 1.0),
            ];
            (ramp, (v * 2.2).min(1.0))
        }
        Fx::Pulse => {
            let v = (1.0 - d / 26.0).clamp(0.0, 1.0);
            ([0.31, 0.71, 1.0], v * v * (0.55 + 0.45 * (TAU * t).sin()))
        }
        Fx::Chase => {
            let v = (1.0 - d / 20.0).clamp(0.0, 1.0);
            let trail = (1.0 - (t - u).rem_euclid(1.0) / 0.35).clamp(0.0, 1.0);
            ([0.31, 1.0, 0.67], v * (0.12 + 0.88 * trail * trail))
        }
    };
    [0, 1, 2].map(|i| bg[i] * (1.0 - a) + colour[i] * a)
}

fn fx_image(pixel: impl Fn(u32, u32) -> [f32; 3]) -> image::RgbaImage {
    image::RgbaImage::from_fn(196, 196, |x, y| {
        let c = pixel(x, y).map(|v| (v * 255.0) as u8);
        image::Rgba([c[0], c[1], c[2], 255])
    })
}

fn fx_gif(frames: Vec<image::RgbaImage>, repeat: bool) -> Vec<u8> {
    use image::codecs::gif::{GifEncoder, Repeat};
    use image::{Delay, Frame};
    let mut out = Vec::new();
    {
        let mut enc = GifEncoder::new_with_speed(&mut out, 10);
        let times = if repeat { Repeat::Infinite } else { Repeat::Finite(0) };
        enc.set_repeat(times).expect("gif");
        for img in frames {
            let delay = Delay::from_numer_denom_ms(FX_DELAY_MS, 1);
            enc.encode_frame(Frame::from_parts(img, 0, 0, delay))
                .expect("gif");
        }
    }
    out
}

fn fx_loop(fx: Fx) -> Vec<u8> {
    if fx == Fx::Fire {
        return fire_gif(FX_FRAMES, FX_DELAY_MS, 10, 8.0);
    }
    let frames = (0..FX_FRAMES)
        .map(|f| fx_image(|x, y| fx_pixel(fx, x, y, f as f32 / FX_FRAMES as f32)))
        .collect();
    fx_gif(frames, true)
}

/// A one-shot blend from `a` into the last frames of `b`, so its loop carries straight on.
fn fx_fade(a: Fx, b: Fx) -> Vec<u8> {
    let frames = (0..FX_FADE_FRAMES)
        .map(|f| {
            let mix = (f + 1) as f32 / FX_FADE_FRAMES as f32;
            let ta = f as f32 / FX_FRAMES as f32;
            let tb = (FX_FRAMES - FX_FADE_FRAMES + f) as f32 / FX_FRAMES as f32;
            fx_image(|x, y| {
                let (p, q) = (fx_pixel(a, x, y, ta), fx_pixel(b, x, y, tb));
                [0, 1, 2].map(|i| p[i] * (1.0 - mix) + q[i] * mix)
            })
        })
        .collect();
    fx_gif(frames, false)
}

fn fx_key(gif: &[u8]) -> Vec<ulanzi_proto::Packet> {
    let name = format!("fx_{}.gif", unique());
    let mut entries = BTreeMap::new();
    entries.insert(
        0u8,
        ButtonEntry {
            state: 0,
            text: None,
            icon: Some(format!("icons/{name}")),
        },
    );
    let images = [ButtonImage {
        name,
        png: gif.to_vec(),
    }];
    file_packets(Command::PartialUpdate, &zip_for(&entries, &images))
}

async fn fade(dev: &mut Hidraw) -> Result<(), String> {
    use tokio::time::{Instant as At, sleep_until};
    use ulanzi_proto::InputKind;
    let t = Instant::now();
    use Fx::{Chase, Fire, Pulse};
    let loops = [Fire, Pulse, Chase].map(|fx| (fx, fx_loop(fx)));
    println!("three loops made in {:?}", t.elapsed());
    for (fx, g) in &loops {
        println!("  {fx:?}: {} KB", g.len() / 1024);
    }
    let t = Instant::now();
    let fades = [(Fire, Pulse), (Pulse, Chase), (Chase, Pulse), (Pulse, Fire)]
        .map(|(a, b)| ((a, b), fx_fade(a, b)));
    println!("four fades made in {:?}, about {} KB each", t.elapsed(), fades[0].1.len() / 1024);
    let looped = |fx: Fx| &loops.iter().find(|l| l.0 == fx).expect("loop").1;
    let faded = |a: Fx, b: Fx| &fades.iter().find(|f| f.0 == (a, b)).expect("fade").1;

    let mut stdin = tokio::io::BufReader::new(tokio::io::stdin()).lines();
    println!("Look at the deck. Press Enter for the first test.  [Enter]");
    let _ = stdin.next_line().await;
    send(dev, &[ulanzi_proto::unlock()]).await?;

    let fire = looped(Fire).clone();
    let (entries, images) = gif_page(|_| fire.clone(), None);
    let zip = zip_for(&entries, &images);
    println!("TEST 1 of 3: Fire on all 13 keys, {} KB in all", zip.len() / 1024);
    if !survives(dev, &file_packets(Command::SetButtons, &zip), 6).await {
        println!("DECK REBOOTED");
        return Ok(());
    }
    println!("Does it look like fire? Is any key choppy? Then press Enter  [Enter]");
    let _ = stdin.next_line().await;

    for (name, smooth) in [("TEST 2 of 3: hard cuts", false), ("TEST 3 of 3: fades", true)] {
        send(
            dev,
            &page(&(0..13).map(|i| (i, solid([18, 18, 22]))).collect::<Vec<_>>()),
        )
        .await?;
        send(dev, &fx_key(looped(Fire))).await?;
        println!(
            "{name}. Key 1 rests on Fire. Hold it for Pulse, let go for Chase, and press again to \
             go back. Do it about ten times, quick and slow, then press Enter  [Enter]"
        );
        let (mut now, mut resting) = (Fire, Fire);
        let mut pending: Option<(At, Fx)> = None;
        loop {
            let wake = pending.map_or_else(|| At::now() + Duration::from_secs(3600), |p| p.0);
            tokio::select! {
                _ = stdin.next_line() => break,
                _ = sleep_until(wake) => {
                    if let Some((_, fx)) = pending.take() {
                        send(dev, &fx_key(looped(fx))).await?;
                    }
                }
                r = dev.recv() => {
                    let Ok(buf) = r else { break };
                    let Some(e) = parse_input(&buf) else { continue };
                    if e.raw_index != 0 {
                        continue;
                    }
                    let next = match e.kind {
                        InputKind::Press => Pulse,
                        InputKind::Release => {
                            resting = if resting == Fire { Chase } else { Fire };
                            resting
                        }
                        _ => continue,
                    };
                    let t = Instant::now();
                    if smooth && faded_exists(now, next) {
                        send(dev, &fx_key(faded(now, next))).await?;
                        let ms = (FX_FADE_FRAMES * FX_DELAY_MS) as u64;
                        pending = Some((At::now() + Duration::from_millis(ms), next));
                    } else {
                        pending = None;
                        send(dev, &fx_key(looped(next))).await?;
                    }
                    println!("{:?} -> {next:?}, written in {:?}", e.kind, t.elapsed());
                    now = next;
                }
            }
        }
    }
    Ok(())
}

async fn fire_limit(dev: &mut Hidraw) -> Result<(), String> {
    let fire = fx_loop(Fx::Fire);
    let grey = page(&(0..13).map(|i| (i, solid([18, 18, 22]))).collect::<Vec<_>>());
    wait_enter("Look at the deck. Press Enter for the first test.");
    send(dev, &[ulanzi_proto::unlock()]).await?;
    if !survives(dev, &grey, 2).await {
        println!("DECK REBOOTED");
        return Ok(());
    }
    if !survives(dev, &fx_key(&fire), 5).await {
        println!("DECK REBOOTED");
        return Ok(());
    }
    wait_enter("TEST 1 of 5: Fire on key 1 alone. Moving fire, or a question mark?");
    for (test, keys) in [(2, 5u8), (3, 8), (4, 10), (5, 13)] {
        if !survives(dev, &grey, 2).await {
            println!("DECK REBOOTED");
            return Ok(());
        }
        let (mut entries, mut images) = gif_page(|_| fire.clone(), None);
        entries.retain(|i, _| *i < keys);
        images.truncate(keys as usize);
        let zip = zip_for(&entries, &images);
        println!("TEST {test} of 5: Fire on {keys} keys, {} KB in all", zip.len() / 1024);
        if !survives(dev, &file_packets(Command::SetButtons, &zip), 6).await {
            println!("DECK REBOOTED");
            return Ok(());
        }
        wait_enter("Moving fire on those keys, or question marks?");
    }
    Ok(())
}

/// Fire with `frames` frames of `ms` each, encoded at `speed`, keeping `levels` shades per channel.
fn fire_gif(frames: u32, ms: u32, speed: i32, levels: f32) -> Vec<u8> {
    use image::codecs::gif::{GifEncoder, Repeat};
    use image::{Delay, Frame};
    let mut out = Vec::new();
    {
        let mut enc = GifEncoder::new_with_speed(&mut out, speed);
        enc.set_repeat(Repeat::Infinite).expect("gif");
        for f in 0..frames {
            let img = fx_image(|x, y| {
                fx_pixel(Fx::Fire, x, y, f as f32 / frames as f32).map(|v| (v * levels).round() / levels)
            });
            let delay = Delay::from_numer_denom_ms(ms, 1);
            enc.encode_frame(Frame::from_parts(img, 0, 0, delay))
                .expect("gif");
        }
    }
    out
}

async fn fire_variants(dev: &mut Hidraw) -> Result<(), String> {
    let gifs = [
        ("Fire as before, 24 frames at 66 ms", fire_gif(24, 66, 10, 255.0)),
        ("24 frames at 100 ms", fire_gif(24, 100, 10, 255.0)),
        ("12 frames at 66 ms", fire_gif(12, 66, 10, 255.0)),
        ("12 frames at 100 ms", fire_gif(12, 100, 10, 255.0)),
        ("24 frames at 66 ms, fewer colours", fire_gif(24, 66, 10, 8.0)),
        ("24 frames at 66 ms, slowest encoder", fire_gif(24, 66, 1, 255.0)),
        ("16 frames at 100 ms", fire_gif(16, 100, 10, 255.0)),
        ("noise that played before", noise_gif(4, 24)),
        ("8 frames at 100 ms", fire_gif(8, 100, 10, 255.0)),
        ("Pulse that played before", fx_loop(Fx::Pulse)),
    ];
    let (mut entries, mut images) = gif_page(|i| gifs.get(i as usize).map_or(Vec::new(), |g| g.1.clone()), None);
    entries.retain(|i, _| (*i as usize) < gifs.len());
    images.truncate(gifs.len());
    let grey = page(&(0..13).map(|i| (i, solid([18, 18, 22]))).collect::<Vec<_>>());
    for (n, (name, g)) in gifs.iter().enumerate() {
        println!("key {:>2}: {name}, {} KB", n + 1, g.len() / 1024);
    }
    wait_enter("Look at the deck. Press Enter to send them.");
    send(dev, &[ulanzi_proto::unlock()]).await?;
    if !survives(dev, &grey, 2).await {
        println!("DECK REBOOTED");
        return Ok(());
    }
    let zip = zip_for(&entries, &images);
    println!("{} KB in all", zip.len() / 1024);
    if !survives(dev, &file_packets(Command::SetButtons, &zip), 8).await {
        println!("DECK REBOOTED");
        return Ok(());
    }
    wait_enter("Top row is keys 1-5, next row 6-10. Which keys show a question mark?");
    Ok(())
}

fn faded_exists(a: Fx, b: Fx) -> bool {
    use Fx::{Chase, Fire, Pulse};
    matches!(
        (a, b),
        (Fire, Pulse) | (Pulse, Chase) | (Chase, Pulse) | (Pulse, Fire)
    )
}
