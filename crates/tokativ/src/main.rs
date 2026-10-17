//! Tokativ: local dictation for Wayland. Run without arguments, or with `daemon`, it starts the
//! background process. Every other subcommand is sent to that process over its socket.

mod assets;
mod autostart;
mod daemon;
mod ipc;
mod tray;

use std::io::ErrorKind;
use std::os::unix::process::CommandExt as _;
use std::process::{ExitCode, Stdio};
use std::time::{Duration, Instant};

use anyhow::{Context as _, Result, bail};
use clap::{Parser, Subcommand};
use tokativ_config::{Config, Model, SANDBOX_VARIABLE, socket_path};

use crate::ipc::Command;

/// How long a command waits for a daemon it had to start.
const STARTUP: Duration = Duration::from_secs(10);

#[derive(Parser)]
#[command(name = "tokativ", version, about = "Local dictation for Wayland")]
struct Cli {
    #[command(subcommand)]
    command: Option<Verb>,
}

#[derive(Subcommand)]
enum Verb {
    /// Run the background daemon
    Daemon,
    /// Run a throwaway daemon in the foreground, set up as a new user would meet it
    Sandbox {
        /// Keep the sandbox's settings and models from the last run instead of starting over
        #[arg(long)]
        keep: bool,
        /// Link the models already downloaded, so setup can be tried without downloading them
        #[arg(long)]
        with_models: bool,
        /// Open the setup window on this step
        #[arg(long, value_parser = ["models", "microphone", "try", "hotkey"])]
        step: Option<String>,
    },
    /// Start dictating, or confirm the running session
    Toggle,
    /// Start dictating
    Start,
    /// Stop dictating and insert what is left
    Stop,
    /// Insert what is left and close the pill
    Confirm,
    /// Discard what is not inserted yet and close the pill
    Cancel,
    /// Switch the model sessions dictate with
    Model {
        #[arg(value_parser = Model::ALL.map(Model::id))]
        model: String,
    },
    /// Open the settings window
    Settings,
    /// Stop the daemon
    Quit,
}

fn main() -> ExitCode {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or(
        "info,i18n=off,naga=warn,wgpu=warn,wgpu_hal=error,ort=warn,transcribe_rs=warn,zbus=warn,tracing=warn",
    ))
    .init();
    let cli = Cli::parse();
    let result = match cli.command {
        None | Some(Verb::Daemon) => Config::load().and_then(daemon::run),
        Some(Verb::Sandbox {
            keep,
            with_models,
            step,
        }) => sandbox(keep, with_models, step),
        Some(verb) => send(command(verb)),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("tokativ: {error:#}");
            ExitCode::FAILURE
        }
    }
}

fn command(verb: Verb) -> Command {
    match verb {
        Verb::Daemon | Verb::Sandbox { .. } => unreachable!("handled in main"),
        Verb::Toggle => Command::Toggle,
        Verb::Start => Command::Start,
        Verb::Stop => Command::Stop,
        Verb::Confirm => Command::Confirm,
        Verb::Cancel => Command::Cancel,
        Verb::Model { model } => {
            Command::Model(Model::from_id(&model).expect("clap checked the model"))
        }
        Verb::Settings => Command::Settings,
        Verb::Quit => Command::Quit,
    }
}

/// Runs a daemon against a folder of its own, fresh unless `keep` is set, so the first-run flow
/// can be tried again and again without touching the real install. Commands reach it with the
/// sandbox variable set to the same folder.
fn sandbox(keep: bool, with_models: bool, step: Option<String>) -> Result<()> {
    let real_models = tokativ_config::models_dir();
    let folder = tokativ_config::sandbox().unwrap_or_else(|| {
        dirs::state_dir()
            .unwrap_or_else(std::env::temp_dir)
            .join("tokativ/sandbox")
    });
    if !keep {
        match std::fs::remove_dir_all(&folder) {
            Err(error) if error.kind() != ErrorKind::NotFound => {
                return Err(error).with_context(|| format!("clearing {}", folder.display()));
            }
            _ => {}
        }
    }
    std::fs::create_dir_all(&folder).with_context(|| format!("creating {}", folder.display()))?;
    // SAFETY: nothing else runs yet, so no other thread can be reading the environment.
    unsafe {
        std::env::set_var(SANDBOX_VARIABLE, &folder);
        if let Some(step) = &step {
            std::env::set_var(tokativ_ui::setup::STEP_VARIABLE, step);
        }
    }
    if with_models {
        link_models(&real_models, &tokativ_config::models_dir())?;
    }
    eprintln!(
        "tokativ: sandbox in {0}\n  send commands with: {SANDBOX_VARIABLE}={0} tokativ toggle",
        folder.display()
    );
    Config::load().and_then(daemon::run)
}

/// Links every downloaded model file into the sandbox one by one. Folders of links rather than a
/// link to the folder, so deleting a model in the sandbox removes only the links.
fn link_models(from: &std::path::Path, to: &std::path::Path) -> Result<()> {
    for model in Model::ALL {
        let source = from.join(model.id());
        let Ok(files) = std::fs::read_dir(&source) else {
            continue;
        };
        let target = to.join(model.id());
        std::fs::create_dir_all(&target)
            .with_context(|| format!("creating {}", target.display()))?;
        for file in files.flatten() {
            let link = target.join(file.file_name());
            if !link.exists() {
                std::os::unix::fs::symlink(file.path(), &link)
                    .with_context(|| format!("linking {}", link.display()))?;
            }
        }
    }
    Ok(())
}

/// Sends `command` to the daemon. Commands that open something start the daemon first when it
/// is not running, so a keybind works even before the service is up.
fn send(command: Command) -> Result<()> {
    let path = socket_path();
    let reply = match ipc::send(&path, command) {
        Ok(reply) => reply,
        Err(error) if absent(&error) => {
            if !matches!(
                command,
                Command::Toggle | Command::Start | Command::Settings
            ) {
                bail!("the daemon is not running");
            }
            spawn_daemon()?;
            let deadline = Instant::now() + STARTUP;
            loop {
                match ipc::send(&path, command) {
                    Ok(reply) => break reply,
                    Err(error) if absent(&error) && Instant::now() < deadline => {
                        std::thread::sleep(Duration::from_millis(100));
                    }
                    Err(error) => return Err(error).context("the daemon did not come up"),
                }
            }
        }
        Err(error) => return Err(error).context("talking to the daemon"),
    };
    match reply.strip_prefix("error: ") {
        Some(error) => bail!("{error}"),
        None => Ok(()),
    }
}

fn absent(error: &std::io::Error) -> bool {
    matches!(
        error.kind(),
        ErrorKind::NotFound | ErrorKind::ConnectionRefused
    )
}

/// Starts the daemon through its systemd user service when there is one, so it runs where the
/// service manager can watch it. Otherwise starts it directly in its own process group, so it
/// outlives the compositor's spawn.
fn spawn_daemon() -> Result<()> {
    // A sandboxed command starts a sandboxed daemon, never the installed service.
    let sandboxed = tokativ_config::sandbox().is_some();
    if !sandboxed {
        let service = std::process::Command::new("systemctl")
            .args(["--user", "start", "tokativ.service"])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
        if service.is_ok_and(|status| status.success()) {
            return Ok(());
        }
    }
    let exe = std::env::current_exe().context("finding the running binary")?;
    std::process::Command::new(exe)
        .arg("daemon")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::inherit())
        .process_group(0)
        .spawn()
        .context("starting the daemon")?;
    Ok(())
}
