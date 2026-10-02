// SPDX-License-Identifier: GPL-3.0-only
//! TEMPORARY one-off for Workflow #337 U1, run once on the Windows runner and
//! reverted right after: drives the wizard's own install, upgrade and ADB
//! server code on synthetic install roots. Not a test of the product.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use crate::adb_server::{self, AdbServer};
use crate::upgrade::{self, Upgraded};
use crate::verify::{self, Reporter};
use crate::{install, platform};

struct Printer {
    lines: Vec<String>,
}

impl Reporter for Printer {
    fn line(&mut self, line: &str) -> Result<(), String> {
        println!("    | {line}");
        self.lines.push(line.to_string());
        Ok(())
    }

    fn warn(&mut self, line: &str) -> Result<(), String> {
        println!("    ! NOTE: {line}");
        self.lines.push(format!("NOTE: {line}"));
        Ok(())
    }
}

impl Printer {
    fn new() -> Self {
        Printer { lines: Vec::new() }
    }

    fn has(&self, needle: &str) -> bool {
        self.lines.iter().any(|line| line.contains(needle))
    }
}

fn env(name: &str) -> PathBuf {
    PathBuf::from(std::env::var_os(name).unwrap_or_else(|| panic!("{name} is not set")))
}

/// Every file under `dir`, `/`-separated, sorted.
fn files(dir: &Path) -> Vec<String> {
    fn walk(base: &Path, dir: &Path, out: &mut Vec<String>) {
        for entry in fs::read_dir(dir).unwrap_or_else(|error| panic!("{}: {error}", dir.display())) {
            let path = entry.unwrap().path();
            if path.is_dir() {
                walk(base, &path, out);
            } else {
                let relative = path.strip_prefix(base).unwrap();
                out.push(
                    relative
                        .components()
                        .map(|c| c.as_os_str().to_string_lossy().into_owned())
                        .collect::<Vec<_>>()
                        .join("/"),
                );
            }
        }
    }
    let mut out = Vec::new();
    walk(dir, dir, &mut out);
    out.sort();
    out
}

/// Every file under `dir` with its size and sha256.
fn snapshot(dir: &Path) -> Vec<(String, u64, String)> {
    files(dir)
        .into_iter()
        .map(|file| {
            let path = dir.join(file.replace('/', "\\"));
            let size = fs::metadata(&path).unwrap().len();
            let sha = verify::sha256_file(&path).unwrap_or_else(|error| format!("unreadable: {error}"));
            (file, size, sha)
        })
        .collect()
}

/// The names directly under `root`, staging directories left out.
fn top(root: &Path) -> Vec<String> {
    let mut names: Vec<String> = fs::read_dir(root)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|name| !name.starts_with(".staging-"))
        .collect();
    names.sort();
    names
}

/// A copy of PING.EXE at `copy`, run for five minutes with `cwd` as its
/// working directory: a stand-in for a process that works in a directory.
fn holder(ping: &Path, copy: &Path, cwd: &Path) -> Child {
    fs::copy(ping, copy).unwrap();
    let child = Command::new(copy)
        .args(["-n", "300", "127.0.0.1"])
        .current_dir(cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    std::thread::sleep(Duration::from_millis(1500));
    println!("  stand-in {} PID {} working in {}", copy.display(), child.id(), cwd.display());
    child
}

fn end(mut child: Child) {
    let _ = child.kill();
    let _ = child.wait();
}

fn free_port(from: u16) -> u16 {
    (from..from + 100)
        .find(|port| std::net::TcpListener::bind(("127.0.0.1", *port)).is_ok())
        .expect("a free port")
}

fn stage(new: &Path, root: &Path, tag: &str, p: &mut Printer) -> verify::Verified {
    verify::run(new, &root.join(format!(".staging-oneoff-{tag}")), p).expect("verify")
}

fn show(server: &AdbServer) -> String {
    match server {
        AdbServer::Ready(how) => format!("Ready: {how}"),
        AdbServer::NotReady(why) => format!("NotReady: {why}"),
    }
}

#[test]
fn oneoff_337_u1() {
    let new = env("ONEOFF_NEW");
    let root = env("ONEOFF_ROOT");
    let base = env("ONEOFF_BASE");
    let ping = PathBuf::from(r"C:\Windows\System32\PING.EXE");
    let wanted = verify::members_of(&fs::read_to_string(new.join("MEMBERS.json")).unwrap()).unwrap();
    println!("release to install: runtime {} ui {}", wanted.0, wanted.1);

    println!("== T1 fresh install");
    let fresh = base.join("fresh");
    fs::create_dir_all(&fresh).unwrap();
    let mut p = Printer::new();
    let verified = stage(&new, &fresh, "t1", &mut p);
    for staged in [&verified.runtime, &verified.ui, &verified.tools] {
        println!("  staged into {}", staged.dir.display());
    }
    assert!(verified.runtime.dir.ends_with("runtime") && verified.ui.dir.ends_with("ui") && verified.tools.dir.ends_with("tools"));
    let manifest: serde_json::Value =
        serde_json::from_slice(&fs::read(verified.tools.dir.join("BUILD-MANIFEST.json")).unwrap()).unwrap();
    println!(
        "  tools manifest: commit {} · tools_payload_layout {} · {} files",
        manifest["commit_sha"],
        manifest["tools_payload_layout"],
        manifest["files"].as_array().unwrap().len()
    );
    let pinned: Vec<(String, String)> = manifest["files"]
        .as_array()
        .unwrap()
        .iter()
        .map(|file| (file["path"].as_str().unwrap().to_string(), file["sha256"].as_str().unwrap().to_lowercase()))
        .filter(|(path, _)| path.starts_with("platform-tools/"))
        .collect();
    assert_eq!(pinned.len(), 5);
    let laid = install::lay_out(&fresh, &verified, &mut p).expect("lay_out");
    let tools = files(&laid.tools_dir);
    println!("  tools\\ holds {} files: {tools:?}", tools.len());
    assert_eq!(tools.len(), 8);
    for (path, sha) in &pinned {
        let actual = verify::sha256_file(&laid.tools_dir.join(path.replace('/', "\\"))).unwrap();
        println!("  {path}: sha256 {actual} · manifest {sha} · {}", if &actual == sha { "MATCH" } else { "MISMATCH" });
        assert_eq!(&actual, sha);
    }
    println!("  finish page Tools line names: {}", super::platform_tools(&laid.tools_dir));
    assert!(!fresh.join(".staging-oneoff-t1").exists());
    println!("PASS T1");

    println!("== T2 upgrade from a v0.9.0-shaped root");
    let config = root.join("actingd.config.json");
    let config_before = fs::read(&config).unwrap();
    let installed = upgrade::installed(&root).unwrap().expect("installed");
    println!(
        "  installed runtime {} ui {} · tools\\platform-tools\\adb.exe present: {}",
        installed.runtime_sha, installed.ui_sha, installed.adb
    );
    assert!(!installed.adb && !installed.current(&wanted));
    println!("  page line: {}", super::upgrade_line(&installed, &wanted));
    let old_tools = files(&root.join("tools"));
    let mut p = Printer::new();
    let verified = stage(&new, &root, "t2", &mut p);
    let up = upgrade::upgrade(&root, &verified, &mut p).expect("upgrade");
    let config_after = fs::read(&config).unwrap();
    println!(
        "  actingd.config.json byte-identical: {} ({} bytes, sha256 before {} after {})",
        config_before == config_after,
        config_after.len(),
        sha(&config_before),
        sha(&config_after)
    );
    assert_eq!(config_before, config_after);
    let previous_tools = files(&root.join("previous").join("tools"));
    println!("  previous\\tools: {previous_tools:?}");
    assert_eq!(previous_tools, old_tools);
    assert!(!previous_tools.iter().any(|file| file.starts_with("platform-tools")));
    let tools = files(&root.join("tools"));
    println!("  tools\\: {tools:?}");
    assert_eq!(tools.len(), 8);
    for dir in ["runtime", "ui", "tools"] {
        let staged = root.join(".staging-oneoff-t2").join(dir);
        assert!(p.has(&format!("→ {}", staged.display())), "staging {dir}");
        println!("  log names staging dir {}", staged.display());
    }
    assert!(p.has(&format!("moved aside: {}", root.join("ui").display())));
    println!("  ADB server: {}", show(&up.adb_server));
    assert!(matches!(up.adb_server, AdbServer::Ready(_)));
    match platform::loopback_listeners(adb_server::PORT) {
        Ok(pids) => {
            for pid in pids {
                println!("  listening on 127.0.0.1:{}: PID {pid} · {:?}", adb_server::PORT, platform::process_image(pid));
            }
        }
        Err(error) => println!("  listeners: {error}"),
    }
    println!("PASS T2");

    println!("== T3 same release, tools\\platform-tools\\adb.exe missing: laid out again");
    let adb = upgrade::ac_adb(&root);
    fs::rename(&adb, adb.with_file_name("adb.exe.oneoff-moved")).unwrap();
    let installed = upgrade::installed(&root).unwrap().unwrap();
    println!(
        "  installed runtime {} ui {} · adb present: {} · current: {}",
        installed.runtime_sha,
        installed.ui_sha,
        installed.adb,
        installed.current(&wanted)
    );
    assert!(installed.runtime_sha == wanted.0 && installed.ui_sha == wanted.1);
    assert!(!installed.current(&wanted));
    println!("  page line: {}", super::upgrade_line(&installed, &wanted));
    let mut p = Printer::new();
    let verified = stage(&new, &root, "t3", &mut p);
    let up = upgrade::upgrade(&root, &verified, &mut p).expect("reinstall");
    assert!(adb.is_file());
    let installed = upgrade::installed(&root).unwrap().unwrap();
    println!("  after: adb present {} · current {}", installed.adb, installed.current(&wanted));
    assert!(installed.current(&wanted));
    // The server the check started in T2 works in the root: ui\ moved whole.
    assert!(p.has(&format!("moved aside: {}", root.join("ui").display())));
    assert!(!p.has("逐项移开"));
    println!("  ui\\ moved whole (the server started in T2 does not work in ui\\)");
    println!("  ADB server: {}", show(&up.adb_server));
    assert!(matches!(up.adb_server, AdbServer::Ready(_)));
    println!("PASS T3");

    println!("== T3b the older version holds the running server's image");
    let mut p = Printer::new();
    let verified = stage(&new, &root, "t3b", &mut p);
    let up = upgrade::upgrade(&root, &verified, &mut p).expect("upgrade");
    println!(
        "  older version warning with the adb note shown: {}",
        p.has("NOTE: 更早的旧版本未能删除") && p.has("adb.exe / AdbWinApi.dll")
    );
    println!("  ADB server: {}", show(&up.adb_server));
    println!("DONE T3b (informational)");

    println!("== T4 ui\\ held as a working directory by a stand-in process");
    let mut holder_ui = holder(&ping, &base.join("holder-ui.exe"), &root.join("ui"));
    let mut p = Printer::new();
    let verified = stage(&new, &root, "t4", &mut p);
    let up = upgrade::upgrade(&root, &verified, &mut p).expect("upgrade with ui held");
    assert!(p.has("逐项移开"));
    assert!(holder_ui.try_wait().unwrap().is_none(), "the stand-in still runs");
    println!(
        "  stand-in still runs; ui\\ {} files, previous\\ui {} files",
        files(&root.join("ui")).len(),
        files(&root.join("previous").join("ui")).len()
    );
    assert!(root.join("ui").join("acui.exe").is_file());
    println!("  ADB server: {}", show(&up.adb_server));
    println!("PASS T4");

    println!("== T5 undo puts ui\\ back entry by entry (tools\\ held by a second stand-in)");
    let ui_before = snapshot(&root.join("ui"));
    let tools_before = snapshot(&root.join("tools"));
    let runtime_before = snapshot(&root.join("runtime"));
    let previous_before = snapshot(&root.join("previous"));
    let top_before = top(&root);
    let holder_tools = holder(&ping, &base.join("holder-tools.exe"), &root.join("tools"));
    let mut p = Printer::new();
    let verified = stage(&new, &root, "t5", &mut p);
    let reason = upgrade::upgrade(&root, &verified, &mut p).err().expect("must stop at tools");
    println!("  reason:\n{reason}");
    assert!(p.has("逐项移开"));
    assert!(reason.contains(&format!("cannot move {}", root.join("tools").display())));
    for broken in ["not put back", "not removed", "未能放回", "未能删除"] {
        assert!(!reason.contains(broken), "{broken}");
    }
    assert_eq!(snapshot(&root.join("ui")), ui_before);
    assert_eq!(snapshot(&root.join("tools")), tools_before);
    assert_eq!(snapshot(&root.join("runtime")), runtime_before);
    assert_eq!(snapshot(&root.join("previous")), previous_before);
    assert_eq!(top(&root), top_before);
    println!(
        "  ui\\ ({} files), tools\\, runtime\\, previous\\ and the root's entries are as before",
        ui_before.len()
    );
    end(holder_tools);
    println!("PASS T5");

    println!("== T6 an unrecognised running Runtime stops it");
    let actingd = root.join("runtime").join("actingcommand-actingd.exe");
    let real = root.join("runtime").join("actingcommand-actingd.exe.oneoff-real");
    fs::rename(&actingd, &real).unwrap();
    let fake = holder(&ping, &actingd, &root);
    let ui_before = snapshot(&root.join("ui"));
    let top_before = top(&root);
    let mut p = Printer::new();
    let verified = stage(&new, &root, "t6", &mut p);
    let reason = upgrade::upgrade(&root, &verified, &mut p).err().expect("must stop");
    println!("  reason:\n{reason}");
    assert!(reason.contains("Runtime 仍在运行"));
    assert!(!p.has("逐项移开"));
    assert_eq!(snapshot(&root.join("ui")), ui_before);
    assert_eq!(top(&root), top_before);
    end(fake);
    fs::rename(&actingd, root.join("runtime").join("actingcommand-actingd.exe.oneoff-ping")).unwrap();
    fs::rename(&real, &actingd).unwrap();
    println!("PASS T6");

    println!("== T7 a running console stops it as before");
    let acui = root.join("ui").join("acui.exe");
    let real = root.join("ui").join("acui.exe.oneoff-real");
    fs::rename(&acui, &real).unwrap();
    let fake = holder(&ping, &acui, &root.join("ui"));
    let top_before = top(&root);
    let mut p = Printer::new();
    let verified = stage(&new, &root, "t7", &mut p);
    let reason = upgrade::upgrade(&root, &verified, &mut p).err().expect("must stop");
    println!("  reason:\n{reason}");
    assert!(reason.contains("请先关闭监控台"));
    assert!(!p.has("逐项移开"));
    assert_eq!(top(&root), top_before);
    end(fake);
    fs::rename(&acui, root.join("ui").join("acui.exe.oneoff-ping")).unwrap();
    fs::rename(&real, &acui).unwrap();
    println!("PASS T7");

    println!("== T8 A5: a listener that does not answer, on a free port through the port parameter");
    let port = free_port(5101);
    let script = format!(
        "$l = [System.Net.Sockets.TcpListener]::new([System.Net.IPAddress]::Loopback, {port}); $l.Start(); $held = [System.Collections.Generic.List[object]]::new(); while ($true) {{ $held.Add($l.AcceptTcpClient()) }}"
    );
    let mut listener = Command::new("pwsh")
        .args(["-NoProfile", "-NonInteractive", "-Command", &script])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(60);
    while !platform::loopback_listeners(port).unwrap().contains(&listener.id()) {
        assert!(Instant::now() < deadline, "the stand-in listener never listened");
        std::thread::sleep(Duration::from_millis(250));
    }
    println!(
        "  stand-in listener on 127.0.0.1:{port}: PID {} · {:?} (accepts, never answers)",
        listener.id(),
        platform::process_image(listener.id())
    );
    let mut p = Printer::new();
    let started = Instant::now();
    let server = adb_server::ensure(&upgrade::ac_adb(&root), &root, port, &mut p).unwrap();
    println!("  took {:.1} s · {}", started.elapsed().as_secs_f64(), show(&server));
    assert!(matches!(server, AdbServer::Ready(_)));
    println!("  first attempt timed out: {}", p.has("did not finish within 20 s"));
    assert!(p.has(&format!("ending the process listening on 127.0.0.1:{port}: PID {}", listener.id())));
    assert!(listener.try_wait().unwrap().is_some(), "the listener was ended");
    for pid in platform::loopback_listeners(port).unwrap() {
        println!("  now listening on 127.0.0.1:{port}: PID {pid} · {:?}", platform::process_image(pid));
    }
    println!("PASS T8");

    println!("== T9 A5: a second failure is said loudly; an adb that cannot run ends nothing");
    let fake_dir = base.join("fake-adb");
    fs::create_dir_all(&fake_dir).unwrap();
    fs::copy(&ping, fake_dir.join("adb.exe")).unwrap();
    let port = free_port(5201);
    let mut p = Printer::new();
    let server = adb_server::ensure(&fake_dir.join("adb.exe"), &root, port, &mut p).unwrap();
    println!("  {}", show(&server));
    assert!(matches!(server, AdbServer::NotReady(_)));
    assert!(p.has("NOTE: ADB 服务未就绪"));
    let summary = summary_with(&root, &wanted, &up_like(&root), server);
    println!("  finish page summary:\n{summary}");
    assert!(summary.contains("NOT ready") && summary.contains("ADB 服务未就绪"));
    let mut p = Printer::new();
    let server = adb_server::ensure(&base.join("missing").join("adb.exe"), &root, port, &mut p).unwrap();
    println!("  {}", show(&server));
    assert!(matches!(server, AdbServer::NotReady(ref why) if why.contains("nothing was ended")));
    assert!(!p.has("ending the process"));
    println!("PASS T9");

    end(holder_ui);
    println!("ALL PASS");
}

fn sha(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    verify::hex(&Sha256::digest(bytes))
}

/// An `Upgraded` shaped like the one an upgrade of `root` returns.
fn up_like(root: &Path) -> (crate::install::LaidOut, PathBuf) {
    (
        crate::install::LaidOut {
            ui_dir: root.join("ui"),
            tools_dir: root.join("tools"),
            actingd_exe: root.join("runtime").join("actingcommand-actingd.exe"),
            acui_exe: root.join("ui").join("acui.exe"),
        },
        root.join("previous"),
    )
}

/// The finish page's summary for an upgrade whose ADB check ended in `server`.
fn summary_with(
    root: &Path,
    wanted: &(String, String),
    laid: &(crate::install::LaidOut, PathBuf),
    server: AdbServer,
) -> String {
    let state: super::Shared = std::sync::Arc::new(std::sync::Mutex::new(super::State::default()));
    {
        let mut locked = super::lock(&state);
        locked.root = root.to_path_buf();
        locked.members = Some(wanted.clone());
        locked.source = "oneoff".into();
        locked.installed = upgrade::installed(root).unwrap();
        if let AdbServer::NotReady(why) = &server {
            locked.warnings.push(format!(
                "ADB 服务未就绪（升级已完成）/ The ADB server is not ready (the upgrade is in place): {why}"
            ));
        }
        locked.laid_out = Some(laid.0.clone());
        locked.upgraded = Some(Upgraded {
            laid_out: laid.0.clone(),
            previous: laid.1.clone(),
            restarted: None,
            adb_server: server,
        });
    }
    super::summary(&state)
}
