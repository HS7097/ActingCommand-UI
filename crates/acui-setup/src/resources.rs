// SPDX-License-Identifier: GPL-3.0-only
//! The resource-only update (Workflow #364 item B): one resource repository bundle (v2 or v3)
//! into an existing A/B installation, the programs and the slot unchanged. Its packs are laid
//! out under `packages\<game>\<digest>\`, the maintenance bindings it declares (startup,
//! prerequisite, return-home) are planned with the same association and conflict rules as an
//! upgrade (#359), a new configuration generation is checked by the selected slot's
//! `check-config` and committed, and a Runtime that was running is drained, closed and started
//! again on it.
//!
//! It mirrors the coordinator's stopgap step by step — `place_bundle.py` (the zip against its
//! SHA256SUMS line, every pack's digest, present / new / present-but-different) and `rebind.py`
//! (bindings from the declaration, commit through the generation writer) — and adds the v3
//! qualification the scripts lack. Nothing here deletes or moves a pack directory: one that is
//! there but differs refuses the run. A plan (`--plan`) runs the same checks and changes nothing
//! under the root.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use acui_installation::Snapshot;
use serde_json::Value;

use crate::bundle::{self, Bundle, OnDiffers};
use crate::generations::{self, Writer};
use crate::install::LaidOut;
use crate::interfaces::{self, Agreed, Stop};
use crate::maintenance::{self, Conflict, Prepared, Side};
use crate::verify::{self, Report, MISMATCH};
use crate::{lifecycle, runtime, upgrade};

/// What the Runtime went through.
pub enum Runtime {
    /// The configuration did not change: the Runtime was not touched.
    Untouched,
    /// It was not running: closed through the cold gate and left stopped.
    Stopped,
    /// Drained, closed and started again on the new generation; its process log.
    Restarted(PathBuf),
}

/// What a resource-only update did, for its summary.
pub struct Outcome {
    pub zip: PathBuf,
    pub game: String,
    pub source: Option<String>,
    pub placed: usize,
    pub reused: usize,
    /// Every binding changed, `<key>: <old> → <new>`.
    pub changes: Vec<String>,
    /// The generation selected; `None` when the configuration did not change.
    pub generation: Option<u64>,
    pub runtime: Runtime,
    pub slot: String,
    /// The selected slot's programs, `(runtime, ui)` commits: unchanged.
    pub programs: (String, String),
    /// The installation's fixed entries (`upgrade::outcome`, review L6).
    pub laid_out: LaidOut,
}

/// Everything steps 2 to 7 established, nothing under the installation changed yet.
struct Checked {
    baseline: Snapshot,
    config: PathBuf,
    bundle: Bundle,
    agreed: Agreed,
    prepared: Vec<Prepared>,
    reused: usize,
    new: usize,
    /// The current configuration, its paths made absolute as the generation writer does.
    base: Value,
    /// `base` with the bundle's bindings proposed; conflicts still unanswered.
    document: Value,
    conflicts: Vec<Conflict>,
}

/// The installation a resource-only update needs: an A/B one. The reason otherwise.
pub fn installation(root: &Path) -> Result<(), String> {
    if root
        .join(acui_installation::INSTALL_SELECTION_PATH)
        .try_exists()
        .map_err(|error| format!("Cannot inspect installation selection: {error}"))?
    {
        return Ok(());
    }
    match upgrade::installed(root)? {
        Some(_) => Err("只更新资源需要 A/B 安装，请先做一次完整升级 / A resource-only update needs an A/B installation; upgrade first".into()),
        None => Err(format!("此处没有安装 / Nothing is installed here: {}", root.display())),
    }
}

/// Steps 2 to 7: the zip against its SHA256SUMS line, the bundle read (v2/v3 only), the
/// interfaces of the selected slot, this installer and the bundle, every pack staged and
/// admitted (and a v3 declaration qualified), each pack against what the root already holds,
/// and the bindings the bundle proposes. Nothing under the installation changes.
fn check(
    root: &Path,
    zip: &Path,
    sums: Option<&Path>,
    staging: &Path,
    choose: maintenance::Choose<'_>,
    report: Report<'_>,
) -> Result<Checked, Stop> {
    // 2. The zip's SHA256SUMS line (place_bundle.py:22-28); other entries are not read.
    let name = zip
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| format!("标准包文件名不能使用 / the bundle file name cannot be used: {}", zip.display()))?;
    let sums = match sums {
        Some(sums) => sums.to_path_buf(),
        None => zip
            .parent()
            .ok_or_else(|| format!("标准包没有所在文件夹 / the bundle has no folder: {}", zip.display()))?
            .join("SHA256SUMS"),
    };
    let text = fs::read_to_string(&sums)
        .map_err(|error| format!("缺少文件 / missing: {}: {error}", sums.display()))?;
    let listed: Vec<String> = verify::parse_sha256sums(&text)?
        .into_iter()
        .filter(|(_, listed)| listed == name)
        .map(|(hash, _)| hash)
        .collect();
    if listed.is_empty() {
        return Err(Stop::Failed(format!(
            "{} 未列出 / does not list {name}：用与标准包同一处下载的 SHA256SUMS，或用 --sums 指明 / use the SHA256SUMS downloaded with the bundle, or name it with --sums\n安装未改动 / The installation was not changed",
            sums.display()
        )));
    }
    let actual = verify::sha256_file(zip)
        .map_err(|error| format!("读取失败 / read failed: {}: {error}", zip.display()))?;
    if listed.iter().any(|expected| *expected != actual) {
        return Err(Stop::Failed(format!(
            "{MISMATCH}: {name} sha256 应为 / expected {}（{}），实为 / actual {actual}\n安装未改动 / The installation was not changed",
            listed.join(" / "),
            sums.display()
        )));
    }
    report.line(&format!(
        "SHA256SUMS 已核对 / ok: {name}（{}）",
        sums.display()
    ))?;
    // 3. The bundle (place_bundle.py:29-31): content directories only.
    let bundle = bundle::read(zip)?;
    if !bundle.content_directories() {
        return Err(Stop::Failed(format!(
            "只更新资源只接受 bundle v2/v3（内容目录）；v1 标准包请走完整升级 / A resource-only update takes a bundle v2 or v3 (content directories); a v1 bundle goes through a full upgrade: {}",
            zip.display()
        )));
    }
    report.line(&format!(
        "标准包 / Bundle: {name} → {}（game {}，bundle {}，source {}，{} 个包 / packs）",
        bundle.name(),
        bundle.game,
        bundle.format,
        bundle.source.as_deref().unwrap_or("—"),
        bundle.packs.len()
    ))?;
    // 4. The selected generation (its configuration bound by active.json, rebind.py:24-28)
    // and the interfaces of its programs, this installer and the bundle.
    let baseline = Snapshot::read(root)?;
    report.line(&format!(
        "当前选择 / Selected: slot {} · generation {}",
        baseline.selection.slot.as_str(),
        baseline.selection.generation
    ))?;
    let agreed = interfaces::selected(root, std::slice::from_ref(&bundle), report)?;
    // 5. Every pack staged and admitted, a v3 declaration qualified (place_bundle.py:33-42).
    let prepared = maintenance::prepare(
        std::slice::from_ref(&bundle),
        &staging.join("resource-packages"),
        report,
    )?;
    // 6. Each pack against what the root already holds (place_bundle.py:43-50).
    let (reused, new, differs) = bundle.presence(&root.join("packages").join(&bundle.game))?;
    report.line(&format!(
        "{name}: game {}，{} 个包 / packs；已有 / present {reused}，新 / new {new}，已有但不同 / present-but-different {}",
        bundle.game,
        bundle.packs.len(),
        differs.len()
    ))?;
    if !differs.is_empty() {
        return Err(Stop::Failed(format!(
            "{MISMATCH}: 已有但内容不同的任务包目录 / pack directories present but different:\n  {}\n只更新资源不挪动也不删除它们 / A resource-only update neither moves nor deletes them. {}\n安装未改动 / The installation was not changed",
            differs.join("\n  "),
            bundle::DIFFERS_REMEDY
        )));
    }
    // 7. The bindings the bundle declares (rebind.py:34-76; declared bindings only, ruling Q9).
    let config = baseline.config_path()?;
    let mut base: Value = serde_json::from_slice(&baseline.config_bytes)
        .map_err(|error| format!("Selected configuration is unreadable: {error}"))?;
    generations::rebase_config(
        &mut base,
        config
            .parent()
            .ok_or_else(|| "Configuration has no parent".to_string())?,
    )?;
    let mut document = base.clone();
    let selections = maintenance::upgrade_selections(&mut document, root, &prepared, choose, report)?;
    let old_source = generations::plain(&config).display().to_string();
    let (_, conflicts) =
        maintenance::augment(&mut document, root, &prepared, &selections, &old_source)?;
    Ok(Checked {
        baseline,
        config,
        bundle,
        agreed,
        prepared,
        reused,
        new,
        base,
        document,
        conflicts,
    })
}

/// A binding that is not there.
static NULL: Value = Value::Null;

/// A binding as one line: `—` for none, a plain path, or `maintenance::brief`.
fn text(value: &Value) -> String {
    match value {
        Value::Null => "—".to_string(),
        Value::String(path) => generations::plain(Path::new(path)).display().to_string(),
        other => maintenance::brief(other),
    }
}

/// Every binding `document` changes against `base`, however its paths are spelled (review L5):
/// each instance's `resource_package` and `startup_package`, each prerequisite by package id,
/// each return-home binding by game and server.
fn changes(base: &Value, document: &Value, root: &Path) -> Vec<String> {
    let mut lines = Vec::new();
    let empty = Vec::new();
    let before = base["instances"].as_array().unwrap_or(&empty);
    for (at, instance) in document["instances"].as_array().unwrap_or(&empty).iter().enumerate() {
        for key in ["resource_package", "startup_package"] {
            let old = before.get(at).map_or(&NULL, |old| &old[key]);
            let new = &instance[key];
            if !maintenance::same_binding(old, new, root) {
                lines.push(format!(
                    "instances[{at}].{key}（{}）: {} → {}",
                    maintenance::alias_of(instance, at),
                    text(old),
                    text(new)
                ));
            }
        }
    }
    let keyed = |field: &str, key: fn(&Value) -> String, lines: &mut Vec<String>| {
        let by_key = |document: &Value| -> Vec<(String, Value)> {
            document[field]
                .as_array()
                .map(|items| items.iter().map(|item| (key(item), item.clone())).collect())
                .unwrap_or_default()
        };
        let (old, new) = (by_key(base), by_key(document));
        for (name, value) in &new {
            let previous = old
                .iter()
                .find(|(known, _)| known == name)
                .map_or(&NULL, |(_, value)| value);
            if !maintenance::same_binding(previous, value, root) {
                lines.push(format!("{field}[{name}]: {} → {}", text(previous), text(value)));
            }
        }
        for (name, value) in &old {
            if !new.iter().any(|(known, _)| known == name) {
                lines.push(format!("{field}[{name}]: {} → —", text(value)));
            }
        }
    };
    keyed(
        "prerequisite_packages",
        |item: &Value| item["package_id"].as_str().unwrap_or("?").to_string(),
        &mut lines,
    );
    keyed(
        "return_home_packages",
        |item: &Value| {
            format!(
                "{}/{}",
                item["game"].as_str().unwrap_or("?"),
                item["server"].as_str().unwrap_or("?")
            )
        },
        &mut lines,
    );
    lines
}

/// Step 8's account of the binding changes.
fn describe(changes: &[String], report: Report<'_>) -> Result<(), String> {
    if changes.is_empty() {
        return report.line("维护绑定没有变化 / No maintenance binding changes");
    }
    report.line(&format!(
        "维护绑定将改变 / Maintenance bindings that change: {}",
        changes.len()
    ))?;
    for line in changes {
        report.line(&format!("  {line}"))?;
    }
    Ok(())
}

/// Step 8: whether the selected Runtime answers, and what a real run then does with it.
fn probe(baseline: &Snapshot, report: Report<'_>) -> Result<(), String> {
    let mut status = lifecycle::command(
        &baseline.slot_root().join("runtime").join(runtime::ACTINGCTL),
        Some(baseline),
    )?;
    status
        .arg("status")
        .arg("--state-root")
        .arg(baseline.state_root()?);
    report.line(&match runtime::run_observed(&mut status, Duration::from_secs(75)) {
        Ok(output) if output.success => "Runtime 正在运行：配置有变化时，真实运行先排空并关闭它，提交新配置后再拉起 / The Runtime is running: when the configuration changes, a real run drains and closes it first and starts it again after the commit".to_string(),
        Ok(output) => format!(
            "Runtime 未应答（退出 {}）：配置有变化时，真实运行先做冷态账本验证，提交后它保持停止 / The Runtime does not answer (exit {}): when the configuration changes, a real run verifies the ledger cold first, and it stays stopped after the commit",
            output.exit, output.exit
        ),
        Err(reason) => format!("Runtime 状态未能查询 / The Runtime's status could not be asked: {reason}"),
    })
}

fn outcome(
    zip: &Path,
    checked: &Checked,
    changes: Vec<String>,
    snapshot: &Snapshot,
    generation: Option<u64>,
    runtime: Runtime,
) -> Result<Outcome, String> {
    let programs = verify::members_of(
        std::str::from_utf8(&snapshot.members_bytes()?).map_err(|error| error.to_string())?,
    )?;
    Ok(Outcome {
        zip: zip.to_path_buf(),
        game: checked.bundle.game.clone(),
        source: checked.bundle.source.clone(),
        placed: checked.new,
        reused: checked.reused,
        changes,
        generation,
        runtime,
        slot: snapshot.selection.slot.as_str().to_string(),
        programs,
        laid_out: upgrade::outcome(snapshot, PathBuf::new(), None).laid_out,
    })
}

/// The real run, steps 1 to 17 of the model, under the writer lock from start to end. The
/// caller made the log and owns `staging`, which it removes afterwards.
pub fn run(
    root: &Path,
    zip: &Path,
    sums: Option<&Path>,
    staging: &Path,
    choose: maintenance::Choose<'_>,
    resolve: maintenance::Resolve<'_>,
    report: Report<'_>,
) -> Result<Outcome, Stop> {
    // 1. The writer lock excludes --commit-config, upgrades and a second run.
    let writer = Writer::acquire(root)?;
    let root = writer.root();
    let mut checked = check(root, zip, sums, staging, choose, report)?;
    maintenance::decide(&mut checked.document, &checked.conflicts, resolve, report)?;
    // 8. What changes.
    let changes = changes(&checked.base, &checked.document, root);
    describe(&changes, report)?;
    probe(&checked.baseline, report)?;
    // 9. Nothing to do.
    if checked.new == 0 && changes.is_empty() {
        report.line("无需改动：任务包都已就位，维护绑定没有变化 / Nothing to do: every pack is in place and no maintenance binding changes")?;
        let snapshot = checked.baseline.clone();
        return Ok(outcome(zip, &checked, changes, &snapshot, None, Runtime::Untouched)?);
    }
    // 10. The new pack directories only, each through `<digest>.part` (place_bundle.py:51-56);
    // a directory that came to differ since step 6 refuses (ruling Q4).
    maintenance::place(&checked.prepared, root, OnDiffers::Refuse, report).map_err(|error| {
        format!("{error}\n已放入的新任务包目录保留，不删除；选择未变，Runtime 未触动 / The new pack directories already placed are kept, never deleted; the selection is unchanged and the Runtime untouched")
    })?;
    // 11. The configuration did not change: the packs are placed, nothing else.
    if changes.is_empty() {
        report.line("配置不变：未生成新代际，Runtime 未重启 / Configuration unchanged: no new generation, the Runtime was not restarted")?;
        let snapshot = checked.baseline.clone();
        return Ok(outcome(zip, &checked, changes, &snapshot, None, Runtime::Untouched)?);
    }
    // 12. A new, unselected generation: the full chain qualified, the selected slot's
    // check-config (rebind.py:81).
    let slot = checked.baseline.selection.slot;
    let mut plan = writer
        .prepare(
            Some(checked.baseline.clone()),
            slot,
            &checked.config,
            checked.document.clone(),
            true,
            report,
        )
        .map_err(|error| {
            format!("{error}\n新任务包目录保留；选择未变，Runtime 未触动 / The new pack directories are kept; the selection is unchanged and the Runtime untouched")
        })?;
    // 13. Nobody changed the installation meanwhile (deploy_v9.py:43-46).
    checked.baseline.unchanged().map_err(|error| {
        format!(
            "{error}\n新任务包目录与未选中的配置代际 {} 保留；选择未变，Runtime 未触动 / The new pack directories and the unselected generation {} are kept; the selection is unchanged and the Runtime untouched",
            plan.snapshot.selection.generation,
            plan.snapshot.selection.generation
        )
    })?;
    // 14. Close: Host drain and commit_shutdown when it runs, the cold gate when not.
    let closed = lifecycle::close(
        &checked.baseline.slot_root(),
        &checked.config,
        &checked.baseline.state_root()?,
        Some(&checked.baseline),
        checked.agreed.closing()?,
        report,
    )?;
    // 15. Commit; on failure the original selection comes back and the Runtime stays stopped.
    let committed = writer.commit(&mut plan).and_then(|()| {
        report.line("安装选择已原子提交 / Installation selection committed atomically")
    });
    if let Err(error) = committed {
        return Err(Stop::Failed(match writer.restore_before_start(&mut plan) {
            Ok(()) => format!("{error}; original selection restored; Runtime remains stopped; placed packs kept"),
            Err(restore) => format!(
                "{error}; selection restoration failed: {restore}; Runtime remains stopped; placed packs kept"
            ),
        }));
    }
    // 16. Started again (held, then released) when it was running; otherwise it stays stopped.
    let was_running = closed.was_running;
    if was_running {
        plan.mark_start_attempt()?;
    }
    let restarted = lifecycle::start(&plan.snapshot, closed, checked.agreed.start, report)?;
    let runtime = match restarted {
        Some(log) => Runtime::Restarted(log),
        None => Runtime::Stopped,
    };
    Ok(outcome(
        zip,
        &checked,
        changes,
        &plan.snapshot,
        Some(plan.snapshot.selection.generation),
        runtime,
    )?)
}

/// What a plan found: whether it can tell the outcome, and the conflicts it leaves open.
pub struct Planned {
    pub conflicts: usize,
}

/// `--plan`: steps 2 to 9 with staging under the plan's scratch directory; nothing under the
/// root changes (review M3). With `conflicts` given, its answer is applied to say what a real
/// run would select.
pub fn plan(
    root: &Path,
    zip: &Path,
    sums: Option<&Path>,
    staging: &Path,
    choose: maintenance::Choose<'_>,
    conflicts: Option<Side>,
    report: Report<'_>,
) -> Result<Planned, Stop> {
    let root = fs::canonicalize(root)
        .map_err(|error| format!("Cannot resolve installation root: {error}"))?;
    let mut checked = check(&root, zip, sums, staging, choose, report)?;
    maintenance::list(&checked.conflicts, report)?;
    let open = checked.conflicts.len();
    if open > 0 {
        let Some(side) = conflicts else {
            report.line("有维护绑定差异：真实运行需要 --conflicts new|old，配置是否改变取决于它 / There are maintenance binding differences: a real run needs --conflicts new|old, and whether the configuration changes depends on it")?;
            probe(&checked.baseline, report)?;
            return Ok(Planned { conflicts: open });
        };
        maintenance::apply(&mut checked.document, &checked.conflicts, &vec![side; open])?;
        report.line(match side {
            Side::New => "--conflicts new：真实运行全部采用新值 / a real run takes every proposed value",
            Side::Old => "--conflicts old：真实运行全部保留旧值 / a real run keeps every current value",
        })?;
    }
    let changes = changes(&checked.base, &checked.document, &root);
    describe(&changes, report)?;
    probe(&checked.baseline, report)?;
    report.line(match (checked.new, changes.is_empty()) {
        (0, true) => "无需改动：任务包都已就位，维护绑定没有变化 / Nothing to do: every pack is in place and no maintenance binding changes",
        (_, true) => "配置不变：只放入新任务包，不生成新代际，不重启 Runtime / Configuration unchanged: only the new packs are placed; no new generation, no Runtime restart",
        (_, false) => "生成新的配置代际，经选中槽的 check-config 后提交 / A new configuration generation, checked by the selected slot's check-config, then committed",
    })?;
    Ok(Planned { conflicts: open })
}

/// The summary lines (model §2.2.5) after the install root; the caller adds the log and notes.
pub fn summary(outcome: &Outcome) -> Vec<String> {
    let short = |sha: &str| sha.get(..8).unwrap_or(sha).to_string();
    let mut lines = vec![
        format!(
            "只更新资源 / Resources updated: {}（{}，source {}）",
            outcome.zip.display(),
            outcome.game,
            outcome
                .source
                .as_deref()
                .map_or_else(|| "—".to_string(), |source| source.chars().take(12 + source.find('@').map_or(0, |at| at + 1)).collect())
        ),
        format!(
            "任务包 / Packs: 新放入 / placed {} · 已有并复用 / reused {}",
            outcome.placed, outcome.reused
        ),
    ];
    if outcome.changes.is_empty() {
        lines.push("维护绑定 / Bindings: 没有变化 / none changed".to_string());
    } else {
        lines.push("维护绑定 / Bindings:".to_string());
        lines.extend(outcome.changes.iter().map(|line| format!("  {line}")));
    }
    lines.push(match outcome.generation {
        Some(generation) => format!("选中配置代际 / Selected configuration generation: {generation}"),
        None => "配置不变 / Configuration unchanged".to_string(),
    });
    lines.push(match &outcome.runtime {
        Runtime::Restarted(log) => format!(
            "Runtime 已按新配置重新拉起 / restarted on the new configuration; 日志 / log: {}",
            log.display()
        ),
        Runtime::Stopped => "Runtime 之前未在运行，未拉起 / was not running, not started".to_string(),
        Runtime::Untouched => "Runtime 未触动 / The Runtime was not touched".to_string(),
    });
    lines.push(format!(
        "程序与槽未变 / Programs and slot unchanged: slot {} · runtime {} · ui {}",
        outcome.slot,
        short(&outcome.programs.0),
        short(&outcome.programs.1)
    ));
    lines
}
