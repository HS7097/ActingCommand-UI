// SPDX-License-Identifier: GPL-3.0-only
//! Where a failure goes that has no line of its own on screen: the technical
//! log, the startup-failure window, and the hand-off of a worker's result to
//! the event loop.
//!
//! The technical log is `acui-<start unix ms>.log` in the directory actingd's
//! start logs go to (`logs_dir`). It is created by its first entry, and only a
//! few kinds of entry go into it: a start that failed or found no state root,
//! a problem of the settings file, a hand-off to the event loop that failed, a
//! ledger window that could not be made, shown or hidden, a ledger reader
//! thread that could not start, panicked or was gone, and the answer of a
//! person's resume whose ledger window closed before it came back. No read
//! itself goes there. Once it exists the windows show where it is, and when it cannot be
//! written they say that instead.
//!
//! A start that fails is said everywhere it can be: the log, the console (this
//! process's own, a stream the caller redirected, or the parent's, attached),
//! `StartupErrorWindow`, and when not even that window can be made, a system
//! message box; the exit code is not 0. `--help` goes to the console when there
//! is one and to the same window otherwise, and exits 0.

use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock, PoisonError};
use std::time::{SystemTime, UNIX_EPOCH};

use slint::ComponentHandle;

use crate::strings::{fill, Labels};
use crate::{Scale, StartupErrorWindow};

/// A start that cannot go on: what to say, and the exit code it ends with.
pub struct Fatal {
    text: String,
    code: u8,
}

impl Fatal {
    /// The command line was not understood: exit code 2.
    pub fn arguments(text: String) -> Self {
        Self { text, code: 2 }
    }

    /// Anything else that stops the start: exit code 1.
    pub fn failed(text: String) -> Self {
        Self { text, code: 1 }
    }
}

/// The process's start, which names the log; set by `begin`.
static STARTED: OnceLock<u128> = OnceLock::new();

static LOG: Mutex<Log> = Mutex::new(Log::Unwritten);

/// Whether `log` has written anything: a log that only holds `log_quiet`
/// entries is not worth a line on every window.
static WORTH_SHOWING: AtomicBool = AtomicBool::new(false);

enum Log {
    /// Nothing was logged yet, so there is no file.
    Unwritten,
    Open { path: PathBuf, file: File },
    /// No log directory: the variables it comes from are not set.
    NoDirectory { names: &'static str },
    /// The last entry could not be written there.
    Failed { path: PathBuf, error: String },
}

/// Notes the process's start; the log made later is named by it.
pub fn begin() {
    STARTED.get_or_init(unix_ms);
}

fn unix_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis())
        .unwrap_or(0)
}

/// `%LOCALAPPDATA%\ActingCommand\logs` on Windows, `$XDG_STATE_HOME` (or
/// `$HOME/.local/state`) elsewhere, plus `ActingCommand/logs`. `Err` names the
/// variables that are not set. The directory is not created here.
pub fn logs_dir() -> Result<PathBuf, &'static str> {
    let base = if cfg!(windows) {
        std::env::var_os("LOCALAPPDATA").map(PathBuf::from).ok_or("%LOCALAPPDATA%")
    } else {
        std::env::var_os("XDG_STATE_HOME")
            .map(PathBuf::from)
            .or_else(|| {
                std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".local").join("state"))
            })
            .ok_or("$XDG_STATE_HOME / $HOME")
    }?;
    Ok(base.join("ActingCommand").join("logs"))
}

/// Appends one entry, `<unix ms> <kind>: <text>`, creating the log first when
/// there is none yet or the last entry could not be written. A failure is kept
/// for `log_note` to say; it never stops the caller.
pub fn log(kind: &str, text: &str) {
    WORTH_SHOWING.store(true, Ordering::Relaxed);
    log_quiet(kind, text);
}

/// The same entry for what went as it should, such as a window's glass being
/// built: it goes in the file and does not by itself put the log's place on
/// the windows. Not being able to write it is still shown.
pub fn log_quiet(kind: &str, text: &str) {
    let entry = format!("{} {kind}: {text}\n", unix_ms());
    let mut log = LOG.lock().unwrap_or_else(PoisonError::into_inner);
    if !matches!(*log, Log::Open { .. }) {
        *log = open();
    }
    let failed = match &mut *log {
        Log::Open { path, file } => match file.write_all(entry.as_bytes()).and_then(|()| file.flush()) {
            Ok(()) => None,
            Err(error) => Some((path.clone(), error.to_string())),
        },
        Log::Unwritten | Log::NoDirectory { .. } | Log::Failed { .. } => None,
    };
    if let Some((path, error)) = failed {
        *log = Log::Failed { path, error };
    }
}

fn open() -> Log {
    let dir = match logs_dir() {
        Ok(dir) => dir,
        Err(names) => return Log::NoDirectory { names },
    };
    let started = *STARTED.get_or_init(unix_ms);
    let path = dir.join(format!("acui-{started}.log"));
    let opened = std::fs::create_dir_all(&dir)
        .and_then(|()| OpenOptions::new().create(true).append(true).open(&path));
    match opened {
        Ok(file) => Log::Open { path, file },
        Err(error) => Log::Failed { path, error: error.to_string() },
    }
}

/// The line every window shows about the log, and whether it says a failure:
/// empty while nothing but `log_quiet` entries was logged. Painted by the notices, and by every ledger
/// repaint, since a reader that could not start, panicked or was gone may have
/// just made the log (`Ui::paint_log_notes`).
pub fn log_note(labels: &Labels) -> (String, bool) {
    let log = LOG.lock().unwrap_or_else(PoisonError::into_inner);
    match &*log {
        Log::Unwritten => (String::new(), false),
        Log::Open { path, .. } if WORTH_SHOWING.load(Ordering::Relaxed) => {
            (fill(labels.log_written, &[&path.display().to_string()]), false)
        }
        Log::Open { .. } => (String::new(), false),
        Log::NoDirectory { names } => (fill(labels.log_no_dir, &[*names]), true),
        Log::Failed { path, error } => {
            (fill(labels.log_failed, &[&path.display().to_string(), error]), true)
        }
    }
}

/// Hands `job` from a worker thread to the event loop. That fails only once the
/// loop is gone, as the program ends: the job is dropped, and the log says so.
pub fn post(what: &str, job: impl FnOnce() + Send + 'static) {
    if let Err(error) = slint::invoke_from_event_loop(job) {
        log("post", &format!("{what}: {error}"));
    }
}

/// Says a failed start everywhere it can, then gives its exit code.
pub fn fatal(fatal: &Fatal, labels: &Labels, factor: f32) -> ExitCode {
    log("startup", &fatal.text);
    // The window follows either way: a console, if any, is only the first place.
    to_console(Stream::Err, &fatal.text);
    let page = Page { heading: labels.startup_failed, body: &fatal.text, failed: true };
    if let Err(error) = show(&page, labels, factor) {
        fallback(&page, labels, &error);
    }
    ExitCode::from(fatal.code)
}

/// The usage, on the console when there is one, else in the window; exits 0.
pub fn help(labels: &Labels, factor: f32) -> ExitCode {
    if !to_console(Stream::Out, labels.usage) {
        let page = Page { heading: labels.usage_title, body: labels.usage, failed: false };
        if let Err(error) = show(&page, labels, factor) {
            fallback(&page, labels, &error);
        }
    }
    ExitCode::SUCCESS
}

/// What the startup window shows.
struct Page<'a> {
    heading: &'a str,
    body: &'a str,
    failed: bool,
}

/// The startup window, until it is closed.
fn show(page: &Page<'_>, labels: &Labels, factor: f32) -> Result<(), slint::PlatformError> {
    let window = StartupErrorWindow::new()?;
    window.global::<Scale>().set_factor(factor);
    window.set_heading(page.heading.into());
    window.set_body(page.body.into());
    window.set_failed(page.failed);
    let (note, failed) = log_note(labels);
    window.set_log_note(note.into());
    window.set_log_failed(failed);
    window.set_close_label(labels.close.into());
    window.on_dismiss(|| {
        if let Err(error) = slint::quit_event_loop() {
            log("startup", &error.to_string());
        }
    });
    window.run()
}

/// The window could not be made or run: the system message box, with why.
fn fallback(page: &Page<'_>, labels: &Labels, error: &slint::PlatformError) {
    let why = fill(labels.window_failed, &[&error.to_string()]);
    log("startup", &why);
    let mut text = format!("{}\n\n{why}", page.body);
    let (note, _) = log_note(labels);
    if !note.is_empty() {
        text.push_str("\n\n");
        text.push_str(&note);
    }
    if let Err(error) = platform::message_box(page.heading, &text, page.failed) {
        log("startup", &format!("MessageBoxW: {error}"));
    }
}

/// The standard stream a text is for.
#[derive(Clone, Copy)]
enum Stream {
    Out,
    Err,
}

/// How a stream reaches a console.
#[derive(Clone, Copy, PartialEq, Eq)]
#[cfg_attr(not(windows), allow(dead_code))]
enum Reach {
    /// Neither a console nor a redirected stream.
    None,
    /// This process's own console, or a file or pipe the caller gave.
    Own,
    /// The parent's console, attached just now.
    Attached,
}

/// Writes `text` and a line end where the stream reaches; false when it
/// reaches nowhere or the write failed.
fn to_console(stream: Stream, text: &str) -> bool {
    let reach = platform::reach(stream);
    if reach == Reach::None {
        return false;
    }
    // The prompt the caller's shell printed already keeps a line of its own.
    let lead = if reach == Reach::Attached { "\n" } else { "" };
    let written = match stream {
        Stream::Out => write_line(&mut std::io::stdout().lock(), lead, text),
        Stream::Err => write_line(&mut std::io::stderr().lock(), lead, text),
    };
    written.is_ok()
}

fn write_line(out: &mut impl Write, lead: &str, text: &str) -> std::io::Result<()> {
    writeln!(out, "{lead}{text}")?;
    out.flush()
}

/// The Windows calls std does not make: the standard handles, attaching the
/// parent's console, and the system message box. Each library is linked by
/// name: std promises none of these calls, and user32 not at all.
#[cfg(windows)]
mod platform {
    use std::ffi::c_void;
    use std::os::windows::io::IntoRawHandle;

    use super::{Reach, Stream};

    #[link(name = "kernel32")]
    extern "system" {
        fn AttachConsole(process: u32) -> i32;
        fn GetStdHandle(which: u32) -> *mut c_void;
        fn SetStdHandle(which: u32, handle: *mut c_void) -> i32;
        fn GetFileType(handle: *mut c_void) -> u32;
    }

    #[link(name = "user32")]
    extern "system" {
        fn MessageBoxW(owner: *mut c_void, text: *const u16, caption: *const u16, kind: u32) -> i32;
    }

    const STD_OUTPUT_HANDLE: u32 = -11i32 as u32;
    const STD_ERROR_HANDLE: u32 = -12i32 as u32;
    const ATTACH_PARENT_PROCESS: u32 = u32::MAX;
    const FILE_TYPE_DISK: u32 = 1;
    const FILE_TYPE_CHAR: u32 = 2;
    const FILE_TYPE_PIPE: u32 = 3;
    const MB_ICONERROR: u32 = 0x10;
    const MB_ICONINFORMATION: u32 = 0x40;

    /// A stream that already goes somewhere — this process's own console, or a
    /// file or pipe the caller gave — is used as it is. Otherwise the parent's
    /// console is attached and `CONOUT$` becomes the stream.
    pub fn reach(stream: Stream) -> Reach {
        let which = match stream {
            Stream::Out => STD_OUTPUT_HANDLE,
            Stream::Err => STD_ERROR_HANDLE,
        };
        // SAFETY: plain queries of this process's own standard handle.
        let kind = unsafe {
            let handle = GetStdHandle(which);
            if handle.is_null() || handle as isize == -1 {
                0
            } else {
                GetFileType(handle)
            }
        };
        if matches!(kind, FILE_TYPE_DISK | FILE_TYPE_CHAR | FILE_TYPE_PIPE) {
            return Reach::Own;
        }
        // SAFETY: attaching to the parent's console changes nothing else here.
        let attached = unsafe { AttachConsole(ATTACH_PARENT_PROCESS) } != 0;
        // Without a console, attached now or before, `CONOUT$` does not open.
        // Read access too: Rust writes UTF-16 to a console only when it can ask
        // the console's mode.
        let Ok(console) = std::fs::OpenOptions::new().read(true).write(true).open("CONOUT$") else {
            return Reach::None;
        };
        // The handle stays open for the life of the process.
        let handle = console.into_raw_handle();
        // SAFETY: `handle` is a valid console handle this process owns.
        if unsafe { SetStdHandle(which, handle) } == 0 {
            return Reach::None;
        }
        if attached {
            Reach::Attached
        } else {
            Reach::Own
        }
    }

    pub fn message_box(caption: &str, text: &str, failed: bool) -> std::io::Result<()> {
        let wide = |value: &str| value.encode_utf16().chain(std::iter::once(0)).collect::<Vec<u16>>();
        let (text, caption) = (wide(text), wide(caption));
        let kind = if failed { MB_ICONERROR } else { MB_ICONINFORMATION };
        // SAFETY: both strings are NUL-terminated UTF-16 that outlive the call,
        // and the box has no owner window.
        let pressed =
            unsafe { MessageBoxW(std::ptr::null_mut(), text.as_ptr(), caption.as_ptr(), kind) };
        if pressed == 0 {
            Err(std::io::Error::last_os_error())
        } else {
            Ok(())
        }
    }
}

/// Elsewhere the standard streams are this process's own, and stderr is the
/// last place a failure goes.
#[cfg(not(windows))]
mod platform {
    use super::{Reach, Stream};

    pub fn reach(_stream: Stream) -> Reach {
        Reach::Own
    }

    pub fn message_box(_caption: &str, _text: &str, _failed: bool) -> std::io::Result<()> {
        Ok(())
    }
}
