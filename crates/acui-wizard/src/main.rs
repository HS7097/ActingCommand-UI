// SPDX-License-Identifier: GPL-3.0-only
//! `acsetup`, the 0.12 setup wizard. This slice (UI-W2a) has the command line only; the window
//! comes with UI-W2b and shares the whole back end (`acui_wizard`).
//!
//! Exit codes: 0 done, 1 failed (stderr says which step and why), 2 usage.

use std::ffi::OsString;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use acui_wizard::manifest::Program;
use acui_wizard::mumu::{self, Discovery};
use acui_wizard::occupancy::{self, OtherRoot};
use acui_wizard::plan::parse_plan;
use acui_wizard::root::size_text;
use acui_wizard::set::{verify_set, VerifiedSet};
use acui_wizard::{install, Event, Request, Said, Step};

const USAGE: &str = "\
acsetup — ActingCommand 安装向导 / setup wizard (0.12)

  acsetup check-set --set <安装集文件夹 / install set folder>
      只核对安装集，打印摘要 / checks the install set only and prints what it holds
  acsetup discover [--mumu-root <MuMu 文件夹 / folder>]
      列出 MuMu 实例和别的 ActingCommand 用着的实例（只读）
      / lists the MuMu instances and those another ActingCommand uses (read only)
  acsetup install --set <文件夹 / folder> --root <新的安装文件夹 / new install folder> --plan <plan.json>
      照计划安装 / installs as the plan says

退出码 / exit codes: 0 完成 / done, 1 失败 / failed, 2 用法 / usage";

const SET_LIMIT: u64 = 1 << 20;

fn main() -> ExitCode {
    let arguments: Vec<OsString> = std::env::args_os().skip(1).collect();
    let Some(command) = arguments.first().and_then(|word| word.to_str()).map(str::to_owned) else {
        eprintln!("{USAGE}");
        return ExitCode::from(2);
    };
    let flags = match parse_flags(&arguments[1..]) {
        Ok(flags) => flags,
        Err(problem) => return usage(&problem),
    };
    match command.as_str() {
        "-h" | "--help" | "help" if arguments.len() == 1 => {
            println!("{USAGE}");
            ExitCode::SUCCESS
        }
        "check-set" => match only(&flags, &["--set"]).and_then(|()| required(&flags, "--set")) {
            Ok(set) => check_set(&set),
            Err(problem) => usage(&problem),
        },
        "discover" => match only(&flags, &["--mumu-root"]) {
            Ok(()) => discover(flags.iter().find(|(flag, _)| flag == "--mumu-root").map(|(_, value)| PathBuf::from(value))),
            Err(problem) => usage(&problem),
        },
        "install" => {
            let parsed = only(&flags, &["--set", "--root", "--plan"]).and_then(|()| {
                Ok((required(&flags, "--set")?, required(&flags, "--root")?, required(&flags, "--plan")?))
            });
            match parsed {
                Ok((set, root, plan)) => run_install(&set, &root, &plan),
                Err(problem) => usage(&problem),
            }
        }
        other => usage(&format!("unknown command {other}")),
    }
}

fn usage(problem: &str) -> ExitCode {
    eprintln!("{problem}\n\n{USAGE}");
    ExitCode::from(2)
}

/// `--flag value` pairs, each flag once.
fn parse_flags(words: &[OsString]) -> Result<Vec<(String, OsString)>, String> {
    let mut flags: Vec<(String, OsString)> = Vec::new();
    let mut words = words.iter();
    while let Some(word) = words.next() {
        let Some(flag) = word.to_str().filter(|flag| flag.starts_with("--")) else {
            return Err(format!("unexpected argument {}", word.to_string_lossy()));
        };
        let Some(value) = words.next() else {
            return Err(format!("{flag} needs a value"));
        };
        if flags.iter().any(|(seen, _)| seen == flag) {
            return Err(format!("{flag} is given twice"));
        }
        flags.push((flag.to_owned(), value.clone()));
    }
    Ok(flags)
}

fn only(flags: &[(String, OsString)], allowed: &[&str]) -> Result<(), String> {
    match flags.iter().find(|(flag, _)| !allowed.contains(&flag.as_str())) {
        Some((flag, _)) => Err(format!("unknown option {flag}")),
        None => Ok(()),
    }
}

/// The flag's value as an absolute path.
fn required(flags: &[(String, OsString)], name: &str) -> Result<PathBuf, String> {
    let value = flags
        .iter()
        .find(|(flag, _)| flag == name)
        .map(|(_, value)| PathBuf::from(value))
        .ok_or_else(|| format!("{name} is required"))?;
    std::path::absolute(&value).map_err(|error| format!("{name} {}: {error}", value.display()))
}

fn failed(said: &Said) -> ExitCode {
    eprintln!("失败 / failed: {said}");
    ExitCode::FAILURE
}

/// Prints the set's events: lines as they come, the hashing every tenth.
fn printer() -> impl FnMut(Event) {
    let mut shown = u64::MAX;
    move |event| match event {
        Event::Started(step) => println!("[{}/{}] {}", step.number(), Step::ALL.len(), step.title()),
        Event::Finished(step, said) => println!("[{}/{}] 完成 / done: {said}", step.number(), Step::ALL.len()),
        Event::Skipped(step, said) => println!("[{}/{}] 跳过 / skipped: {said}", step.number(), Step::ALL.len()),
        Event::Failed(step, said) => eprintln!("[{}/{}] 失败 / failed: {said}", step.number(), Step::ALL.len()),
        Event::Line(said) => println!("  {said}"),
        Event::Bytes { done, total } => {
            let tenth = if total == 0 { 10 } else { done.saturating_mul(10) / total };
            if tenth != shown {
                shown = tenth;
                println!("  … {}% ({} / {})", tenth * 10, size_text(done), size_text(total));
            }
        }
    }
}

fn check_set(dir: &Path) -> ExitCode {
    let mut print = printer();
    let set = match verify_set(dir, &mut print) {
        Ok(set) => set,
        Err(said) => return failed(&said),
    };
    print_set(&set);
    println!("全部核对通过 / every file checked");
    ExitCode::SUCCESS
}

fn print_set(set: &VerifiedSet) {
    let runtime = &set.set.runtime;
    let console = &set.set.console;
    println!("安装集 / install set: {}", set.set.label);
    println!(
        "  Runtime {} {} (package [{}, {}])",
        runtime.tag, runtime.commit, set.package_range[0], set.package_range[1]
    );
    println!("  监控台 / console {} {}", console.tag, console.commit);
    for program in Program::ALL {
        let verified = set.program(program);
        println!(
            "  {}: {} files, {}",
            program.label(),
            verified.manifest.files.len(),
            size_text(verified.bytes)
        );
    }
    for bundle in &set.bundles {
        let servers: Vec<String> = bundle
            .game
            .servers
            .iter()
            .map(|server| format!("{} {} ({})", server.server, server.label, server.application_id))
            .collect();
        println!(
            "  游戏 / game {} 「{}」: {} packs, package [{}, {}], servers {}",
            bundle.entry.game,
            bundle.game.label,
            bundle.index.packs.len(),
            bundle.package[0],
            bundle.package[1],
            servers.join("; ")
        );
    }
    println!(
        "  视觉 / vision {}: {} files, {}",
        set.set.vision.provider,
        set.set.vision.files.len(),
        size_text(set.vision_bytes)
    );
    println!(
        "  全部游戏约需 / all games need about {}",
        size_text(set.space_needed(&set.bundles.iter().map(|bundle| bundle.entry.game.clone()).collect::<Vec<_>>()))
    );
}

fn discover(given: Option<PathBuf>) -> ExitCode {
    let mut notes = Vec::new();
    let discovery = mumu::discover(given.as_deref(), &mut |said: Said| notes.push(said));
    let others = occupancy::scan(&mut |said: Said| notes.push(said));
    for note in &notes {
        println!("  {note}");
    }
    print_discovery(&discovery, &others);
    match (&discovery.problem, discovery.absent) {
        (Some(problem), false) => failed(problem),
        _ => ExitCode::SUCCESS,
    }
}

fn print_discovery(discovery: &Discovery, others: &[OtherRoot]) {
    match &discovery.found {
        Some(found) => println!(
            "MuMu: {} ({}), {}, version {}",
            found.root.display(),
            found.source.said(),
            found.manager.display(),
            discovery.version.as_deref().unwrap_or("?")
        ),
        None => println!("MuMu: -"),
    }
    if let Some(problem) = &discovery.problem {
        println!("  {problem}");
    }
    for instance in &discovery.instances {
        let state = if instance.running { "运行中 / running" } else { "未运行 / stopped" };
        let used = match occupancy::used_by(others, instance) {
            Some((other, alias)) => format!(
                "；已被别的 ActingCommand 用着 / in use by another ActingCommand: {} ({alias})",
                other.root.as_ref().map_or_else(|| format!("pid {}", other.pid), |root| root.display().to_string())
            ),
            None => String::new(),
        };
        println!("  #{} {} [{state}]{used}", instance.index, instance.name);
    }
    for other in others {
        let root = other.root.as_ref().map_or_else(|| "?".to_owned(), |root| root.display().to_string());
        match (&other.instances, &other.unknown) {
            (Some(instances), _) => {
                let listed: Vec<String> = instances
                    .iter()
                    .map(|used| match (used.index, &used.name) {
                        (Some(index), _) => format!("{} #{index}", used.alias),
                        (None, Some(name)) => format!("{} \"{name}\"", used.alias),
                        (None, None) => format!("{} (adb)", used.alias),
                    })
                    .collect();
                println!("ActingCommand pid {} {root}: {}", other.pid, listed.join(", "));
            }
            (None, Some(reason)) => println!("ActingCommand pid {} {root}: ? ({reason})", other.pid),
            (None, None) => println!("ActingCommand pid {} {root}: ?", other.pid),
        }
    }
}

fn run_install(set_dir: &Path, root: &Path, plan_path: &Path) -> ExitCode {
    let plan = match read_small(plan_path).and_then(|bytes| parse_plan(&bytes)) {
        Ok(plan) => plan,
        Err(said) => return failed(&said),
    };
    let request = Request {
        set_dir: set_dir.to_path_buf(),
        root: root.to_path_buf(),
        plan,
    };
    let mut print = printer();
    match install(&request, &mut print) {
        Ok(done) => {
            println!("安装完成 / installed: {}", done.root.display());
            println!("  监控台 / console: {}", done.console.display());
            println!("  配置 / configuration: {}", done.config.display());
            println!("  日志 / log: {}", done.log.display());
            for shortcut in &done.shortcuts {
                println!("  快捷方式 / shortcut: {}", shortcut.display());
            }
            if done.paused {
                println!("  所有号已暂停：在监控台确认无误后点「全部开始执勤」 / every account is paused: after a look in the console, press \"put all on duty\"");
            }
            if done.watchdog {
                println!("  看门狗已装；删除这个文件夹前先在监控台设置页或用 actingctl watchdog uninstall 停掉 / the watchdog is installed; before deleting this folder, remove it in the console's settings or with actingctl watchdog uninstall");
            }
            ExitCode::SUCCESS
        }
        Err(failure) => {
            eprintln!("{failure}");
            if let Some(log) = &failure.log {
                eprintln!("日志 / log: {}", log.display());
            }
            if failure.wrote {
                eprintln!(
                    "已写的东西原样留着；这个文件夹可以整个删掉重来（向导不自动删） / what was written stays as it is; the folder may be deleted whole and the install started again (the wizard deletes nothing): {}",
                    root.display()
                );
            }
            ExitCode::FAILURE
        }
    }
}

fn read_small(path: &Path) -> Result<Vec<u8>, Said> {
    let unreadable = |error: &dyn std::fmt::Display| {
        Said::new("文件读不出", "the file cannot be read").with(format!("{}: {error}", path.display()))
    };
    let file = fs::File::open(path).map_err(|error| unreadable(&error))?;
    let mut bytes = Vec::new();
    file.take(SET_LIMIT + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| unreadable(&error))?;
    if bytes.len() as u64 > SET_LIMIT {
        return Err(unreadable(&"larger than 1 MiB"));
    }
    Ok(bytes)
}
