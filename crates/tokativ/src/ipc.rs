//! The control socket at `$XDG_RUNTIME_DIR/tokativ.sock`. Each connection carries one command as
//! a line of text and gets one line back, `ok` or `error: <reason>`.

use std::io::{BufRead as _, BufReader, ErrorKind, Write as _};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::Path;
use std::time::Duration;
use std::{fs, thread};

use anyhow::{Context as _, Result, bail};
use tokativ_config::Model;

/// What a client can ask the daemon to do.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Command {
    Toggle,
    Start,
    Stop,
    Confirm,
    Cancel,
    Model(Model),
    Settings,
    Quit,
    Ping,
}

impl Command {
    pub fn parse(line: &str) -> Result<Self> {
        let mut words = line.split_whitespace();
        let command = match (words.next(), words.next()) {
            (Some("toggle"), None) => Self::Toggle,
            (Some("start"), None) => Self::Start,
            (Some("stop"), None) => Self::Stop,
            (Some("confirm"), None) => Self::Confirm,
            (Some("cancel"), None) => Self::Cancel,
            (Some("settings"), None) => Self::Settings,
            (Some("quit"), None) => Self::Quit,
            (Some("ping"), None) => Self::Ping,
            (Some("model"), Some(model)) => Self::Model(
                Model::from_id(model).with_context(|| format!("unknown model {model:?}"))?,
            ),
            _ => bail!("unknown command {line:?}"),
        };
        if words.next().is_some() {
            bail!("trailing arguments in {line:?}");
        }
        Ok(command)
    }

    pub fn line(self) -> String {
        match self {
            Self::Toggle => "toggle".into(),
            Self::Start => "start".into(),
            Self::Stop => "stop".into(),
            Self::Confirm => "confirm".into(),
            Self::Cancel => "cancel".into(),
            Self::Model(model) => format!("model {}", model.id()),
            Self::Settings => "settings".into(),
            Self::Quit => "quit".into(),
            Self::Ping => "ping".into(),
        }
    }
}

/// Binds the socket and hands every command it receives to `deliver` on a background thread.
/// Fails when another daemon already answers on the socket. A socket file left by a daemon that
/// died is removed and rebound.
pub fn serve(path: &Path, deliver: impl Fn(Command) + Send + 'static) -> Result<()> {
    if UnixStream::connect(path).is_ok() {
        bail!(
            "another tokativ daemon is already listening on {}",
            path.display()
        );
    }
    match fs::remove_file(path) {
        Err(error) if error.kind() != ErrorKind::NotFound => {
            return Err(error).with_context(|| format!("removing stale {}", path.display()));
        }
        _ => {}
    }
    let listener =
        UnixListener::bind(path).with_context(|| format!("binding {}", path.display()))?;
    thread::Builder::new()
        .name("ipc".into())
        .spawn(move || {
            for stream in listener.incoming() {
                match stream {
                    Ok(stream) => {
                        if let Err(error) = answer(stream, &deliver) {
                            log::warn!("ipc: {error:#}");
                        }
                    }
                    Err(error) => log::warn!("ipc: accept failed: {error}"),
                }
            }
        })
        .context("spawning the ipc thread")?;
    Ok(())
}

fn answer(stream: UnixStream, deliver: &impl Fn(Command)) -> Result<()> {
    stream.set_read_timeout(Some(Duration::from_secs(2)))?;
    let mut line = String::new();
    BufReader::new(&stream).read_line(&mut line)?;
    let reply = match Command::parse(line.trim()) {
        Ok(command) => {
            deliver(command);
            "ok".to_string()
        }
        Err(error) => format!("error: {error:#}"),
    };
    writeln!(&stream, "{reply}")?;
    Ok(())
}

/// Sends one command to a running daemon and returns its reply. Errors with
/// [`ErrorKind::NotFound`] or [`ErrorKind::ConnectionRefused`] inside when no daemon listens.
pub fn send(path: &Path, command: Command) -> std::io::Result<String> {
    let stream = UnixStream::connect(path)?;
    stream.set_read_timeout(Some(Duration::from_secs(5)))?;
    writeln!(&stream, "{}", command.line())?;
    let mut reply = String::new();
    BufReader::new(&stream).read_line(&mut reply)?;
    Ok(reply.trim().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn commands_round_trip() {
        for command in [
            Command::Toggle,
            Command::Start,
            Command::Stop,
            Command::Confirm,
            Command::Cancel,
            Command::Model(Model::Parakeet),
            Command::Model(Model::Gigaam),
            Command::Settings,
            Command::Quit,
            Command::Ping,
        ] {
            assert_eq!(Command::parse(&command.line()).unwrap(), command);
        }
        assert!(Command::parse("model whisper").is_err());
        assert!(Command::parse("toggle now").is_err());
    }
}
