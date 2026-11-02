//! `pdsim`: run a Playdate game headless at simulated device speed.
//!
//! ```text
//! pdsim run examples/lua-stress --frames 600 --input script.json \
//!     --report out.json --trace out.trace.json
//! ```

use anyhow::{bail, Context, Result};
use clap::{Parser, Subcommand};
use pdsim_core::{trace, Calibration, FrameStats, Report};
use pdsim_device::InputScript;
use std::path::PathBuf;

#[derive(Parser)]
#[command(name = "pdsim", version, about = "Playdate performance simulator")]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Run a game headless and report device timings.
    Run(Box<RunArgs>),
    /// Print the built-in calibration profile as JSON.
    DefaultCalibration,
    /// Serve a live session over WebSocket for the web UI (`app/`, `npm run dev`).
    Serve {
        /// Game to load when a UI connects.
        game: Option<PathBuf>,
        /// Port to listen on.
        #[arg(long, default_value_t = 8765)]
        port: u16,
    },
}

#[derive(Parser)]
struct RunArgs {
    /// Game folder (with main.lua or Source/main.lua), or a pdex.elf.
    game: PathBuf,
    /// Frames to simulate.
    #[arg(long, default_value_t = 300)]
    frames: u64,
    /// Scripted input (JSON).
    #[arg(long)]
    input: Option<PathBuf>,
    /// Write a JSON report here.
    #[arg(long)]
    report: Option<PathBuf>,
    /// Write a Chrome/Perfetto trace here.
    #[arg(long)]
    trace: Option<PathBuf>,
    /// Write per-frame stats (JSON lines) here.
    #[arg(long)]
    frames_out: Option<PathBuf>,
    /// Save the last frame as a PNG.
    #[arg(long)]
    screenshot: Option<PathBuf>,
    /// Calibration profile (defaults to the built-in estimates).
    #[arg(long)]
    calibration: Option<PathBuf>,
    /// Folder for the game's saved data.
    #[arg(long)]
    data_dir: Option<PathBuf>,
    /// Playdate SDK (defaults to $PLAYDATE_SDK_PATH).
    #[arg(long)]
    sdk: Option<PathBuf>,
    /// Skip per-function attribution (faster).
    #[arg(long)]
    no_profile: bool,
    /// Exit with an error if more than this many frames go over budget.
    #[arg(long)]
    max_over_budget: Option<u64>,
    /// Exit with an error if the average frame work exceeds this (ms).
    #[arg(long)]
    max_avg_ms: Option<f64>,
    /// Print only the summary, not the game's console output.
    #[arg(long, short)]
    quiet: bool,
}

fn main() -> Result<()> {
    match Cli::parse().cmd {
        Cmd::Run(a) => run(*a),
        Cmd::Serve { game, port } => serve(game, port),
        Cmd::DefaultCalibration => {
            println!("{}", Calibration::default_estimates().to_json());
            Ok(())
        }
    }
}

fn run(a: RunArgs) -> Result<()> {
    let calibration = match &a.calibration {
        Some(p) => Calibration::load(p).with_context(|| format!("loading {}", p.display()))?,
        None => Calibration::default_estimates(),
    };
    let script = match &a.input {
        Some(p) => Some(
            InputScript::from_json(&std::fs::read_to_string(p)?)
                .with_context(|| format!("parsing {}", p.display()))?,
        ),
        None => None,
    };
    let cfg = pdsim_runner::RunConfig {
        calibration: calibration.clone(),
        data_dir: a.data_dir.clone(),
        sdk: a.sdk.clone().or_else(pdsim_device::Device::sdk_from_env),
        profile: !a.no_profile,
    };
    let mut game = pdsim_runner::load(&a.game, &cfg).map_err(|e| anyhow::anyhow!(e.0))?;
    if !a.quiet {
        eprintln!("{} ({})", game.describe(), calibration.name);
    }
    let mut frames: Vec<FrameStats> = Vec::with_capacity(a.frames as usize);
    let mut printed = 0;
    let mut failure = None;
    for _ in 0..a.frames {
        match game.frame(script.as_ref()) {
            Ok(f) => frames.push(f),
            Err(e) => {
                failure = Some(e.0);
                break;
            }
        }
        if !a.quiet {
            let d = game.device().borrow();
            for l in &d.log[printed.min(d.log.len())..] {
                println!("{l}");
            }
            printed = d.log.len();
        }
    }
    let dev = game.device().borrow();
    let report = Report::build(&calibration.name, calibration.estimated, &frames, &dev.hotspots);
    eprint!("{}", report.summary());
    if let Some(p) = &a.report {
        std::fs::write(p, serde_json::to_string_pretty(&report)?)?;
    }
    if let Some(p) = &a.trace {
        std::fs::write(p, serde_json::to_string(&trace::chrome_trace(&frames))?)?;
    }
    if let Some(p) = &a.frames_out {
        let mut s = String::new();
        for f in &frames {
            s.push_str(&serde_json::to_string(f)?);
            s.push('\n');
        }
        std::fs::write(p, s)?;
    }
    if let Some(p) = &a.screenshot {
        std::fs::write(
            p,
            dev.gfx
                .display()
                .encode_png()
                .map_err(|e| anyhow::anyhow!(e.to_string()))?,
        )?;
    }
    if let Some(e) = failure {
        bail!("game error after {} frames: {e}", frames.len());
    }
    if let Some(m) = a.max_over_budget {
        if report.over_budget_frames > m {
            bail!("{} frames over budget (limit {m})", report.over_budget_frames);
        }
    }
    if let Some(m) = a.max_avg_ms {
        if report.avg_work_ms > m {
            bail!("average frame work {:.2} ms exceeds {m} ms", report.avg_work_ms);
        }
    }
    Ok(())
}

/// Serves one UI at a time over WebSocket. Each connection gets a fresh
/// session; messages are the session's commands and events as JSON.
fn serve(game: Option<PathBuf>, port: u16) -> Result<()> {
    use std::sync::mpsc::channel;
    use std::time::Duration;
    use tungstenite::{Error as WsError, Message};
    let listener = std::net::TcpListener::bind(("127.0.0.1", port)).with_context(|| format!("binding port {port}"))?;
    eprintln!("pdsim: serving on ws://127.0.0.1:{port} (open the app's dev server, `npm run dev` in app/)");
    for stream in listener.incoming() {
        let stream = stream?;
        stream.set_nodelay(true).ok();
        let mut ws = match tungstenite::accept(stream) {
            Ok(ws) => ws,
            Err(e) => {
                eprintln!("pdsim: handshake failed: {e}");
                continue;
            }
        };
        ws.get_ref().set_read_timeout(Some(Duration::from_millis(5))).ok();
        eprintln!("pdsim: UI connected");
        let (tx, rx) = channel::<String>();
        let session = pdsim_session::Session::spawn(move |v| {
            let _ = tx.send(v.to_string());
        });
        if let Some(g) = &game {
            session.send(pdsim_session::Command::Load { path: g.display().to_string(), calibration: None });
        }
        'conn: loop {
            match ws.read() {
                Ok(Message::Text(t)) => match serde_json::from_str(t.as_str()) {
                    Ok(v) => {
                        if let Err(e) = session.send_json(v) {
                            eprintln!("pdsim: bad command: {e}");
                        }
                    }
                    Err(e) => eprintln!("pdsim: bad message: {e}"),
                },
                Ok(Message::Close(_)) => break,
                Ok(_) => {}
                Err(WsError::Io(e)) if matches!(e.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut) => {}
                Err(_) => break,
            }
            while let Ok(m) = rx.try_recv() {
                if ws.send(Message::text(m)).is_err() {
                    break 'conn;
                }
            }
        }
        eprintln!("pdsim: UI disconnected");
    }
    Ok(())
}
