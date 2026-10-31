// SPDX-License-Identifier: GPL-3.0-only
//! `fixture-state-root`: writes a small Runtime v0.12 state root, the console's
//! offline acceptance material (Workflow #429 E11).
//!
//! It starts the pinned Runtime host on a new directory with two fake devices,
//! `node.a` (ADB 127.0.0.1:16384) and `node.b` (127.0.0.1:16416), drives it
//! through the typed client and closes it gracefully. The root then holds what
//! the host itself wrote: one `runtime.instance_bound` fact per instance (both
//! with their port, so the port table has rows), two committed capture frames
//! with their PNG material (one before the lease rounds, one after them), and a
//! few hundred lease, input and client-action events, more than one page of the
//! console's offline read.
//!
//! It makes material and is not a test: it checks nothing about the Runtime.
//! Every failure prints the stage that failed and ends with a non-zero exit code.

use std::fmt;
use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use actingcommand_contract::{
    ApplicationLifecycleAction, BackendOpenEntry, CaptureSequenceSpec, ClientActionKind,
    ClientActionRecord, EventActor, EventSource, FencedWrite, IdentifierIssuer, InputAction,
    InstanceId, RuntimeChannel, MAX_RUNTIME_EVENT_QUERY_EVENTS,
};
use actingcommand_device::{
    CaptureBackend, CaptureBackendName, DeviceCloseAuthority, DeviceError,
    DeviceResourceCloseOutcome, DeviceResult, Frame, FrameMemoryBudget, InputBackend,
    OpenedBackend, PixelFormat,
};
use actingcommand_ledger::{GlobalLedger, GlobalLedgerEvidenceConfig};
use actingcommand_runtime_client::{RuntimeClient, RuntimeClientConfig};
use actingcommand_runtime_host::{
    ExecutionBackendProvider, ForegroundApplicationObservation, ResolvedAdbEndpoint,
    ResolvedExecutionInstance, RuntimeHost, RuntimeHostConfig,
};

const USAGE: &str =
    "usage: fixture-state-root --out <new or empty directory> [--rounds <1..=10000>]";
/// Roughly ten events a round (lease, presses, release, each with its request
/// lifecycle): 60 rounds fill two to three console pages.
const DEFAULT_ROUNDS: u32 = 60;
const MAX_ROUNDS: u32 = 10_000;
/// The salt of the root's secret fingerprints. The root is acceptance material
/// for anyone to open, so the salt is fixed and public.
const FINGERPRINT_SALT: &[u8] = b"acui-e11-fixture-state-root";
/// The application both fake devices report in the foreground and assigned, so
/// the Runtime's foreground gate admits every press.
const APPLICATION_ID: &str = "neutral.application";
const ADB_HOST: &str = "127.0.0.1";
const FRAME_WIDTH: u32 = 320;
const FRAME_HEIGHT: u32 = 180;

struct InstanceSpec {
    alias: &'static str,
    port: u16,
    color: [u8; 3],
}

const INSTANCES: [InstanceSpec; 2] = [
    InstanceSpec {
        alias: "node.a",
        port: 16384,
        color: [40, 120, 220],
    },
    InstanceSpec {
        alias: "node.b",
        port: 16416,
        color: [220, 140, 40],
    },
];

fn main() -> ExitCode {
    let args = match parse_args() {
        Ok(args) => args,
        Err(error) => {
            eprintln!("fixture-state-root: arguments: {error}");
            eprintln!("{USAGE}");
            return ExitCode::from(2);
        }
    };
    match run(&args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("fixture-state-root: FAILED: {error}");
            ExitCode::FAILURE
        }
    }
}

struct Args {
    out: PathBuf,
    rounds: u32,
}

fn parse_args() -> Result<Args, String> {
    let mut out = None;
    let mut rounds = DEFAULT_ROUNDS;
    let mut args = std::env::args_os().skip(1);
    while let Some(arg) = args.next() {
        match arg.to_str() {
            Some("--out") => {
                let value = args.next().ok_or("--out needs a directory")?;
                out = Some(PathBuf::from(value));
            }
            Some("--rounds") => {
                let value = args.next().ok_or("--rounds needs a number")?;
                rounds = value
                    .to_str()
                    .and_then(|text| text.parse::<u32>().ok())
                    .filter(|count| (1..=MAX_ROUNDS).contains(count))
                    .ok_or_else(|| {
                        format!(
                            "--rounds must be 1..={MAX_ROUNDS}, not {}",
                            value.to_string_lossy()
                        )
                    })?;
            }
            _ => return Err(format!("unknown argument {}", arg.to_string_lossy())),
        }
    }
    let out = out.ok_or("--out is required")?;
    Ok(Args { out, rounds })
}

/// `stage: Display [Debug]`, so a failure carries its code and its detail.
fn failure<E: fmt::Display + fmt::Debug>(stage: &str, error: &E) -> String {
    format!("{stage}: {error} [{error:?}]")
}

/// The absolute output directory: created when missing, refused when it holds
/// anything, so the root is only what this run wrote.
fn prepare_out(requested: &Path) -> Result<PathBuf, String> {
    let out = std::path::absolute(requested)
        .map_err(|error| format!("resolve --out {}: {error}", requested.display()))?;
    match fs::read_dir(&out) {
        Ok(mut entries) => {
            if entries.next().is_some() {
                return Err(format!(
                    "--out {} is not empty; give a new or empty directory",
                    out.display()
                ));
            }
        }
        Err(error) if error.kind() == ErrorKind::NotFound => {
            fs::create_dir_all(&out)
                .map_err(|error| format!("create --out {}: {error}", out.display()))?;
        }
        Err(error) => return Err(format!("read --out {}: {error}", out.display())),
    }
    Ok(out)
}

fn run(args: &Args) -> Result<(), String> {
    let root = prepare_out(&args.out)?;
    let issuer = IdentifierIssuer::new().map_err(|error| failure("identifier issuer", &error))?;
    let mut instances = Vec::with_capacity(INSTANCES.len());
    for spec in &INSTANCES {
        let instance_id = *issuer
            .mint_instance_id()
            .map_err(|error| failure("mint instance id", &error))?
            .transport();
        instances.push(FixtureInstance {
            alias: spec.alias,
            instance_id,
            port: spec.port,
            color: spec.color,
        });
    }
    let device = Arc::new(FixtureDevice::default());
    let provider = FixtureProvider {
        instances,
        device: Arc::clone(&device),
    };
    // The frame cleaner is off, so no committed frame is evicted before the
    // root is packed.
    let host = RuntimeHost::start(
        RuntimeHostConfig::new(&root, FINGERPRINT_SALT).with_frame_retention_enabled(false),
        Arc::new(provider),
    )
    .map_err(|error| failure("start runtime host", &error))?;
    println!(
        "fixture-state-root: runtime host started on {}",
        root.display()
    );

    // Every client is dropped inside `drive`, before the host closes.
    let driven = drive(&root, args.rounds);
    let health = match host.fatal_error() {
        Ok(None) => Ok(()),
        Ok(Some(error)) => Err(failure("runtime host latched a fatal error", &error)),
        Err(error) => Err(failure("read runtime host health", &error)),
    };
    let closed = host
        .close()
        .map_err(|error| failure("close runtime host", &error));
    let failures: Vec<String> = [driven, health, closed]
        .into_iter()
        .filter_map(Result::err)
        .collect();
    if !failures.is_empty() {
        return Err(failures.join("; "));
    }
    println!("fixture-state-root: runtime host closed");
    report(&root, &device)
}

/// Drives the running host: a frame of `node.a`, `rounds` lease rounds over both
/// instances with the console person's presses between them, a frame of `node.b`.
fn drive(root: &Path, rounds: u32) -> Result<(), String> {
    let cli = RuntimeClient::connect(RuntimeClientConfig::new(
        root,
        RuntimeChannel::Cli,
        EventActor::Cli,
        EventSource::Cli,
    ))
    .map_err(|error| failure("connect the cli client", &error))?;
    // The console's person connection (actor `user`, source `ui`), as the
    // console records a press.
    let person = RuntimeClient::connect(RuntimeClientConfig::new(
        root,
        RuntimeChannel::Ui,
        EventActor::User,
        EventSource::Ui,
    ))
    .map_err(|error| failure("connect the console person client", &error))?;
    let presses = person
        .begin_interaction()
        .map_err(|error| failure("begin the console interaction", &error))?;

    let first = INSTANCES[0].alias;
    cli.observe_readonly(first)
        .map_err(|error| failure(&format!("observe_readonly {first}"), &error))?;

    for round in 0..rounds {
        let alias = INSTANCES[round as usize % INSTANCES.len()].alias;
        let stage = |what: &str| format!("round {round} {what} {alias}");
        let token = cli
            .acquire_lease(alias)
            .map_err(|error| failure(&stage("acquire_lease"), &error))?;
        let x = 10 + (round % 300) as i32;
        cli.input(&token, InputAction::Tap { x, y: 20 })
            .map_err(|error| failure(&stage("tap"), &error))?;
        if round % 5 == 4 {
            cli.input(
                &token,
                InputAction::Swipe {
                    x1: 40,
                    y1: 90,
                    x2: 280,
                    y2: 90,
                    duration_ms: 120,
                },
            )
            .map_err(|error| failure(&stage("swipe"), &error))?;
            cli.input(
                &token,
                InputAction::LongTap {
                    x: 160,
                    y: 90,
                    duration_ms: 300,
                },
            )
            .map_err(|error| failure(&stage("long tap"), &error))?;
        }
        cli.release_lease(&token)
            .map_err(|error| failure(&stage("release_lease"), &error))?;
        if round % 4 == 3 {
            let action = ClientActionRecord::new(
                "acui.fixture",
                format!("press_{round}"),
                ClientActionKind::Button,
                Some(alias.to_owned()),
                None,
            )
            .map_err(|error| failure(&stage("client action record"), &error))?;
            presses
                .record_client_action(action)
                .map_err(|error| failure(&stage("record_client_action"), &error))?;
        }
    }

    let last = INSTANCES[1].alias;
    let spec =
        CaptureSequenceSpec::new(1, 0).map_err(|error| failure("capture sequence spec", &error))?;
    cli.capture_sequence(last, spec)
        .map_err(|error| failure(&format!("capture_sequence {last}"), &error))?;
    Ok(())
}

/// Reopens the closed root through the console's own offline opening and prints
/// what it holds.
fn report(root: &Path, device: &FixtureDevice) -> Result<(), String> {
    let ledger = GlobalLedger::open_metadata(GlobalLedgerEvidenceConfig::new(root))
        .map_err(|error| failure("open the closed root offline", &error))?;
    let events = ledger.event_count();
    println!(
        "fixture-state-root: ledger backend={} events={} latest_sequence={}",
        ledger.backend(),
        events,
        ledger.latest_sequence()
    );
    println!(
        "fixture-state-root: fake devices taps={} swipes={} long_taps={} captures={}",
        device.taps.load(Ordering::Acquire),
        device.swipes.load(Ordering::Acquire),
        device.long_taps.load(Ordering::Acquire),
        device.captures.load(Ordering::Acquire)
    );
    // The material exists to page: a root within one page is not what was asked.
    let page = usize::from(MAX_RUNTIME_EVENT_QUERY_EVENTS);
    if events <= page {
        return Err(format!(
            "the root holds {events} events, no more than one console page ({page}); \
             run again with a larger --rounds"
        ));
    }
    Ok(())
}

#[derive(Default)]
struct FixtureDevice {
    taps: AtomicUsize,
    long_taps: AtomicUsize,
    swipes: AtomicUsize,
    captures: AtomicUsize,
}

struct FixtureInstance {
    alias: &'static str,
    instance_id: InstanceId,
    port: u16,
    color: [u8; 3],
}

struct FixtureProvider {
    instances: Vec<FixtureInstance>,
    device: Arc<FixtureDevice>,
}

impl FixtureProvider {
    fn instance(&self, alias: &str) -> DeviceResult<&FixtureInstance> {
        self.instances
            .iter()
            .find(|instance| instance.alias == alias)
            .ok_or_else(|| DeviceError::fatal(format!("fixture provider has no instance {alias}")))
    }
}

impl ExecutionBackendProvider for FixtureProvider {
    fn instance_aliases(&self) -> Vec<String> {
        self.instances
            .iter()
            .map(|instance| instance.alias.to_owned())
            .collect()
    }

    /// An explicit HOST:PORT endpoint (no serial): the Runtime writes the port
    /// into the instance's `runtime.instance_bound` fact.
    fn resolve(&self, instance_alias: &str) -> Option<ResolvedExecutionInstance> {
        self.instances
            .iter()
            .find(|instance| instance.alias == instance_alias)
            .map(|instance| {
                ResolvedExecutionInstance::new(
                    instance.instance_id,
                    format!("{ADB_HOST}:{}", instance.port),
                )
                .with_adb_endpoint(ResolvedAdbEndpoint::new(
                    ADB_HOST,
                    instance.port,
                    false,
                ))
            })
    }

    fn open_input(
        &self,
        instance_alias: &str,
    ) -> DeviceResult<OpenedBackend<Box<dyn InputBackend>>> {
        self.instance(instance_alias)?;
        let backend: Box<dyn InputBackend> = Box::new(FixtureInput {
            device: Arc::clone(&self.device),
        });
        Ok(OpenedBackend::unobserved(backend, BackendOpenEntry::Input))
    }

    fn open_capture(
        &self,
        instance_alias: &str,
        _memory: Option<&FrameMemoryBudget>,
    ) -> DeviceResult<OpenedBackend<Box<dyn CaptureBackend>>> {
        let instance = self.instance(instance_alias)?;
        let backend: Box<dyn CaptureBackend> = Box::new(FixtureCapture {
            device: Arc::clone(&self.device),
            color: instance.color,
        });
        Ok(OpenedBackend::unobserved(
            backend,
            BackendOpenEntry::Capture,
        ))
    }

    fn control_application(
        &self,
        _witness: &FencedWrite,
        instance_alias: &str,
        _action: ApplicationLifecycleAction,
    ) -> DeviceResult<()> {
        self.instance(instance_alias).map(|_| ())
    }

    fn observe_foreground_application(
        &self,
        instance_alias: &str,
    ) -> DeviceResult<ForegroundApplicationObservation> {
        self.instance(instance_alias)?;
        Ok(ForegroundApplicationObservation {
            foreground: Some(APPLICATION_ID.to_owned()),
            assigned: APPLICATION_ID.to_owned(),
        })
    }
}

/// Counts every press and performs nothing.
struct FixtureInput {
    device: Arc<FixtureDevice>,
}

impl InputBackend for FixtureInput {
    fn tap(&mut self, _witness: &FencedWrite, _x: i32, _y: i32) -> DeviceResult<()> {
        self.device.taps.fetch_add(1, Ordering::AcqRel);
        Ok(())
    }

    fn long_tap(
        &mut self,
        _witness: &FencedWrite,
        _x: i32,
        _y: i32,
        _duration_ms: u64,
    ) -> DeviceResult<()> {
        self.device.long_taps.fetch_add(1, Ordering::AcqRel);
        Ok(())
    }

    fn swipe(
        &mut self,
        _witness: &FencedWrite,
        _x1: i32,
        _y1: i32,
        _x2: i32,
        _y2: i32,
        _duration_ms: u64,
    ) -> DeviceResult<()> {
        self.device.swipes.fetch_add(1, Ordering::AcqRel);
        Ok(())
    }

    fn key(&mut self, _witness: &FencedWrite, _key: &str) -> DeviceResult<()> {
        Ok(())
    }

    fn text(&mut self, _witness: &FencedWrite, _text: &str) -> DeviceResult<()> {
        Ok(())
    }

    fn reset(&mut self, _witness: &FencedWrite) -> DeviceResult<()> {
        Ok(())
    }

    fn close_once(
        &mut self,
        _authority: DeviceCloseAuthority,
    ) -> DeviceResult<DeviceResourceCloseOutcome> {
        Ok(DeviceResourceCloseOutcome::confirmed(1))
    }
}

/// Serves the instance's own colour as a 320x180 RGB frame: two gradients and a
/// checkerboard whose phase alternates per capture.
struct FixtureCapture {
    device: Arc<FixtureDevice>,
    color: [u8; 3],
}

impl CaptureBackend for FixtureCapture {
    fn capture(&mut self) -> DeviceResult<Frame> {
        let shot = self.device.captures.fetch_add(1, Ordering::AcqRel);
        fixture_frame(self.color, shot)
    }

    fn close_once(
        &mut self,
        _authority: DeviceCloseAuthority,
    ) -> DeviceResult<DeviceResourceCloseOutcome> {
        Ok(DeviceResourceCloseOutcome::confirmed(1))
    }
}

fn fixture_frame(color: [u8; 3], shot: usize) -> DeviceResult<Frame> {
    let [red, green, blue] = color;
    let phase = (shot % 2) as u32;
    let mut pixels = Vec::with_capacity(FRAME_WIDTH as usize * FRAME_HEIGHT as usize * 3);
    for y in 0..FRAME_HEIGHT {
        for x in 0..FRAME_WIDTH {
            let across = (x * 127 / (FRAME_WIDTH - 1)) as u8;
            let down = (y * 127 / (FRAME_HEIGHT - 1)) as u8;
            let checker = (x / 20 + y / 20 + phase) % 2 == 0;
            pixels.push(red / 2 + across);
            pixels.push(green / 2 + down);
            pixels.push(if checker { blue } else { blue / 3 });
        }
    }
    Frame::from_pixels(
        FRAME_WIDTH,
        FRAME_HEIGHT,
        pixels,
        PixelFormat::Rgb8,
        CaptureBackendName::AdbScreencap,
    )
}
