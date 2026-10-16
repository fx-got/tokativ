//! Finds which app has keyboard focus, so the paste chord can follow terminal conventions.

use std::io::{BufRead as _, BufReader, Write as _};
use std::os::unix::net::UnixStream;
use std::time::Duration;

/// The app id of niri's focused window, or `None` off niri or when nothing has focus.
pub fn focused_app_id() -> Option<String> {
    let path = std::env::var_os("NIRI_SOCKET")?;
    let stream = UnixStream::connect(path).ok()?;
    stream
        .set_read_timeout(Some(Duration::from_millis(500)))
        .ok()?;
    (&stream).write_all(b"\"FocusedWindow\"\n").ok()?;
    let mut line = String::new();
    BufReader::new(&stream).read_line(&mut line).ok()?;
    let reply: serde_json::Value = serde_json::from_str(&line).ok()?;
    reply
        .pointer("/Ok/FocusedWindow/app_id")?
        .as_str()
        .map(str::to_owned)
}

/// The first running input method daemon that would already hold the seat's input method, by
/// process name. Binding a second one on niri 26.04 detaches the first, so this is checked
/// before ever asking for the protocol.
pub fn other_input_method() -> Option<String> {
    const KNOWN: [&str; 5] = ["fcitx5", "fcitx", "ibus-daemon", "kime", "maliit-server"];
    let processes = std::fs::read_dir("/proc").ok()?;
    processes.flatten().find_map(|entry| {
        let name = std::fs::read_to_string(entry.path().join("comm")).ok()?;
        let name = name.trim();
        KNOWN.contains(&name).then(|| name.to_owned())
    })
}
