// SPDX-License-Identifier: GPL-3.0-only
//! The account cards, from their views and the table of the language shown, into
//! the list the workbench already shows: a row changes only when its words do, and
//! rows come and go only when the accounts do, so a read every few seconds costs
//! nothing and moves nothing. And the top bar's writes, its performance preset and the
//! stepper under it, from their views; which page shows; the settings page's rows of
//! the Runtime's settings, the same way as the cards, the editor of every account's quiet times
//! heading the everyday ones (UI-3e1-c), with the field of the evidence days among them and each
//! device's group of its backends after them; what the start report says
//! of the settings; and whether the Runtime is started again when it is gone. And the right
//! column's live view, from its view, the frame's pixels once when it lands; and under it the
//! picture's refresh, painted with it. And over the picture the account's rescue bar, and while
//! it says so the account's rules panel in the picture's place (UI-3e1-b). And which of the right
//! column's two tabs shows, and under the second the account's daily task list (UI-3f1), its
//! reasons in the cards' words, and its packages' switches (UI-3f2).

use std::time::{Instant, SystemTime, UNIX_EPOCH};

use acui_rows::outcome::explain;
use acui_rows::{format_minute, format_second, local_utc_offset_minutes};
use acui_source::workbench::backends::{
    unnamed_backend_words, AppIndexProblem, BackendRefusal, BackendRow, BackendWhy, CaptureCause,
    CheckStatus, FailedStep, Holder, InEffect, InstanceBackends, OpenFailure, SwitchRefusal,
    SwitchVersion,
};
use acui_source::workbench::device::{DeviceDone, DeviceOp, Startup};
use acui_source::workbench::pack::{PackSource, PackState};
use acui_source::workbench::preview::Overlay;
use acui_source::workbench::rules::{IdleChoice, IdleNow, QuietOf, QuietRead, QuietShown};
use acui_source::workbench::stop::{Origin as RunOrigin, Refusal, Reset};
use acui_source::workbench::{
    PresetMember, PresetSetting, RefusalWhy, ReportedName, SettingName, SettingSource,
    SettingValue, TasksRead, ValueProblem, CONSOLE_PAUSE_ACCOUNT, CONSOLE_PAUSE_ALL, MODE_FULL,
    MODE_OBSERVE, MODE_PACE,
};
use slint::{Image, Model as _, ModelRc, Rgba8Pixel, SharedPixelBuffer, SharedString, VecModel};

use super::actions::{
    AccountEntry, AccountRules, AccountSaid, AccountWrite, ActionsView, Choice, EvidenceView,
    Field, FormProblem, GroupState, HostQuiet, PresetState, PresetView, QuietFormView, QuietRule,
    QuietTarget, Reread, ResumeEntry, RowEdit, RuleItem, RulesHold, RulesView, Said, StartRule,
    StepperView, Write, PRESET_ADAPTIVE, PRESET_LIGHT, PRESET_STANDARD,
};
use super::card::{reason_logged, Band, CardView, Down, Line, ReasonClass};
use super::model::{Fault, Page, RightTab};
use super::picture::{
    DimWhy, Doing, EmptyWhy, Offline, Origin, OverlayView, PictureView, RefreshHold, RefreshWhyNot,
    Refreshed, Unread,
};
use super::preview;
use super::rescue::{Button, ButtonWhy, RescueHold, RescueRows, RescueView, Rescued, Watch};
use super::settings::{
    self, AutostartView, BackendGroup, CheckEntry, ReportedAccount, ReportedLine, SelfCheckView,
    SettingLine, SettingsView, StartView,
};
use super::stop::{CloseRefusal, CloseWhy, StopLine, StopSaid, TaskClose};
use super::switch::SwitchSaid;
use super::tasks::{
    GroupTitle, PackRow, PackSwitch, SwitchSlot, SwitchesNote, TaskLine, TaskList, TaskName,
    TaskSaid, TasksView, Unreadable,
};
use crate::ledger::paint::models;
use crate::ledger::resume::Lifter;
use crate::report;
use crate::strings::{fill, outcome_short, Labels};
use crate::ui::Ui;
use crate::{
    AccountCard, BackendGroupItem, IdlePick, OverlayItem, PresetChoice, QuietData, QuietEditor,
    QuietLine, RescueBar, RescueOp, RulesData, SettingItem, StatusTone, TargetItem, TaskListData,
    TaskRowData, TaskTone, WorkbenchPage, WorkbenchTab, WorkbenchWindow,
};

/// What the technical log's lines about the settings page are under.
const SETTINGS_LOG: &str = "acui-settings";
/// And those about the live view, and its refresh.
const PICTURE_LOG: &str = preview::THREAD;
const REFRESH_LOG: &str = super::refresh::THREAD;
/// And those about the rescue bar's words (each operation's own are under its thread's name).
const RESCUE_LOG: &str = "acui-rescue";
/// And those about the daily task list's switches' words (each press's own are under its thread's
/// name, `switch`).
const TASKS_LOG: &str = "acui-tasks";

/// The codes a device's operation is refused or fails with that the rescue bar has words of its
/// own for, before the code table's (`rescue_code_words`); only looked up, never shown. A request
/// that waited its turn in vain names what held the instance (`rescue_busy`).
const LEASE_QUEUE_EXPIRED: &str = "lease_queue_expired";
const EMULATOR_CONTROL_BUSY: &str = "emulator_control_busy";
const EMULATOR_CONTROL_WAIT_TIMEOUT: &str = "emulator_control_wait_timeout";
const MUMU_MANAGER_WAIT_TIMEOUT: &str = "mumu_manager_wait_timeout";
const EMULATOR_CONTROL_ADB_NOT_READY: &str = "emulator_control_adb_not_ready";
const EMULATOR_RESTART_STOP_FAILED: &str = "emulator_restart_stop_failed";
const EMULATOR_RESTART_START_FAILED: &str = "emulator_restart_start_failed";
const EMULATOR_RESTART_PREVIOUS_RUNNING: &str = "emulator_restart_previous_instance_running";
/// Of an instance whose capability profile offers no such control, which the bar draws no
/// button for: the log hears of one, once.
const EMULATOR_CONTROL_UNAVAILABLE: &str = "emulator_control_unavailable";
const EMULATOR_CONTROL_UNSUPPORTED: &str = "emulator_control_unsupported";
const INSTANCE_NOT_RUNNING: &str = "instance_not_running";
const APPLICATION_BACKEND_FAILED: &str = "application_backend_operation_failed";
const APPLICATION_NOT_ASSIGNED: &str = "application_effect_requires_assigned_application";
/// A self-check's own failures, in the settings page's words for the same step
/// (`backend_cause_*`): the code table has only their category's.
const INPUT_BACKEND_OPEN_FAILED: &str = "input_backend_open_failed";
const CAPTURE_BACKEND_OPEN_FAILED: &str = "capture_backend_open_failed";
const INPUT_CHECK_FAILED: &str = "input_check_failed";
const CAPTURE_CHECK_FAILED: &str = "capture_check_failed";
const PAIRED_BACKEND_OPEN_FAILED: &str = "paired_backend_open_failed";

/// Which page shows.
pub fn page(ui: &Ui) {
    let Some(window) = ui.workbench() else {
        return;
    };
    let page = match ui.model.borrow().page() {
        Page::Main => WorkbenchPage::Main,
        Page::Settings => WorkbenchPage::Settings,
    };
    window.set_page(page);
}

/// The page a press on the top bar asks for.
pub fn page_picked(page: WorkbenchPage) -> Page {
    match page {
        WorkbenchPage::Main => Page::Main,
        WorkbenchPage::Settings => Page::Settings,
    }
}

/// The right column's tab a press asks for (UI-3f1).
pub fn tab_picked(tab: WorkbenchTab) -> RightTab {
    match tab {
        WorkbenchTab::Picture => RightTab::Picture,
        WorkbenchTab::Tasks => RightTab::Tasks,
    }
}

/// The operation a button of the rescue bar names.
pub fn rescue_picked(op: RescueOp) -> DeviceOp {
    match op {
        RescueOp::Check => DeviceOp::SelfCheck,
        RescueOp::EmulatorStart => DeviceOp::EmulatorStart,
        RescueOp::EmulatorStop => DeviceOp::EmulatorStop,
        RescueOp::EmulatorRestart => DeviceOp::EmulatorRestart,
        RescueOp::GameLaunch => DeviceOp::GameLaunch,
        RescueOp::GameRestart => DeviceOp::GameRestart,
    }
}

/// The idle action's choice a segment of the rules panel names.
pub fn idle_picked(pick: IdlePick) -> IdleChoice {
    match pick {
        IdlePick::DoNothing => IdleChoice::Nothing,
        IdlePick::CloseGame => IdleChoice::CloseGame,
    }
}

/// The editor of quiet times a press names (UI-3e1-c): an account's own on its rules panel, or
/// every account's on the settings page.
pub fn quiet_editor(editor: QuietEditor) -> QuietTarget {
    match editor {
        QuietEditor::Account => QuietTarget::Account,
        QuietEditor::Host => QuietTarget::Host,
    }
}

/// What the newest reads say of every account.
pub fn cards(ui: &Ui) {
    let Some(window) = ui.workbench() else {
        return;
    };
    let labels = ui.labels();
    let views = ui.model.borrow().cards(Instant::now());
    let cards = views.iter().map(|view| account_card(ui, labels, view)).collect();
    sync(window.get_account_cards(), cards, |rows| window.set_account_cards(rows));
}

/// The right column's live view: whose picture it is, beside the title; the frame, with when
/// it was captured on its corner, why it is dimmed across its top, and what took it under it;
/// or what it says while no frame shows, red when that is a failure. The frame's pixels are
/// the window's from when it landed (`picture_pixels`), let go here once no frame shows. On
/// a task's frame, unless turned off, the boxes its recognition evaluated and the points its
/// inputs were meant at, which the box beside the title turns off and on; under the frame in
/// amber why nothing is drawn on it, and that its proportions are not among those its game's
/// standard pack declares. Under all that the picture's refresh, which goes by what changes the
/// picture (`refresh`), and over the frame the rescue bar, which `refresh` paints with it.
pub fn picture(ui: &Ui) {
    refresh(ui);
    let Some(window) = ui.workbench() else {
        return;
    };
    let labels = ui.labels();
    let (account, view, overlays_on) = {
        let model = ui.model.borrow();
        let account = model.picture_account().map(str::to_owned).unwrap_or_default();
        (account, model.picture_view(now_ms()), model.pictures().overlays_on())
    };
    window.set_picture_account(account.into());
    window.set_picture_withdrawn(matches!(view, PictureView::Withdrawn));
    if window.get_picture_overlays_on() != overlays_on {
        window.set_picture_overlays_on(overlays_on);
    }
    let (empty, empty_failed) = match &view {
        PictureView::NoAccount => (labels.picture_no_account.to_owned(), false),
        // UI-3d says why it was taken down.
        PictureView::Withdrawn | PictureView::Frame { .. } => (String::new(), false),
        PictureView::Empty { why } => empty_text(labels, why),
    };
    window.set_picture_empty(empty.into());
    window.set_picture_empty_failed(empty_failed);
    let PictureView::Frame { captured_at_ms, age, origin, dim, overlay, aspect } = &view else {
        if window.get_picture_ready() {
            window.set_picture_ready(false);
            window.set_picture_image(Image::default());
        }
        window.set_picture_corner(SharedString::new());
        window.set_picture_corner_warn(false);
        window.set_picture_dimmed(false);
        window.set_picture_banner(SharedString::new());
        window.set_picture_origin(SharedString::new());
        picture_overlay(&window, None);
        window.set_picture_overlays_offered(false);
        window.set_picture_hint(SharedString::new());
        return;
    };
    let at = format_second(*captured_at_ms);
    let (corner, warn) = corner_text(labels, &at, *age);
    let banner = dim.as_ref().map_or_else(String::new, |dim| dim_text(ui, labels, dim, &at));
    let origin_text = match origin {
        Origin::Task => labels.picture_from_task,
        Origin::Observation => labels.picture_from_observation,
        Origin::OwnRefresh => labels.picture_from_refresh,
    };
    window.set_picture_corner(corner.into());
    window.set_picture_corner_warn(warn);
    window.set_picture_dimmed(dim.is_some());
    window.set_picture_banner(banner.into());
    window.set_picture_origin(origin_text.into());
    let drawn = match overlay {
        OverlayView::Shown(overlay) => Some(overlay),
        OverlayView::Off | OverlayView::NotOnThisFrame | OverlayView::Mismatch => None,
    };
    picture_overlay(&window, drawn);
    // Nothing is recognized on an observation's frame: the box has nothing to turn on.
    window.set_picture_overlays_offered(*origin == Origin::Task);
    let mut hint = Vec::new();
    match overlay {
        // A task's frame whose recognition is not recorded yet says so. An observation's
        // never has any: its greyed box and the source line say that already, and an amber
        // line under every observed frame would read as something wrong.
        OverlayView::NotOnThisFrame if *origin == Origin::Task => {
            hint.push(labels.picture_marks_none.to_owned());
        }
        OverlayView::NotOnThisFrame => {}
        OverlayView::Mismatch => hint.push(labels.picture_marks_mismatch.to_owned()),
        OverlayView::Off | OverlayView::Shown(_) => {}
    }
    if let Some((width, height)) = aspect {
        hint.push(fill(labels.picture_aspect_undeclared, &[&format!("{width}:{height}")]));
    }
    window.set_picture_hint(hint.join("\n").into());
    window.set_picture_ready(true);
}

/// What is drawn on the frame: its targets' boxes, without words, and its inputs' points as
/// tap marks; nothing with no `overlay`. The rows change only where they differ.
fn picture_overlay(window: &WorkbenchWindow, overlay: Option<&Overlay>) {
    let (targets, points): (Vec<TargetItem>, Vec<OverlayItem>) = match overlay {
        Some(overlay) => (
            overlay
                .boxes
                .iter()
                .map(|target| TargetItem {
                    label: SharedString::new(),
                    x: target.x,
                    y: target.y,
                    width: target.width,
                    height: target.height,
                    passed: target.passed,
                })
                .collect(),
            overlay
                .points
                .iter()
                .map(|&(x, y)| OverlayItem {
                    kind: SharedString::new(),
                    x,
                    y,
                    width: 0.0,
                    height: 0.0,
                    point: true,
                    mark: true,
                })
                .collect(),
        ),
        None => (Vec::new(), Vec::new()),
    };
    window.set_picture_overlay_shown(overlay.is_some());
    sync(window.get_picture_targets(), targets, |rows| window.set_picture_targets(rows));
    sync(window.get_picture_points(), points, |rows| window.set_picture_points(rows));
}

/// A frame that landed, into the window: its pixels, decoded off the event loop, and its
/// size, which the frame keeps its proportions by.
pub fn picture_pixels(ui: &Ui, pixels: SharedPixelBuffer<Rgba8Pixel>) {
    let Some(window) = ui.workbench() else {
        return;
    };
    let (width, height) = (pixels.width(), pixels.height());
    window.set_picture_image(Image::from_rgba8(pixels));
    window.set_picture_width(width as f32);
    window.set_picture_height(height as f32);
}

/// Every second the main page shows: the frame's corner says how old it is once it is an old
/// picture. Set only when its words change.
pub fn picture_age(ui: &Ui) {
    let Some(window) = ui.workbench() else {
        return;
    };
    let view = {
        let model = ui.model.borrow();
        if model.page() != Page::Main {
            return;
        }
        model.picture_view(now_ms())
    };
    let PictureView::Frame { captured_at_ms, age, .. } = view else {
        return;
    };
    let (corner, warn) = corner_text(ui.labels(), &format_second(captured_at_ms), age);
    if window.get_picture_corner() != corner.as_str() {
        window.set_picture_corner(corner.into());
    }
    if window.get_picture_corner_warn() != warn {
        window.set_picture_corner_warn(warn);
    }
}

/// Now, in the milliseconds a frame's capture is stated in.
pub(super) fn now_ms() -> u64 {
    let since = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default();
    u64::try_from(since.as_millis()).unwrap_or(u64::MAX)
}

/// The frame's corner: when it was captured, or, once it is an old picture, how long ago, in
/// amber. Never a count of anything.
fn corner_text(labels: &Labels, at: &str, age: Option<u64>) -> (String, bool) {
    let Some(seconds) = age else {
        return (fill(labels.picture_captured, &[at]), false);
    };
    let minutes = seconds / 60;
    let text = match minutes {
        0 => labels.picture_old_half.to_owned(),
        1..=59 => fill(labels.picture_old_minutes, &[&minutes.to_string()]),
        60..=1439 => fill(labels.picture_old_hours, &[&(minutes / 60).to_string()]),
        _ => fill(labels.picture_old_days, &[&(minutes / 1440).to_string()]),
    };
    (text, true)
}

/// Why no frame shows, and whether that is a failure.
fn empty_text(labels: &Labels, why: &EmptyWhy) -> (String, bool) {
    match why {
        EmptyWhy::Looking => (labels.picture_looking.to_owned(), false),
        EmptyWhy::NoneRecent => (labels.picture_none_recent.to_owned(), false),
        EmptyWhy::Unreadable(why) => (unread_text(labels, *why).to_owned(), why.is_problem()),
        EmptyWhy::Offline(Offline::NotRunning) => {
            (labels.picture_not_running_empty.to_owned(), false)
        }
        EmptyWhy::Offline(Offline::Unreachable) => {
            (labels.picture_unreachable_empty.to_owned(), false)
        }
        EmptyWhy::Interrupted => (labels.picture_interrupted_empty.to_owned(), false),
        EmptyWhy::Freeze => (labels.picture_handing_over_empty.to_owned(), false),
        EmptyWhy::Fault(fault) => (fault_text(labels, fault), true),
    }
}

/// Why the frame shown is dimmed, with when it was captured, `at`.
fn dim_text(ui: &Ui, labels: &Labels, dim: &DimWhy, at: &str) -> String {
    match dim {
        DimWhy::Offline(Offline::NotRunning) => fill(labels.picture_not_running, &[at]),
        DimWhy::Offline(Offline::Unreachable) => fill(labels.picture_unreachable, &[at]),
        DimWhy::Freeze => fill(labels.picture_handing_over, &[at]),
        DimWhy::Fault(fault) => fault_text(labels, fault),
        DimWhy::Interrupted => fill(labels.picture_interrupted, &[at]),
        DimWhy::Restarted => labels.picture_restarted.to_owned(),
        DimWhy::CaptureFailed(code) => {
            let words = capture_words(ui, labels, code.as_deref());
            fill(labels.picture_capture_failed, &[at, &words])
        }
        DimWhy::NewerUnreadable(why) => {
            fill(labels.picture_newer_unreadable, &[unread_text(labels, *why), at])
        }
    }
}

fn fault_text(labels: &Labels, fault: &Fault) -> String {
    match fault {
        Fault::Panicked(_) => labels.picture_panicked.to_owned(),
        Fault::NoThread(error) => fill(labels.picture_no_thread, &[error]),
    }
}

fn unread_text(labels: &Labels, why: Unread) -> &'static str {
    match why {
        Unread::Missing => labels.picture_gone,
        Unread::Evicted => labels.picture_evicted,
        Unread::TimedOut => labels.picture_timed_out,
        Unread::Integrity => labels.picture_integrity,
        Unread::Busy => labels.picture_busy,
        Unread::TooLarge => labels.picture_too_large,
        Unread::Undecodable => labels.picture_undecodable,
        Unread::Failed => labels.picture_read_failed,
    }
}

/// Why a capture failed in plain words, by its outcome code. The picture never shows the
/// code: one the tables do not know, or none, gets a general sentence, and an unknown one goes
/// to the log once.
fn capture_words(ui: &Ui, labels: &Labels, code: Option<&str>) -> String {
    let Some(code) = code else {
        return labels.picture_cause_unknown.to_owned();
    };
    match explain(code, labels.tongue.outcome_tongue()) {
        Ok(Some(words)) => words.short.to_owned(),
        Ok(None) => {
            let line = format!("no words for capture failure {code}");
            if ui.model.borrow_mut().first_told(&line) {
                report::log_quiet(PICTURE_LOG, &line);
            }
            labels.picture_cause_unknown.to_owned()
        }
        Err(error) => fill(labels.outcome_table_unusable, &[&error.to_string()]),
    }
}

/// Under the live view, while an account's picture shows: the button that refreshes it
/// (UI-3c-2), pressed only when it may be; beside it why it may not, and while it cools down
/// that it was just refreshed, never for how long; under it, on a line of its own so that
/// neither hides the other, what the last refresh of that account came to, red when it failed.
/// What a refresh costs is the window's own, in grey. First the rescue bar (`rescue`): whatever
/// holds one back may hold the other, and so a refresh pressed, landed or cooled down, an
/// operation of the device as well, paints both.
pub fn refresh(ui: &Ui) {
    rescue(ui);
    let Some(window) = ui.workbench() else {
        return;
    };
    let labels = ui.labels();
    let view = ui.model.borrow().refresh_view();
    let Some(view) = view else {
        window.set_refresh_shown(false);
        window.set_refresh_offered(false);
        window.set_refresh_reason(SharedString::new());
        window.set_refresh_line(SharedString::new());
        window.set_refresh_failed(false);
        return;
    };
    let reason = view.why_not.map_or_else(String::new, |why| refresh_why_not(labels, why));
    let (line, failed) = match &view.said {
        Some(said) => refreshed_text(ui, labels, said),
        None => (String::new(), false),
    };
    window.set_refresh_shown(true);
    window.set_refresh_offered(view.offered);
    window.set_refresh_reason(reason.into());
    window.set_refresh_line(line.into());
    window.set_refresh_failed(failed);
}

/// Why the refresh's button cannot be pressed now; nothing while the page does not show.
fn refresh_why_not(labels: &Labels, why: RefreshWhyNot) -> String {
    let hold = match why {
        RefreshWhyNot::CoolingDown => return labels.refresh_cooling.to_owned(),
        RefreshWhyNot::Hold(hold) => hold,
    };
    match hold {
        RefreshHold::Hidden => String::new(),
        RefreshHold::NotOnline => labels.refresh_not_online.to_owned(),
        RefreshHold::GateShut => labels.refresh_gate_shut.to_owned(),
        RefreshHold::Running => labels.refresh_running.to_owned(),
        RefreshHold::Busy(doing) => fill(labels.refresh_busy, &[doing_text(labels, doing)]),
        RefreshHold::EmulatorOff { at_ms } => {
            fill(labels.refresh_emulator_off, &[&format_second(at_ms)])
        }
        RefreshHold::Fault => labels.refresh_fault.to_owned(),
    }
}

/// The same words the card has for it; for this console's own work with the device, which no
/// card says, words of its own.
fn doing_text(labels: &Labels, doing: Doing) -> &'static str {
    match doing {
        Doing::Reconnecting => labels.card_reconnecting,
        Doing::SelfChecking => labels.card_self_checking,
        Doing::ControllingEmulator => labels.card_controlling_emulator,
        Doing::Climbing => labels.card_climbing,
        Doing::OperatingDevice => labels.refresh_operating_device,
    }
}

/// What a refresh came to in words, and whether that is a failure.
fn refreshed_text(ui: &Ui, labels: &Labels, said: &Refreshed) -> (String, bool) {
    match said {
        Refreshed::Sending => (labels.refresh_sending.to_owned(), false),
        Refreshed::Observed { at_ms } => {
            (fill(labels.refresh_observed, &[&format_second(*at_ms)]), false)
        }
        Refreshed::Unread => (labels.refresh_unread.to_owned(), false),
        Refreshed::Refused { host, class } => {
            let words = refusal_words(ui, labels, host.as_deref(), class.as_deref());
            (fill(labels.refresh_refused, &[&words]), true)
        }
        Refreshed::Unconfirmed => (labels.refresh_unconfirmed.to_owned(), true),
        Refreshed::NotSent => (labels.refresh_not_sent.to_owned(), true),
        Refreshed::NoThread { error } => {
            let why = fill(labels.thread_failed, &[error.as_str()]);
            (fill(labels.refresh_no_thread, &[why.as_str()]), true)
        }
        Refreshed::Restarted => (labels.refresh_restarted.to_owned(), false),
    }
}

/// Why the Runtime captured nothing, in plain words: by the host's own code, which says why,
/// else by the Runtime's, whose category speaks for a code the tables give no words of its
/// own. The refresh never shows a code: with words for neither it says so, and the log hears
/// of them once (the refusal itself is in the log already, codes and all).
fn refusal_words(ui: &Ui, labels: &Labels, host: Option<&str>, class: Option<&str>) -> String {
    for code in [host, class].into_iter().flatten() {
        match explain(code, labels.tongue.outcome_tongue()) {
            Ok(Some(words)) => return words.short.to_owned(),
            Ok(None) => {}
            Err(error) => return fill(labels.outcome_table_unusable, &[&error.to_string()]),
        }
    }
    let line = format!("no words for a refresh refused with host code {host:?}, code {class:?}");
    if ui.model.borrow_mut().first_told(&line) {
        report::log_quiet(REFRESH_LOG, &line);
    }
    labels.refresh_cause_unknown.to_owned()
}

/// Under the live view's title, while an account's picture shows on the main page: its rescue
/// bar (UI-3e1-a), from its view. While no Runtime answers, only that no device can be operated,
/// and over it, once no `Status` is read any more, what a stop out or awaited then came to.
/// Else, of an account that drives a device of its own, its reconnect and check, its emulator's
/// row while the Runtime offers its control, and its game's: each button pressed only when it may
/// be, and at the row's end why one may not, or what the check does. While the account does
/// something, "stop this run" (UI-3e2), or "close this task" of a run the scheduler dispatched
/// (UI-3e2b), pressed only when it may be, and beside it what a press does or why it may not;
/// under it, without it too, what the account's last stop came to, of a close what became of the
/// task first, red when it failed. Then why every device button is off, or the question a press
/// that closes the game asks; and what the account's last operation came to, red when it failed.
/// Set only when anything in it changed.
///
/// While the bar has rows, a fixture's account's as well, "auto rules" at the end of its first
/// row; while that is open, the account's rules panel in the picture's place (`rules_panel`),
/// painted with the bar, since any change of settings out or landed holds or frees both.
pub fn rescue(ui: &Ui) {
    let Some(window) = ui.workbench() else {
        return;
    };
    let labels = ui.labels();
    let (alias, view, rules) = {
        let model = ui.model.borrow();
        let alias = model.picture_account().map(str::to_owned);
        let view = alias.as_deref().and_then(|alias| model.rescue_view(alias));
        let rules = alias
            .as_deref()
            .filter(|_| model.rules_open())
            .and_then(|alias| model.rules_view(alias));
        (alias, view, rules)
    };
    let mut bar = match (alias.as_deref(), view) {
        (Some(alias), Some(RescueView::Online(rows))) => {
            rescue_told(ui, alias, &rows);
            rescue_bar(ui, labels, alias, &rows)
        }
        (_, Some(RescueView::Offline { stop })) => {
            let (close_line, close_failed) = match &stop {
                Some(said) => stop_text(ui, labels, said),
                None => (String::new(), false),
            };
            RescueBar {
                shown: true,
                reason: labels.rescue_offline.into(),
                close_line: close_line.into(),
                close_failed,
                ..RescueBar::default()
            }
        }
        (_, None) | (None, Some(RescueView::Online(_))) => RescueBar::default(),
    };
    // The panel shows only under a bar with rows: none while no Runtime answers.
    let rules = rules.filter(|_| bar.rules_shown);
    bar.rules_open = rules.is_some();
    rules_panel(ui, &window, labels, alias.as_deref().unwrap_or(""), rules.as_ref());
    if window.get_rescue() != bar {
        window.set_rescue(bar);
    }
}

/// The bar's words for the rows of `alias`'s view. Why every device button is off stands only
/// over device buttons: a fixture's account has none, only "stop this run" or "close this task"
/// with its own words, which that reason does not hold back.
fn rescue_bar(ui: &Ui, labels: &Labels, alias: &str, rows: &RescueRows) -> RescueBar {
    let offered = |button: Option<Button>| button.is_some_and(|button| button.offered);
    let devices = rows.check.is_some() || rows.emulator.is_some() || rows.game.is_some();
    let reason = match rows.hold {
        Some(hold) if devices => hold_text(labels, hold),
        Some(_) | None => String::new(),
    };
    let (check_words, check_urgent) = check_note(labels, rows);
    // HS-5c: the fence a switch lifts, while the check is the thing to do first.
    let check_hint =
        if check_urgent && rows.file_value_lifts { labels.rescue_check_or_file_value } else { "" };
    let emulator = rows.emulator;
    let (confirm, confirm_label) = confirm_text(labels, alias, rows);
    let (line, failed) = match &rows.said {
        Some(said) => rescued_text(ui, labels, rows, said),
        None => (String::new(), false),
    };
    let (close_line, close_failed) = match &rows.stop {
        Some(said) => stop_text(ui, labels, said),
        None => (String::new(), false),
    };
    RescueBar {
        shown: true,
        reason: reason.into(),
        check_shown: rows.check.is_some(),
        check_offered: offered(rows.check),
        check_note: check_words.into(),
        check_urgent,
        check_hint: check_hint.into(),
        emulator_shown: emulator.is_some(),
        start_shown: emulator.is_some_and(|row| row.start.is_some()),
        start_offered: offered(emulator.and_then(|row| row.start)),
        stop_shown: emulator.is_some_and(|row| row.stop.is_some()),
        stop_offered: offered(emulator.and_then(|row| row.stop)),
        restart_shown: emulator.is_some_and(|row| row.restart.is_some()),
        restart_offered: offered(emulator.and_then(|row| row.restart)),
        emulator_note: emulator_note(labels, rows).into(),
        game_shown: rows.game.is_some(),
        launch_offered: offered(rows.game.map(|row| row.launch)),
        game_restart_offered: offered(rows.game.map(|row| row.restart)),
        game_note: game_note(labels, rows).into(),
        close_shown: rows.close.is_some(),
        close_task: rows.close.is_some_and(|row| row.task),
        close_offered: rows.close.is_some_and(|row| row.offered),
        close_reason: rows.close.map_or("", |row| close_text(labels, row.why)).into(),
        close_line: close_line.into(),
        close_failed,
        confirm: confirm.into(),
        confirm_label: confirm_label.into(),
        line: line.into(),
        failed,
        // Every account the newest `Status` lists has rules; whether their panel is open is the
        // bar's caller's to say.
        rules_shown: true,
        rules_open: false,
    }
}

/// Why every device button of the account is off. An operation out says so on the line of what
/// it came to; the account's doing is in the card's words.
fn hold_text(labels: &Labels, hold: RescueHold) -> String {
    match hold {
        RescueHold::GateShut => labels.rescue_handing_over.to_owned(),
        RescueHold::Out(_) => String::new(),
        RescueHold::Refreshing => labels.rescue_refreshing.to_owned(),
        RescueHold::Switching => labels.rescue_switching.to_owned(),
        RescueHold::CoolingDown => labels.rescue_cooling.to_owned(),
        RescueHold::Running => labels.rescue_running.to_owned(),
        RescueHold::Recovering => labels.rescue_recovering.to_owned(),
        RescueHold::Busy(doing) => fill(labels.rescue_busy_doing, &[doing_text(labels, doing)]),
    }
}

/// Beside the check, and whether in amber: why it cannot be pressed, by its own reading, with
/// when that was seen; that it is the thing to do first, the device being fenced; or what it
/// does. Nothing while every button is off: the reason line says why.
fn check_note(labels: &Labels, rows: &RescueRows) -> (String, bool) {
    let Some(button) = rows.check else {
        return (String::new(), false);
    };
    match button.why_not {
        Some(ButtonWhy::EmulatorOff { at_ms }) => {
            (fill(labels.rescue_emulator_off_first, &[&format_minute(at_ms)]), false)
        }
        // The check has no other reason of its own.
        Some(ButtonWhy::EmulatorOn { .. } | ButtonWhy::Fault) => (String::new(), false),
        None if !button.offered => (String::new(), false),
        None if rows.check_urgent => (labels.rescue_check_first.to_owned(), true),
        None => (labels.rescue_check_note.to_owned(), false),
    }
}

/// Beside the emulator's buttons: why one cannot be pressed, as the emulator was last read, and
/// when; off, that a start is the way, while one can be pressed.
fn emulator_note(labels: &Labels, rows: &RescueRows) -> String {
    let Some(row) = rows.emulator else {
        return String::new();
    };
    let why = [row.start, row.stop, row.restart]
        .into_iter()
        .flatten()
        .find_map(|button| button.why_not);
    match why {
        Some(ButtonWhy::EmulatorOn { at_ms }) => {
            fill(labels.rescue_seen_on, &[&format_minute(at_ms)])
        }
        Some(ButtonWhy::EmulatorOff { at_ms }) => {
            let start = row.start.is_some_and(|button| button.offered);
            let template = if start { labels.rescue_seen_off_start } else { labels.rescue_seen_off };
            fill(template, &[&format_minute(at_ms)])
        }
        Some(ButtonWhy::Fault) | None => String::new(),
    }
}

/// Beside the game's buttons, which share their reason: the emulator seen off, or a fault to deal
/// with first.
fn game_note(labels: &Labels, rows: &RescueRows) -> String {
    match rows.game.and_then(|row| row.launch.why_not) {
        Some(ButtonWhy::EmulatorOff { at_ms }) => {
            fill(labels.rescue_emulator_off_first, &[&format_minute(at_ms)])
        }
        Some(ButtonWhy::Fault) => labels.rescue_game_fault.to_owned(),
        Some(ButtonWhy::EmulatorOn { .. }) | None => String::new(),
    }
}

/// Beside "stop this run" or "close this task": what a press does, or why it cannot be pressed;
/// nothing while a stop or a close is out or waits for the run, which the line under it says.
/// Never the task's name, nor the run's.
fn close_text(labels: &Labels, why: CloseWhy) -> &'static str {
    match why {
        CloseWhy::Stopping => "",
        CloseWhy::CoolingDown => labels.rescue_cooling,
        CloseWhy::SwitchWriting => labels.close_task_busy,
        CloseWhy::Unsupported => labels.stop_unsupported,
        CloseWhy::CloseUnsupported => labels.close_task_unsupported,
        CloseWhy::CloseTask => labels.close_task_note,
        CloseWhy::CloseUnidentified => labels.close_task_unidentified,
        CloseWhy::DirectRun => labels.stop_note,
        CloseWhy::Unidentified => labels.stop_unidentified,
        CloseWhy::RuntimeOwn => labels.rescue_close_system,
        CloseWhy::NoTask => labels.rescue_close_no_task,
    }
}

/// What the account's last stop of a run came to in words, each that landed with when, and
/// whether that is a failure: never a code, a count or an id, which the log has. Of "close this
/// task": while it is out, that it closes the task and then stops the run; once its close row was
/// written, that the task is closed and no longer scheduled, with when, and then what the stop
/// came to, which says how to open the task again once the run stopped (`closed_stop_text`); of a
/// package the account's daily tasks do not run, before what the stop came to, that only the run
/// was stopped when it stopped, and else that nothing was closed here; of a close not written,
/// why, and that the run was not stopped either; of a close whose thread panicked, that neither is
/// known. After a reset that failed, or a run that ended before the stop's reset, the card says
/// what holds now.
fn stop_text(ui: &Ui, labels: &Labels, line: &StopLine) -> (String, bool) {
    match line.task {
        Some(TaskClose::Closed { at_ms }) => {
            let head = fill(labels.close_task_done, &[&format_second(at_ms)]);
            let (rest, failed) = closed_stop_text(ui, labels, &line.said);
            (fill(labels.rescue_then, &[&head, &rest]), failed)
        }
        // Only a stop that settled as stopped says that only this run was stopped; any other says
        // that nothing was closed here, and then what the stop came to.
        Some(TaskClose::NotListed) => {
            let stopped = matches!(
                line.said,
                StopSaid::Stopped { reset: Reset::Done | Reset::Failed { .. }, .. }
            );
            let head = if stopped {
                labels.close_task_not_listed
            } else {
                labels.close_task_not_listed_unclosed
            };
            let (rest, failed) = stop_said_text(ui, labels, &line.said);
            (fill(labels.rescue_then, &[head, &rest]), failed)
        }
        Some(TaskClose::Asked) => match &line.said {
            StopSaid::Sending => (labels.close_task_sending.to_owned(), false),
            StopSaid::Restarted { at_ms } => (close_restarted(labels, *at_ms), false),
            said => stop_said_text(ui, labels, said),
        },
        None => stop_said_text(ui, labels, &line.said),
    }
}

/// What a stop came to after its task's close row was written, in words: the rest of its line,
/// after the close's. The run stopped and reset says how to open the task again; what a stop alone
/// says the same way (a run that ended for another reason first, the follow-up out or not settled,
/// the follow-up's refusals, a request not sent, the account unlisted, no status read) is said so.
fn closed_stop_text(ui: &Ui, labels: &Labels, said: &StopSaid) -> (String, bool) {
    match said {
        StopSaid::Stopped { at_ms, reset: Reset::Done } => {
            (fill(labels.close_task_stop_done, &[&format_second(*at_ms)]), false)
        }
        StopSaid::Stopped { at_ms, reset: Reset::Failed { cause, .. } } => {
            let codes: Vec<&str> = cause.as_deref().into_iter().collect();
            let words = stop_words(ui, labels, &codes);
            (fill(labels.close_task_stop_reset_failed, &[&format_second(*at_ms), &words]), true)
        }
        StopSaid::Asked { at_ms } => {
            (fill(labels.close_task_stop_asked, &[&format_second(*at_ms)]), false)
        }
        StopSaid::Refused { at_ms, why, origin, host, class, follow_up, .. } => {
            let at = format_second(*at_ms);
            match (why, origin) {
                // The run ran out on its own between the `Status` and the press: no failure.
                (Refusal::RunUnknown, _) if !*follow_up => {
                    (labels.close_task_stop_run_ended.to_owned(), false)
                }
                // A Runtime with task switches and without R-6b, which no release is.
                (Refusal::NotStoppable, Some(RunOrigin::Policy)) => {
                    (fill(labels.close_task_stop_policy, &[&at]), true)
                }
                // The Runtime's own step took the device between the `Status` and the press.
                (Refusal::NotStoppable, Some(RunOrigin::Host)) => {
                    (fill(labels.close_task_stop_host, &[&at]), true)
                }
                (Refusal::RunUnknown, _) => stop_said_text(ui, labels, said),
                (Refusal::NotStoppable, None) | (Refusal::NotImplemented | Refusal::Other, _) => {
                    let codes: Vec<&str> =
                        [host.as_deref(), class.as_deref()].into_iter().flatten().collect();
                    let words = stop_words(ui, labels, &codes);
                    (fill(labels.close_task_stop_refused, &[&words, &at]), true)
                }
            }
        }
        StopSaid::Unconfirmed { at_ms } => {
            (fill(labels.close_task_stop_unconfirmed, &[&format_second(*at_ms)]), true)
        }
        StopSaid::Restarted { at_ms } => (close_restarted(labels, *at_ms), false),
        StopSaid::Stopped { reset: Reset::NotNeeded, .. }
        | StopSaid::Sending
        | StopSaid::Reading
        | StopSaid::Unsettled { .. }
        | StopSaid::CloseRefused { .. }
        | StopSaid::CloseUnconfirmed { .. }
        | StopSaid::ClosePanicked { .. }
        | StopSaid::NotSent { .. }
        | StopSaid::NoThread { .. }
        | StopSaid::Unlisted { .. }
        | StopSaid::NoStatus { .. } => stop_said_text(ui, labels, said),
    }
}

/// A restart let "close this task" go, out or awaiting its stop's follow-up: what came of it is not
/// known, and the daily task list's switch says whether the task is closed.
fn close_restarted(labels: &Labels, at_ms: u64) -> String {
    let at = format_second(at_ms);
    let restarted = fill(labels.stop_at, &[labels.rescue_runtime_restarted, &at]);
    fill(labels.rescue_then, &[&restarted, labels.close_task_restarted])
}

/// What a stop came to in words, as a stop alone says it (`stop_text`); and a close not written.
fn stop_said_text(ui: &Ui, labels: &Labels, said: &StopSaid) -> (String, bool) {
    let timed = |text: &str, at_ms: u64| fill(labels.stop_at, &[text, &format_second(at_ms)]);
    match said {
        StopSaid::Sending => (labels.stop_sending.to_owned(), false),
        StopSaid::Stopped { at_ms, reset } => {
            let at = format_second(*at_ms);
            match reset {
                Reset::Done => (fill(labels.stop_done, &[&at]), false),
                Reset::NotNeeded => (fill(labels.stop_not_needed, &[&at]), false),
                Reset::Failed { cause, .. } => {
                    let codes: Vec<&str> = cause.as_deref().into_iter().collect();
                    let words = stop_words(ui, labels, &codes);
                    (fill(labels.stop_reset_failed, &[&at, &words]), true)
                }
            }
        }
        StopSaid::Asked { at_ms } => {
            (fill(labels.stop_asked, &[&format_second(*at_ms)]), false)
        }
        StopSaid::Reading => (labels.stop_reading.to_owned(), false),
        StopSaid::Unsettled { at_ms } => {
            (fill(labels.stop_unsettled, &[&format_second(*at_ms)]), false)
        }
        StopSaid::Refused { at_ms, why, origin, host, class, denied, follow_up } => {
            let at = format_second(*at_ms);
            let nothing_done = fill(labels.stop_at, &[labels.rescue_nothing_done, &at]);
            match (why, origin) {
                (Refusal::NotStoppable, Some(RunOrigin::Host)) => {
                    (fill(labels.rescue_then, &[labels.rescue_close_system, &nothing_done]), true)
                }
                // The run ran out on its own between the `Status` and the press: no failure. Of
                // the follow-up, the reset's result is not known.
                (Refusal::RunUnknown, _) if !*follow_up => {
                    (fill(labels.stop_run_ended, &[&at]), false)
                }
                (Refusal::RunUnknown, _) => (fill(labels.stop_run_ended_unread, &[&at]), true),
                (Refusal::NotImplemented, _) => {
                    (fill(labels.stop_refused_unsupported, &[&at]), true)
                }
                // A run the scheduler dispatched is stopped after its close (`closed_stop_text`),
                // whose words say a refusal as the scheduler's; here it is any other refusal.
                (Refusal::NotStoppable, None | Some(RunOrigin::Policy)) | (Refusal::Other, _) => {
                    let codes: Vec<&str> =
                        [host.as_deref(), class.as_deref()].into_iter().flatten().collect();
                    let words = stop_words(ui, labels, &codes);
                    let sentence = refusal_sentence(labels, labels.stop_op, &words, *denied);
                    (fill(labels.stop_at, &[&sentence, &at]), true)
                }
            }
        }
        StopSaid::Unconfirmed { at_ms } => {
            (fill(labels.stop_unconfirmed, &[&format_second(*at_ms)]), true)
        }
        StopSaid::CloseRefused { at_ms, why } => (close_refused_text(ui, labels, why, *at_ms), true),
        StopSaid::CloseUnconfirmed { at_ms } => {
            (fill(labels.close_task_unconfirmed, &[&format_second(*at_ms)]), true)
        }
        StopSaid::ClosePanicked { at_ms } => {
            (fill(labels.close_task_panicked, &[&format_second(*at_ms)]), true)
        }
        StopSaid::NotSent { at_ms } => (timed(labels.rescue_not_sent, *at_ms), true),
        StopSaid::NoThread { at_ms, error } => {
            let why = fill(labels.thread_failed, &[error.as_str()]);
            let not_sent = fill(labels.rescue_no_thread, &[why.as_str()]);
            (timed(not_sent.as_str(), *at_ms), true)
        }
        StopSaid::Restarted { at_ms } => (timed(labels.rescue_runtime_restarted, *at_ms), false),
        StopSaid::Unlisted { at_ms } => (timed(labels.stop_unlisted, *at_ms), false),
        StopSaid::NoStatus { at_ms } => (timed(labels.stop_no_status, *at_ms), true),
    }
}

/// Why the close of "close this task" was not written, at `at_ms`, in words: that this Runtime
/// cannot close a task from here, and nothing was done; or why, and that the run was not stopped
/// either. Any other refusal's reason as a stop's (`stop_words`).
fn close_refused_text(ui: &Ui, labels: &Labels, why: &CloseRefusal, at_ms: u64) -> String {
    let at = format_second(at_ms);
    match why {
        CloseRefusal::Unsupported => fill(labels.close_task_refused_unsupported, &[&at]),
        CloseRefusal::NotNewer => fill(labels.close_task_not_newer, &[&at]),
        CloseRefusal::TooMany => fill(labels.close_task_too_many, &[&at]),
        CloseRefusal::StoredUnreadable => fill(labels.close_task_stored_unreadable, &[&at]),
        CloseRefusal::ViewDropped => fill(labels.close_task_view_dropped, &[&at]),
        CloseRefusal::Other { codes } => {
            let codes: Vec<&str> = codes.iter().map(String::as_str).collect();
            let words = stop_words(ui, labels, &codes);
            fill(labels.close_task_refused, &[&words, &at])
        }
    }
}

/// A stop's reason in plain words, never its code (`codes`, the host's before the Runtime's, or a
/// failed reset's cause; of a close not written, the client's own after them): as a device's
/// operation's (`code_words`); else that it is not known here, which the log hears of once, as the
/// problem it is (the stop's own line has its codes and its chain already).
fn stop_words(ui: &Ui, labels: &Labels, codes: &[&str]) -> String {
    if let Some(words) = code_words(labels, codes) {
        return words;
    }
    let line = format!(
        "no words for a stop of a run or a close of its task refused, or for its reset that \
         failed, with codes {codes:?}"
    );
    if ui.model.borrow_mut().first_told(&line) {
        report::log(RESCUE_LOG, &line);
    }
    labels.rescue_cause_unknown.to_owned()
}

/// The question a press that closes the game asks, naming the account, and its confirming
/// button's words. A stop says too when the emulator's watch starts the emulator again, by the
/// setting's own name, or that it may when that does not read.
fn confirm_text(labels: &Labels, alias: &str, rows: &RescueRows) -> (String, &'static str) {
    match rows.confirming {
        Some(DeviceOp::EmulatorStop) => {
            let setting = labels.setting_start_emulator;
            let text = match rows.watch {
                Watch::Restarts => fill(labels.rescue_confirm_stop_watched, &[alias, setting]),
                Watch::Unknown => fill(labels.rescue_confirm_stop_maybe_watched, &[alias, setting]),
                Watch::Stays => fill(labels.rescue_confirm_stop, &[alias]),
            };
            (text, labels.rescue_confirm_stop_button)
        }
        Some(DeviceOp::EmulatorRestart) => {
            (fill(labels.rescue_confirm_restart, &[alias]), labels.rescue_confirm_restart_button)
        }
        Some(DeviceOp::GameRestart) => {
            let text = fill(labels.rescue_confirm_game_restart, &[alias]);
            (text, labels.rescue_confirm_restart_button)
        }
        // These go out at the first press.
        Some(DeviceOp::SelfCheck | DeviceOp::EmulatorStart | DeviceOp::GameLaunch) | None => {
            (String::new(), "")
        }
    }
}

/// What the account's last operation came to in words, and whether that is a failure.
fn rescued_text(ui: &Ui, labels: &Labels, rows: &RescueRows, said: &Rescued) -> (String, bool) {
    match said {
        Rescued::Sending(op) => (sending_text(labels, *op).to_owned(), false),
        Rescued::Done { op, at_ms, done, .. } => {
            done_text(ui, labels, rows, *op, &format_second(*at_ms), done)
        }
        Rescued::Refused { op, host, class, holder, denied } => {
            let codes = (host.as_deref(), class.as_deref());
            (refused_text(ui, labels, *op, codes, holder.as_ref(), *denied), true)
        }
        Rescued::Unconfirmed(_) => (labels.rescue_unconfirmed.to_owned(), true),
        Rescued::NotSent(_) => (labels.rescue_not_sent.to_owned(), true),
        Rescued::NoThread { error } => {
            let why = fill(labels.thread_failed, &[error.as_str()]);
            (fill(labels.rescue_no_thread, &[why.as_str()]), true)
        }
        Rescued::Restarted => (labels.rescue_runtime_restarted.to_owned(), false),
    }
}

fn sending_text(labels: &Labels, op: DeviceOp) -> &'static str {
    match op {
        DeviceOp::SelfCheck => labels.rescue_sending_check,
        DeviceOp::EmulatorStart => labels.rescue_sending_start,
        DeviceOp::EmulatorStop => labels.rescue_sending_stop,
        DeviceOp::EmulatorRestart => labels.rescue_sending_restart,
        DeviceOp::GameLaunch => labels.rescue_sending_launch,
        DeviceOp::GameRestart => labels.rescue_sending_game_restart,
    }
}

/// The operation, as a refusal names it.
fn op_text(labels: &Labels, op: DeviceOp) -> &'static str {
    match op {
        DeviceOp::SelfCheck => labels.rescue_op_check,
        DeviceOp::EmulatorStart => labels.rescue_op_start,
        DeviceOp::EmulatorStop => labels.rescue_op_stop,
        DeviceOp::EmulatorRestart => labels.rescue_op_restart,
        DeviceOp::GameLaunch => labels.rescue_op_launch,
        DeviceOp::GameRestart => labels.rescue_op_game_restart,
    }
}

/// What a done operation came to, as it answered at `at`, and whether that is a failure. A check
/// that did not pass says why, and of a discovery-bound MuMu instance what may help: a restart
/// of its emulator. Since Runtime 0.12.40 (#402 HS-5) a game that does not run fails no open of
/// MuMu's own pair, so a pair that did not open gets no word of the game. An emulator's control
/// the Runtime carried out says what came of its startup run, unless the emulator's state it
/// read after is not what was asked, which the card then shows. A game's says the command went
/// out: the picture shows where the game is.
fn done_text(
    ui: &Ui,
    labels: &Labels,
    rows: &RescueRows,
    op: DeviceOp,
    at: &str,
    done: &DeviceDone,
) -> (String, bool) {
    match done {
        DeviceDone::SelfCheck { passed: true, size, .. } => {
            let text = match size {
                Some((width, height)) => {
                    fill(labels.rescue_check_passed, &[&format!("{width}×{height}"), at])
                }
                None => fill(labels.rescue_check_passed_unsized, &[at]),
            };
            (text, false)
        }
        DeviceDone::SelfCheck { passed: false, failure, .. } => {
            let words = match failure.as_deref() {
                Some(code) => rescue_code_words(ui, labels, op, (Some(code), None)),
                None => labels.rescue_check_unverified.to_owned(),
            };
            let text = fill(labels.rescue_check_failed, &[&words]);
            if rows.emulator.is_none() {
                return (text, true);
            }
            (fill(labels.rescue_then, &[&text, labels.rescue_check_try_restart]), true)
        }
        DeviceDone::Emulator { running, startup } => {
            let stopping = op == DeviceOp::EmulatorStop;
            if *running == stopping {
                let template = if *running {
                    labels.rescue_done_still_running
                } else {
                    labels.rescue_done_not_running
                };
                return (fill(template, &[at]), true);
            }
            if stopping {
                return (fill(labels.rescue_done_stopped, &[at]), false);
            }
            let template = if op == DeviceOp::EmulatorRestart {
                labels.rescue_done_restarted
            } else {
                labels.rescue_done_started
            };
            let startup = match startup {
                Startup::Scheduled => labels.rescue_startup_scheduled,
                Startup::HeldByOwnPause => labels.rescue_startup_own_pause,
                Startup::HeldByAllPause => labels.rescue_startup_all_pause,
                Startup::None => labels.rescue_startup_none,
            };
            (fill(labels.rescue_then, &[&fill(template, &[at]), startup]), false)
        }
        DeviceDone::Game => {
            let template = if op == DeviceOp::GameRestart {
                labels.rescue_done_game_restarted
            } else {
                labels.rescue_done_launched
            };
            (fill(template, &[at]), false)
        }
    }
}

/// Why the Runtime did not do it, with the host's code and the Runtime's (`codes`): that the
/// account was busy, and with what, for a request that waited its turn in vain, and that nothing
/// was done; else the operation, why in plain words, and that nothing was done when the Runtime
/// denied it, or that the card says how the device is when it failed on the way.
fn refused_text(
    ui: &Ui,
    labels: &Labels,
    op: DeviceOp,
    codes: (Option<&str>, Option<&str>),
    holder: Option<&Holder>,
    denied: bool,
) -> String {
    let (host, class) = codes;
    if holder.is_some() || host == Some(LEASE_QUEUE_EXPIRED) || class == Some(LEASE_QUEUE_EXPIRED) {
        // "Another backend switch" is a switch's word for it: here it is the only one.
        let holder = match holder {
            Some(Holder::BackendSwitch) => labels.rescue_holder_backend_switch,
            Some(holder) => holder_words(labels, holder),
            None => labels.backend_holder_other,
        };
        return fill(labels.rescue_busy, &[holder]);
    }
    let words = rescue_code_words(ui, labels, op, codes);
    refusal_sentence(labels, op_text(labels, op), &words, denied)
}

/// A refusal's sentence: what did not go through (`what`), why in plain words, and that nothing
/// was done when the Runtime denied it, or that the card says how the device is when it failed on
/// the way. A device's operation's (`refused_text`), and a stop's of a run (`stop_text`).
fn refusal_sentence(labels: &Labels, what: &str, words: &str, denied: bool) -> String {
    let end = if denied { labels.rescue_nothing_done } else { labels.rescue_see_card };
    fill(labels.rescue_refused, &[what, words, end])
}

/// A refused or failed operation's reason in plain words, never its code, by the host's code first
/// and then the Runtime's (`codes`, `code_words`); else that it is not known here, which the log
/// hears of once (the operation's own line has the codes already). A refusal of an emulator's
/// control the capability profile offers is out of the ordinary, as the bar drew its button by
/// that profile: the log hears of it once, as the problem it is.
fn rescue_code_words(
    ui: &Ui,
    labels: &Labels,
    op: DeviceOp,
    codes: (Option<&str>, Option<&str>),
) -> String {
    let (host, class) = codes;
    let known: Vec<&str> = [host, class].into_iter().flatten().collect();
    for code in known.iter().copied() {
        if code == EMULATOR_CONTROL_UNAVAILABLE || code == EMULATOR_CONTROL_UNSUPPORTED {
            let line = format!(
                "{op:?} refused with {code}, although the instance's capability profile offers \
                 its emulator's control"
            );
            if ui.model.borrow_mut().first_told(&line) {
                report::log(RESCUE_LOG, &line);
            }
        }
        if rescue_own_words(labels, code).is_some() {
            break;
        }
    }
    if let Some(words) = code_words(labels, &known) {
        return words;
    }
    let line = format!("no words for {op:?} refused or failed with host code {host:?}, code {class:?}");
    if ui.model.borrow_mut().first_told(&line) {
        report::log_quiet(RESCUE_LOG, &line);
    }
    labels.rescue_cause_unknown.to_owned()
}

/// A reason in plain words, never its code, by `codes` in their order: the bar's own words for a
/// code it knows; else the code table's, whose category speaks for a code it gives no words of its
/// own; none when neither has any, which the caller says and logs. A device's operation's
/// (`rescue_code_words`), and a stop's of a run (`stop_words`).
fn code_words(labels: &Labels, codes: &[&str]) -> Option<String> {
    if let Some(words) = codes.iter().copied().find_map(|code| rescue_own_words(labels, code)) {
        return Some(words.to_owned());
    }
    for code in codes.iter().copied() {
        match explain(code, labels.tongue.outcome_tongue()) {
            Ok(Some(words)) => return Some(words.short.to_owned()),
            Ok(None) => {}
            Err(error) => return Some(fill(labels.outcome_table_unusable, &[&error.to_string()])),
        }
    }
    None
}

/// The bar's own words for `code`, when it has any.
fn rescue_own_words(labels: &Labels, code: &str) -> Option<&'static str> {
    let words = match code {
        EMULATOR_CONTROL_BUSY => labels.rescue_emulator_busy,
        EMULATOR_CONTROL_WAIT_TIMEOUT | MUMU_MANAGER_WAIT_TIMEOUT => {
            labels.rescue_emulator_not_ready
        }
        EMULATOR_CONTROL_ADB_NOT_READY => labels.rescue_adb_not_ready,
        EMULATOR_RESTART_STOP_FAILED => labels.rescue_old_not_stopped,
        EMULATOR_RESTART_START_FAILED => labels.rescue_new_not_started,
        EMULATOR_RESTART_PREVIOUS_RUNNING => labels.rescue_old_still_running,
        EMULATOR_CONTROL_UNAVAILABLE | EMULATOR_CONTROL_UNSUPPORTED => {
            labels.rescue_not_discovered
        }
        INSTANCE_NOT_RUNNING => labels.rescue_instance_not_running,
        APPLICATION_BACKEND_FAILED => labels.rescue_game_command_failed,
        APPLICATION_NOT_ASSIGNED => labels.rescue_no_game,
        INPUT_BACKEND_OPEN_FAILED => labels.backend_cause_touch_open,
        CAPTURE_BACKEND_OPEN_FAILED => labels.backend_cause_capture_open,
        PAIRED_BACKEND_OPEN_FAILED => labels.backend_cause_nemu_open,
        INPUT_CHECK_FAILED => labels.backend_cause_touch_check,
        CAPTURE_CHECK_FAILED => labels.backend_cause_capture_check,
        _ => return None,
    };
    Some(words)
}

/// What the status left out of an account's rows, into the log once for each account, each as
/// the problem it is: no provenance, which a Runtime states of every instance, so no device
/// button is drawn; no capability profile of an instance that drives a device of its own, which
/// a Runtime states of every such instance (only a fixture has none), so no emulator's row; a
/// direct run whose status names no request, or a scheduled run that names no request or no
/// package, which the contract says each always does, so it cannot be stopped or closed from here.
fn rescue_told(ui: &Ui, alias: &str, rows: &RescueRows) {
    let unnamed = match rows.close.map(|row| row.why) {
        Some(CloseWhy::Unidentified) => Some("a direct run without its request: no stop"),
        Some(CloseWhy::CloseUnidentified) => {
            Some("a policy run without its request or its package: no close")
        }
        _ => None,
    };
    if let Some(what) = unnamed {
        let line = format!("{alias}: its status names {what}");
        if ui.model.borrow_mut().first_told(&line) {
            report::log(RESCUE_LOG, &line);
        }
    }
    if rows.unstated.provenance {
        let line = format!("{alias}: its status states no backend provenance: no device button");
        if ui.model.borrow_mut().first_told(&line) {
            report::log(RESCUE_LOG, &line);
        }
    }
    if rows.unstated.capabilities && rows.check.is_some() {
        let line = format!("{alias}: its status states no capability profile: no emulator's row");
        if ui.model.borrow_mut().first_told(&line) {
            report::log(RESCUE_LOG, &line);
        }
    }
}

/// The rules panel of `alias` (UI-3e1-b) while it is open (`view`): its words, its own quiet
/// times, and its form's boxes and fields, each set only when it says otherwise, so that a paint
/// every few seconds neither moves a row nor takes a field from the person typing in it. Closed,
/// it holds nothing, and its boxes and fields are as empty as the form is then.
fn rules_panel(
    ui: &Ui,
    window: &WorkbenchWindow,
    labels: &Labels,
    alias: &str,
    view: Option<&RulesView>,
) {
    let (data, lines, form) = match view {
        Some(view) => rules_data(ui, labels, alias, view),
        None => (RulesData::default(), Vec::new(), None),
    };
    sync(window.get_quiet_lines(), lines, |rows| window.set_quiet_lines(rows));
    quiet_form_fields(window, QuietTarget::Account, form.as_ref());
    if window.get_rules() != data {
        window.set_rules(data);
    }
}

/// The panel's words for `view`, its quiet times' lines and its form while open. While the rules
/// cannot be read, only why: nothing an older snapshot said of them shows.
fn rules_data(
    ui: &Ui,
    labels: &Labels,
    alias: &str,
    view: &RulesView,
) -> (RulesData, Vec<QuietLine>, Option<QuietFormView>) {
    let title: SharedString = fill(labels.rules_title, &[alias]).into();
    let (hold, failed) = match view {
        RulesView::Read(rules) => return rules_read(ui, labels, alias, title, rules),
        RulesView::Reading => (labels.rules_reading, false),
        RulesView::ReadFailed { rereading: false } => (labels.rules_read_failed, true),
        RulesView::ReadFailed { rereading: true } => (labels.rules_reread_failed, true),
        // The same line the settings page tells the log, so it is told once.
        RulesView::Unreported => {
            tell_once(ui, "the fact snapshot has no config.parameters record", true);
            (labels.rules_read_failed, true)
        }
    };
    let data = RulesData { title, hold: hold.into(), hold_failed: failed, ..RulesData::default() };
    (data, Vec::new(), None)
}

/// The panel of an account whose rules read. Why none can be changed now, if anything holds them
/// all: a change of its own out says so on its own line, as one saved does until the newest
/// settings show it. Each rule: where its value comes from and when a change takes effect; why it
/// cannot be changed when it cannot, whatever holds the others; what the last change of it came
/// to. Its own quiet times are the editor every account's are as well (`quiet_data`), under which
/// it says whether every account's add to them, pointing at the settings page that changes them.
/// A list or an idle action that does not read, or the host's list, goes to the log once, why
/// with it.
fn rules_read(
    ui: &Ui,
    labels: &Labels,
    alias: &str,
    title: SharedString,
    rules: &AccountRules,
) -> (RulesData, Vec<QuietLine>, Option<QuietFormView>) {
    let view = ui.model.borrow().actions_view();
    let id = rules.instance_id.as_str();
    let whose = format!("instance {id}");
    let quiet = &rules.quiet;
    let idle = &rules.idle;
    let start = rules.start.as_ref();
    let start_said = start.and_then(|start| start.said.as_ref());
    let saved = [quiet.said.as_ref(), idle.said.as_ref(), start_said]
        .into_iter()
        .flatten()
        .any(|said| matches!(said, Said::ConfigApplied | Said::BackToFile));
    let hold = rules_hold_text(labels, rules.hold.as_ref(), saved);

    let of = QuietOf::Instance(id.to_owned());
    let ask = fill(labels.rules_clear_ask, &[alias]);
    let (quiet_words, lines, form) = quiet_data(ui, labels, &view, &of, ask, quiet);
    let host = match &rules.host_quiet {
        QuietRead::Read { list, .. } if list.is_empty() => labels.rules_host_none,
        QuietRead::Read { .. } => labels.rules_host_some,
        QuietRead::Unread(why) => {
            if let Some(why) = why {
                tell_once(ui, &quiet_unread_line(&QuietOf::Host, why), true);
            }
            labels.rules_host_unread
        }
    };
    let quiet_words = QuietData { other: host.into(), ..quiet_words };

    let (idle_nothing, idle_close, idle_source, idle_now, idle_why) = match &idle.now {
        Some(now) => {
            let (nothing, close, words) = match &now.value {
                IdleNow::Nothing => (true, false, None),
                IdleNow::CloseGame => (false, true, None),
                IdleNow::StopEmulator => (false, false, Some(labels.rules_idle_stop)),
                IdleNow::Auto => (false, false, Some(labels.rules_idle_auto)),
                IdleNow::Other(word) => {
                    let line = format!("instance {id}: no words for idle action {word:?}");
                    tell_once(ui, &line, false);
                    (false, false, Some(labels.rules_idle_other))
                }
            };
            // Either choice replaces it, while the Runtime takes a change.
            let now_text = match words {
                Some(words) if now.settable => {
                    fill(labels.rescue_then, &[words, labels.rules_idle_replace])
                }
                Some(words) => words.to_owned(),
                None => String::new(),
            };
            let why = if now.settable { "" } else { labels.rules_file_restart };
            let source = source_effect(labels, now.source, labels.setting_effect_idle);
            (nothing, close, source, now_text, why)
        }
        None => (false, false, String::new(), String::new(), labels.rules_idle_unread),
    };
    let mut idle_note = labels.rules_idle_note.to_owned();
    if idle.paused {
        idle_note = fill(labels.rescue_then, &[&idle_note, labels.rules_idle_paused]);
    }
    if idle.no_device {
        idle_note = fill(labels.rescue_then, &[&idle_note, labels.rules_idle_no_device]);
    }
    let (idle_line, idle_failed) =
        rule_said_text(ui, labels, &view, &whose, RuleItem::Idle, idle.said.as_ref());

    let (start_on, start_off, start_source, start_why) = match start {
        None => (false, false, String::new(), ""),
        Some(StartRule { now: None, .. }) => {
            (false, false, String::new(), labels.rules_start_unread)
        }
        Some(StartRule { now: Some(now), held_on, .. }) => {
            let why = if !now.settable {
                labels.rules_file_restart
            } else if *held_on {
                labels.rules_start_held
            } else {
                ""
            };
            let source = source_effect(labels, now.source, labels.setting_effect_autostart);
            (now.value, !now.value, source, why)
        }
    };
    let (start_line, start_failed) = match start {
        Some(start) => {
            rule_said_text(ui, labels, &view, &whose, RuleItem::Start, start.said.as_ref())
        }
        None => (String::new(), false),
    };
    let start_can_write = start.is_some_and(|start| start.can_write);
    // Off, or back to the file's value, which may be off, not while the idle action needs it on.
    let start_can_off = start_can_write && !start.is_some_and(|start| start.held_on);

    let data = RulesData {
        title,
        hold: hold.into(),
        hold_failed: false,
        shown: true,
        quiet: quiet_words,
        idle_nothing,
        idle_close,
        idle_can_write: idle.can_write,
        idle_source: idle_source.into(),
        idle_now: idle_now.into(),
        idle_why: idle_why.into(),
        idle_note: idle_note.into(),
        idle_back: idle.back_to_file,
        idle_line: idle_line.into(),
        idle_failed,
        start_shown: start.is_some(),
        start_on,
        start_off,
        start_can_on: start_can_write,
        start_can_off,
        start_source: start_source.into(),
        start_why: start_why.into(),
        start_back: start.is_some_and(|start| start.back_to_file),
        start_back_offered: start_can_off,
        start_line: start_line.into(),
        start_failed,
    };
    (data, lines, form)
}

/// Why no rule of an account, or none of every account's quiet times, can be changed now: nothing
/// while its own change is out, which its line says, nor once one was saved (`saved`) while the
/// newest settings are read, which its line says too.
fn rules_hold_text(labels: &Labels, hold: Option<&RulesHold>, saved: bool) -> String {
    match hold {
        None | Some(RulesHold::OwnWrite) => String::new(),
        Some(RulesHold::Switching { account }) => {
            fill(labels.rules_switching, &[account.as_deref().unwrap_or(labels.start_account_gone)])
        }
        Some(RulesHold::OtherWrite) => labels.rules_other_write.to_owned(),
        Some(RulesHold::RowsBehind) if saved => String::new(),
        Some(RulesHold::RowsBehind) => labels.rules_rows_behind.to_owned(),
    }
}

/// What the log hears, once, of the quiet times of `of` that do not read: they are shown so, and
/// not changed.
fn quiet_unread_line(of: &QuietOf, why: &str) -> String {
    format!("{of}: its quiet times are shown, not changed: {why}")
}

/// One editor of quiet times in words (UI-3e1-c), the window's `QuietTimes` of either list: an
/// account's own on its rules panel (`of` an instance), or every account's on the settings page.
/// Its title and what the times do, which tell the two apart; where the list comes from and when a
/// change takes effect; its times' lines, or that it has none; why it cannot be changed when the
/// Runtime does not take a change while it runs, or that it does not read (to the log once, why
/// with it); the form while open, and why its last "add" sent nothing; whether "delete all" can be
/// pressed, and the question `ask` its first press asks while it does; "back to the file's
/// setting"; what the last change of it came to, red when it failed. What holds it, and what the
/// other list adds, are its caller's to fill.
fn quiet_data(
    ui: &Ui,
    labels: &Labels,
    view: &ActionsView,
    of: &QuietOf,
    ask: String,
    quiet: &QuietRule,
) -> (QuietData, Vec<QuietLine>, Option<QuietFormView>) {
    let (title, note, none, unread) = match of {
        QuietOf::Instance(_) => (
            labels.setting_quiet,
            labels.rules_quiet_note,
            labels.rules_quiet_none,
            labels.rules_quiet_unread,
        ),
        QuietOf::Host => (
            labels.setting_quiet_host,
            labels.rules_host_note,
            labels.rules_host_empty,
            labels.rules_host_unread,
        ),
    };
    let (lines, source, empty, why) = match &quiet.read {
        QuietRead::Read { list, source, settable } => {
            let local = local_utc_offset_minutes();
            let lines: Vec<QuietLine> =
                list.entries().map(|entry| quiet_entry(labels, entry, local)).collect();
            let empty = if list.is_empty() { none } else { "" };
            let why = if *settable { "" } else { labels.rules_file_restart };
            (lines, source_effect(labels, *source, labels.setting_effect_round), empty, why)
        }
        QuietRead::Unread(why) => {
            if let Some(why) = why {
                tell_once(ui, &quiet_unread_line(of, why), true);
            }
            (Vec::new(), String::new(), "", unread)
        }
    };
    let form = quiet.form.clone();
    let problem = form
        .as_ref()
        .and_then(|form| form.problem)
        .map_or("", |problem| form_problem_text(labels, problem));
    let confirm = if quiet.confirming { ask } else { String::new() };
    let (line, failed) =
        rule_said_text(ui, labels, view, &of.to_string(), RuleItem::Quiet, quiet.said.as_ref());
    let data = QuietData {
        shown: true,
        title: title.into(),
        source: source.into(),
        note: note.into(),
        hold: SharedString::new(),
        empty: empty.into(),
        why: why.into(),
        can_write: quiet.can_write,
        form_open: form.is_some(),
        problem: problem.into(),
        can_clear: quiet.can_clear,
        confirm: confirm.into(),
        back: quiet.back_to_file,
        other: SharedString::new(),
        line: line.into(),
        failed,
    };
    (data, lines, form)
}

/// The settings page's editor of every account's quiet times (UI-3e1-c), as `quiet_data` has it:
/// above it why it cannot be changed now, as an account's rules panel says it of its rules; under
/// it that each account may have quiet times of its own as well, changed on the main page.
fn host_quiet_data(
    ui: &Ui,
    labels: &Labels,
    host: &HostQuiet,
) -> (QuietData, Vec<QuietLine>, Option<QuietFormView>) {
    let view = ui.model.borrow().actions_view();
    let quiet = &host.quiet;
    let saved = quiet.said.as_ref().is_some_and(|said| {
        matches!(said, Said::ConfigApplied | Said::BackToFile)
    });
    let hold = rules_hold_text(labels, host.hold.as_ref(), saved);
    let ask = labels.rules_host_clear_ask.to_owned();
    let (data, lines, form) = quiet_data(ui, labels, &view, &QuietOf::Host, ask, quiet);
    let data = QuietData { hold: hold.into(), other: labels.rules_host_also.into(), ..data };
    (data, lines, form)
}

/// One quiet time in words. A weekly one by its days, Monday first, and its start and end of its
/// own clock, an end the next day or a whole day said so, the whole of each day when it starts at
/// 0:00 (UI-3e1-c), and that clock beside it when it is not this computer's (`local`); one once by
/// when it starts and ends, of this computer's clock.
fn quiet_entry(labels: &Labels, entry: QuietShown, local: i16) -> QuietLine {
    match entry {
        QuietShown::Weekly { days, start_minute, end_minute, utc_offset_minutes } => {
            let names: Vec<&str> = days
                .iter()
                .zip(labels.rules_weekdays)
                .filter(|(on, _)| **on)
                .map(|(_, name)| name)
                .collect();
            let days = names.join(labels.name_list);
            let start = clock_words(start_minute);
            let text = if end_minute == start_minute && start_minute == 0 {
                fill(labels.rules_quiet_weekly_day, &[&days])
            } else if end_minute == start_minute {
                fill(labels.rules_quiet_weekly_whole, &[&days, &start])
            } else {
                let end = clock_words(end_minute);
                let end = if end_minute < start_minute {
                    fill(labels.rules_next_day, &[&end])
                } else {
                    end
                };
                fill(labels.rules_quiet_weekly, &[&days, &start, &end])
            };
            let zone = if utc_offset_minutes == local {
                String::new()
            } else {
                fill(labels.rules_quiet_zone, &[&utc_words(utc_offset_minutes)])
            };
            QuietLine { text: text.into(), zone: zone.into() }
        }
        QuietShown::Once { from_ms, until_ms } => {
            let (from, until) = (format_minute(from_ms), format_minute(until_ms));
            QuietLine {
                text: fill(labels.rules_quiet_once, &[&from, &until]).into(),
                zone: SharedString::new(),
            }
        }
    }
}

/// A minute of the day as `hh:mm`.
fn clock_words(minute: u16) -> String {
    format!("{:02}:{:02}", minute / 60, minute % 60)
}

/// A clock `offset` minutes east of UTC, as `UTC+9`, `UTC+5:30` or `UTC-3`.
fn utc_words(offset: i16) -> String {
    let sign = if offset < 0 { '-' } else { '+' };
    let minutes = offset.unsigned_abs();
    match (minutes / 60, minutes % 60) {
        (0, 0) => "UTC".to_owned(),
        (hours, 0) => format!("UTC{sign}{hours}"),
        (hours, minutes) => format!("UTC{sign}{hours}:{minutes:02}"),
    }
}

/// Where a rule's value comes from, and when a change of it takes effect.
fn source_effect(labels: &Labels, source: SettingSource, effect: &str) -> String {
    let source = fill(labels.setting_source_line, &[source_text(labels, source)]);
    format!("{source} · {effect}")
}

/// Why the form's "add" sent nothing.
fn form_problem_text(labels: &Labels, problem: FormProblem) -> &'static str {
    match problem {
        FormProblem::NoDay => labels.rules_form_no_day,
        FormProblem::NotTime => labels.rules_form_not_time,
        FormProblem::SameTimes => labels.rules_form_same_time,
        FormProblem::TooBig => labels.rules_form_too_big,
        FormProblem::NotTaken => labels.rules_form_not_taken,
    }
}

/// What the last change of the rule `item` came to, on its own line, and whether that is a
/// failure; nothing before one. `whose` names its list or its account for the log: an
/// instance's, or the host's for every account's quiet times (UI-3e1-c). Saved, that the newest
/// settings are being read, until they show it (D-28). The refusals only these rules meet in their
/// own words: an emulator's start on for an instance not bound by discovery; the check that holds
/// the emulator's start on while the idle action stops the emulator, from either side; and the
/// whole section's check of quiet times, which the console's own check of them passed, so the log
/// hears of it as the problem it is. Anything else as any change of settings is said.
fn rule_said_text(
    ui: &Ui,
    labels: &Labels,
    view: &ActionsView,
    whose: &str,
    item: RuleItem,
    said: Option<&Said>,
) -> (String, bool) {
    let Some(said) = said else {
        return (String::new(), false);
    };
    let problem = match said {
        Said::ConfigApplied | Said::BackToFile => {
            return (labels.rules_saved_reading.to_owned(), false);
        }
        Said::ConfigRefused(RefusalWhy::Value { problem, .. }) => Some(*problem),
        _ => None,
    };
    let own = match (item, problem) {
        (RuleItem::Start, Some(ValueProblem::DiscoveryBindingRequired)) => {
            Some(labels.rules_start_not_discovered)
        }
        (RuleItem::Start, Some(ValueProblem::SectionCheck)) => {
            Some(labels.rules_start_held_refused)
        }
        (RuleItem::Idle, Some(ValueProblem::SectionCheck)) => Some(labels.rules_idle_section),
        (RuleItem::Quiet, Some(ValueProblem::SectionCheck)) => {
            let line = format!(
                "{whose}: the Runtime refused its quiet times by a check of the whole section, \
                 which the same parser passed here: the Runtime is not the pinned one, or \
                 another check of the section refused them"
            );
            tell_once(ui, &line, true);
            Some(labels.rules_quiet_refused)
        }
        _ => None,
    };
    match own {
        Some(words) => (words.to_owned(), true),
        None => said_text(labels, view, said),
    }
}

/// The form's seven boxes, two fields and "whole day" as `form` holds them, or empty while it is
/// not open: each set only when it says otherwise, as the box flips itself and the field holds
/// what is typed before Rust hears of it. Each editor has a set of its own in the window
/// (`target`): the rules panel's paint sets its own back from its form, which would otherwise
/// take from the settings page what is being typed there.
fn quiet_form_fields(window: &WorkbenchWindow, target: QuietTarget, form: Option<&QuietFormView>) {
    let (days, start, end, whole_day) = match form {
        Some(form) => (form.days, form.start.as_str(), form.end.as_str(), form.whole_day),
        None => ([false; 7], "", "", false),
    };
    match target {
        QuietTarget::Account => {
            put_flag(window.get_quiet_day_0(), days[0], |on| window.set_quiet_day_0(on));
            put_flag(window.get_quiet_day_1(), days[1], |on| window.set_quiet_day_1(on));
            put_flag(window.get_quiet_day_2(), days[2], |on| window.set_quiet_day_2(on));
            put_flag(window.get_quiet_day_3(), days[3], |on| window.set_quiet_day_3(on));
            put_flag(window.get_quiet_day_4(), days[4], |on| window.set_quiet_day_4(on));
            put_flag(window.get_quiet_day_5(), days[5], |on| window.set_quiet_day_5(on));
            put_flag(window.get_quiet_day_6(), days[6], |on| window.set_quiet_day_6(on));
            put_text(window.get_quiet_start(), start, |text| window.set_quiet_start(text));
            put_text(window.get_quiet_end(), end, |text| window.set_quiet_end(text));
            put_flag(window.get_quiet_whole_day(), whole_day, |on| {
                window.set_quiet_whole_day(on);
            });
        }
        QuietTarget::Host => {
            put_flag(window.get_host_quiet_day_0(), days[0], |on| window.set_host_quiet_day_0(on));
            put_flag(window.get_host_quiet_day_1(), days[1], |on| window.set_host_quiet_day_1(on));
            put_flag(window.get_host_quiet_day_2(), days[2], |on| window.set_host_quiet_day_2(on));
            put_flag(window.get_host_quiet_day_3(), days[3], |on| window.set_host_quiet_day_3(on));
            put_flag(window.get_host_quiet_day_4(), days[4], |on| window.set_host_quiet_day_4(on));
            put_flag(window.get_host_quiet_day_5(), days[5], |on| window.set_host_quiet_day_5(on));
            put_flag(window.get_host_quiet_day_6(), days[6], |on| window.set_host_quiet_day_6(on));
            put_text(window.get_host_quiet_start(), start, |text| {
                window.set_host_quiet_start(text);
            });
            put_text(window.get_host_quiet_end(), end, |text| window.set_host_quiet_end(text));
            put_flag(window.get_host_quiet_whole_day(), whole_day, |on| {
                window.set_host_quiet_whole_day(on);
            });
        }
    }
}

/// A box of the window set to `on`, through `set`, only when it shows otherwise (`now`).
fn put_flag(now: bool, on: bool, set: impl FnOnce(bool)) {
    if now != on {
        set(on);
    }
}

/// A field of the window set to `text`, through `set`, only when it holds otherwise (`now`).
fn put_text(now: SharedString, text: &str, set: impl FnOnce(SharedString)) {
    if now != text {
        set(text.into());
    }
}

/// The top bar's presets: the one in effect marked, what is in effect in words under them,
/// and whether they can be pressed; nothing while there is no Runtime to say it of.
/// And what a first press on a preset asks to confirm, and the stepper of how many tasks run
/// at once, which goes by the same "can be pressed".
pub fn preset(ui: &Ui) {
    let Some(window) = ui.workbench() else {
        return;
    };
    let labels = ui.labels();
    let view = ui.model.borrow().preset_view();
    let (choices, note, failed) = match &view {
        PresetView::NotRead => (Vec::new(), String::new(), false),
        PresetView::Unreadable => (Vec::new(), labels.preset_unreadable.to_owned(), true),
        PresetView::Read { options, state, .. } => {
            let choices: Vec<PresetChoice> = options
                .iter()
                .map(|option| PresetChoice {
                    name: option.name.as_str().into(),
                    label: preset_label(labels, &option.name).into(),
                    current: option.current,
                })
                .collect();
            (choices, preset_note(labels, state), false)
        }
    };
    let (writable, back_to_file, statement) = match &view {
        PresetView::NotRead | PresetView::Unreadable => (false, false, String::new()),
        PresetView::Read { can_write, back_to_file, confirming, .. } => {
            let statement = match confirming {
                Some(name) => fill(labels.preset_leave_full, &[preset_label(labels, name)]),
                None => String::new(),
            };
            (*can_write, *back_to_file, statement)
        }
    };
    window.set_preset_shown(!matches!(view, PresetView::NotRead));
    window.set_preset_writable(writable);
    window.set_preset_back_offered(back_to_file);
    window.set_preset_statement(statement.into());
    sync(window.get_presets(), choices, |rows| window.set_presets(rows));
    window.set_preset_note(note.into());
    window.set_preset_failed(failed);
    let stepper = match &view {
        PresetView::Read { stepper: Some(stepper), .. } => Some(stepper_lines(labels, stepper)),
        PresetView::Read { stepper: None, .. } | PresetView::NotRead | PresetView::Unreadable => {
            None
        }
    };
    window.set_concurrency_shown(stepper.is_some());
    if let Some(lines) = stepper {
        // The field is the person's while they type: it is set only when it says otherwise.
        if window.get_concurrency_text() != lines.text.as_str() {
            window.set_concurrency_text(lines.text.into());
        }
        window.set_concurrency_placeholder(lines.placeholder.into());
        window.set_concurrency_note(lines.note.into());
        window.set_concurrency_ready(writable && lines.ready);
        window.set_concurrency_back_offered(lines.back_to_file);
    }
}

/// What the stepper shows.
struct StepperLines {
    /// The draft, or the value in effect; empty when there is none.
    text: String,
    /// In the empty field: that a draft left empty goes back to the file's setting, or
    /// that there is no value.
    placeholder: &'static str,
    /// Where the value in effect comes from, and the value too while the field holds a draft.
    note: String,
    ready: bool,
    back_to_file: bool,
}

fn stepper_lines(labels: &Labels, stepper: &StepperView) -> StepperLines {
    let source = source_text(labels, stepper.source);
    let (text, placeholder, note) = match &stepper.draft {
        Some(draft) => {
            let value = match &stepper.value {
                Some(value) => value_text(labels, value),
                None => labels.setting_unset.to_owned(),
            };
            let note = fill(labels.concurrency_now, &[&value, source]);
            (draft.clone(), labels.concurrency_empty, note)
        }
        None => {
            let value = match &stepper.value {
                Some(value) => value_text(labels, value),
                None => String::new(),
            };
            (value, labels.setting_unset, fill(labels.concurrency_source, &[source]))
        }
    };
    StepperLines {
        text,
        placeholder,
        note,
        ready: stepper.can_apply,
        back_to_file: stepper.back_to_file,
    }
}

/// A preset's name in words; one the words here do not know, by its own name.
fn preset_label<'a>(labels: &Labels, name: &'a str) -> &'a str {
    match name {
        PRESET_STANDARD => labels.preset_standard,
        PRESET_ADAPTIVE => labels.preset_adaptive,
        PRESET_LIGHT => labels.preset_light,
        other => other,
    }
}

fn preset_note(labels: &Labels, state: &PresetState) -> String {
    match state {
        PresetState::Named(name) => match name.as_str() {
            PRESET_STANDARD => labels.preset_standard_note.to_owned(),
            PRESET_ADAPTIVE => labels.preset_adaptive_note.to_owned(),
            PRESET_LIGHT => labels.preset_light_note.to_owned(),
            _ => String::new(),
        },
        PresetState::CustomFileFull => labels.preset_custom_file_full.to_owned(),
        PresetState::CustomFileConcurrency => labels.preset_custom_file_concurrency.to_owned(),
        PresetState::Custom(members) => {
            let members: Vec<String> =
                members.iter().map(|member| member_text(labels, member)).collect();
            fill(labels.preset_custom, &[&members.join(" · ")])
        }
    }
}

/// One setting of a preset: what it is, its value in effect and where that comes from.
fn member_text(labels: &Labels, member: &PresetMember) -> String {
    let name = preset_setting_name(labels, member.setting);
    let value = match &member.value {
        None => labels.setting_unset.to_owned(),
        Some(value) => value_text(labels, value),
    };
    fill(labels.preset_member, &[name, &value, source_text(labels, member.source)])
}

fn preset_setting_name(labels: &Labels, setting: PresetSetting) -> &'static str {
    match setting {
        PresetSetting::Mode => labels.setting_mode,
        PresetSetting::MaxConcurrentRuns => labels.setting_concurrent_runs,
        PresetSetting::HeavyDispatchLimit => labels.setting_heavy_runs,
        PresetSetting::ParallelEnabled => labels.setting_parallel,
    }
}

fn value_text(labels: &Labels, value: &SettingValue) -> String {
    match value {
        SettingValue::Flag(true) => labels.setting_on.to_owned(),
        SettingValue::Flag(false) => labels.setting_off.to_owned(),
        SettingValue::Number(number) => number.to_string(),
        SettingValue::Word(word) => match word.as_str() {
            MODE_OBSERVE => labels.setting_mode_observe.to_owned(),
            MODE_PACE => labels.setting_mode_pace.to_owned(),
            MODE_FULL => labels.setting_mode_full.to_owned(),
            other => other.to_owned(),
        },
    }
}

/// The settings page's rows of the Runtime's settings, in their two groups, the editor of every
/// account's quiet times heading the everyday one (`host_quiet_data`, UI-3e1-c), with the field of
/// the evidence days in its place among the advanced ones (`evidence_field`), and what is said
/// instead of them, or besides: while the reads fail and a change of every account's quiet times
/// awaits a snapshot, that what came of it is not known either. A setting without a name here and
/// a snapshot that reports no configuration each go to the log once; rows that do not read, each
/// by its key and why, once for each owner epoch.
pub fn settings(ui: &Ui) {
    let Some(window) = ui.workbench() else {
        return;
    };
    let labels = ui.labels();
    // With no state root nothing reads the Runtime, so nothing is on its way either.
    let view = match &ui.launcher {
        Ok(_) => ui.model.borrow().settings_view(),
        Err(_) => SettingsView::Unreachable,
    };
    let (note, failed) = match &view {
        SettingsView::NotRunning => (labels.settings_table_not_running.to_owned(), false),
        SettingsView::Unreachable => (labels.settings_table_unreachable.to_owned(), false),
        SettingsView::Reading => (labels.settings_table_reading.to_owned(), false),
        // The read's failure is in the log already, with the snapshot's (`config_landed`).
        SettingsView::Failed => {
            let rereading =
                ui.model.borrow().actions_view().reread_failed == Some(Field::HostQuiet);
            let note = if rereading {
                labels.rules_reread_failed
            } else {
                labels.settings_table_unreadable
            };
            (note.to_owned(), true)
        }
        SettingsView::Unreported => {
            tell_once(ui, "the fact snapshot has no config.parameters record", true);
            (labels.settings_table_unreadable.to_owned(), true)
        }
        SettingsView::Read { unread, .. } if unread.is_empty() => (String::new(), false),
        SettingsView::Read { unread, .. } => {
            let count = unread.len().to_string();
            // Each row by its key and why, with the owner epoch: a Runtime started again
            // that reports the same rows is said again.
            let epoch = ui
                .model
                .borrow()
                .config_read()
                .map_or_else(|| "-".to_owned(), |read| read.epoch.clone());
            let rows: Vec<String> = unread.iter().map(|row| row.line()).collect();
            let line = format!(
                "owner epoch {epoch}: {count} config.parameters rows do not read: {}",
                rows.join("; ")
            );
            tell_once(ui, &line, true);
            (fill(labels.settings_table_rows_unreadable, &[&count]), true)
        }
    };
    let (daily, advanced, after, evidence) = match &view {
        SettingsView::Read { daily, advanced, evidence, advanced_after, .. } => (
            setting_items(ui, labels, daily),
            setting_items(ui, labels, advanced),
            setting_items(ui, labels, advanced_after),
            evidence.as_ref(),
        ),
        SettingsView::NotRunning
        | SettingsView::Unreachable
        | SettingsView::Reading
        | SettingsView::Failed
        | SettingsView::Unreported => (Vec::new(), Vec::new(), Vec::new(), None),
    };
    let groups: &[BackendGroup] = match &view {
        SettingsView::Read { backends, unlisted, .. } => {
            for instance in unlisted {
                let line = format!(
                    "instance {instance}: the snapshot names its backends and the Status lists \
                     no such instance; no group for it"
                );
                tell_once(ui, &line, false);
            }
            backends.as_slice()
        }
        SettingsView::NotRunning
        | SettingsView::Unreachable
        | SettingsView::Reading
        | SettingsView::Failed
        | SettingsView::Unreported => &[],
    };
    let (host, host_lines, host_form) = match &view {
        SettingsView::Read { host_quiet, .. } => host_quiet_data(ui, labels, host_quiet),
        SettingsView::NotRunning
        | SettingsView::Unreachable
        | SettingsView::Reading
        | SettingsView::Failed
        | SettingsView::Unreported => (QuietData::default(), Vec::new(), None),
    };
    window.set_runtime_settings_note(note.into());
    window.set_runtime_settings_failed(failed);
    // Its lines and its form's boxes and fields set only when they say otherwise, as the rules
    // panel's are: a paint every few seconds neither moves a row nor takes a field from the
    // person typing in it.
    sync(window.get_host_quiet_lines(), host_lines, |rows| window.set_host_quiet_lines(rows));
    quiet_form_fields(&window, QuietTarget::Host, host_form.as_ref());
    if window.get_host_quiet() != host {
        window.set_host_quiet(host);
    }
    sync(window.get_daily_settings(), daily, |rows| window.set_daily_settings(rows));
    sync(window.get_advanced_settings(), advanced, |rows| window.set_advanced_settings(rows));
    evidence_field(&window, labels, evidence);
    sync(window.get_advanced_settings_after(), after, |rows| {
        window.set_advanced_settings_after(rows);
    });
    backend_groups(ui, &window, labels, groups);
}

/// Each device's group of its backends, a row of the list the window shows at a time. A box's
/// choices keep the list they had while their words do not change, and a row whose words do
/// not change is left as it is: a paint every few seconds neither closes an open box nor takes
/// its focus. What the log is to know of them goes there once: a word without a name here, a
/// kind of device or a self-check that does not read, an online value with no record of it.
fn backend_groups(ui: &Ui, window: &WorkbenchWindow, labels: &Labels, groups: &[BackendGroup]) {
    if !groups.is_empty() {
        let words = unnamed_backend_words();
        if !words.is_empty() {
            let line = format!("no words for backends {}: not offered", words.join(", "));
            tell_once(ui, &line, false);
        }
    }
    let shown = window.get_backend_groups();
    let old = |at: usize| -> Option<BackendGroupItem> {
        shown.as_any().downcast_ref::<VecModel<BackendGroupItem>>()?.row_data(at)
    };
    let items = groups
        .iter()
        .enumerate()
        .map(|(at, group)| backend_item(ui, labels, group, old(at).as_ref()))
        .collect();
    sync(window.get_backend_groups(), items, |rows| window.set_backend_groups(rows));
}

/// One device's group: its title; each row that reads, its box's choices and the one shown, and
/// in grey where the value comes from, with the value while the box shows another, the file's
/// value under an online one, that a change needs the file, and that "use the file's value"
/// takes the other along; the pair; the self-check, but while its switch is out or unconfirmed;
/// the question; where the group stands.
fn backend_item(
    ui: &Ui,
    labels: &Labels,
    group: &BackendGroup,
    old: Option<&BackendGroupItem>,
) -> BackendGroupItem {
    let backends = &group.backends;
    let edit = &group.edit;
    let instance = backends.instance_id();
    backend_told(ui, backends);
    let touch_words = |backend| settings::touch_words(labels, backend);
    let capture_words = |backend| settings::capture_words(labels, backend);
    let touch = backends.touch.as_ref().zip(edit.touch.as_ref()).map(|(row, edit)| {
        let options = choice_words(edit, touch_words);
        let note = row_note(labels, row, edit, touch_words, labels.backend_with_capture);
        (options, edit, note)
    });
    let capture = backends.capture.as_ref().zip(edit.capture.as_ref()).map(|(row, edit)| {
        let options = choice_words(edit, capture_words);
        let note = row_note(labels, row, edit, capture_words, labels.backend_with_touch);
        (options, edit, note)
    });
    let selfcheck_view = settings::selfcheck_of(backends);
    for (_, status) in &selfcheck_view.entries {
        if let CheckStatus::Unread(word) = status {
            let line = format!("instance {instance}: a self-check status does not read: {word}");
            tell_once(ui, &line, true);
        }
    }
    let (selfcheck, selfcheck_warning) = match edit.state {
        GroupState::Switching | GroupState::Unconfirmed { .. } => (String::new(), false),
        GroupState::Open
        | GroupState::KindUnread
        | GroupState::RowUnread
        | GroupState::Rereading
        | GroupState::Rescuing
        | GroupState::Refreshing
        | GroupState::OtherWrite => (
            fill(labels.backend_selfcheck, &[&selfcheck_words(labels, &selfcheck_view)]),
            selfcheck_view.entries.iter().any(|(_, status)| *status == CheckStatus::Failed),
        ),
    };
    // The client's whole wait, in seconds, only while a switch is out.
    let switching = match edit.state {
        GroupState::Switching => settings::switching_words(labels),
        _ => String::new(),
    };
    let (status, status_failed) = match edit.state {
        GroupState::Open => ("", false),
        GroupState::KindUnread => (labels.backend_kind_unread, true),
        GroupState::RowUnread => (labels.backend_row_unread, true),
        GroupState::Switching => (switching.as_str(), false),
        GroupState::Unconfirmed { over: false } => (labels.backend_unconfirmed, true),
        GroupState::Unconfirmed { over: true } | GroupState::Rereading => {
            (labels.backend_rereading, false)
        }
        GroupState::Rescuing => (labels.backend_rescuing, false),
        GroupState::Refreshing => (labels.backend_refreshing, false),
        GroupState::OtherWrite => (labels.backend_other_write, false),
    };
    let ask_words: Vec<SharedString> = match &edit.question {
        Some(question) => std::iter::once(labels.backend_ask_pick)
            .chain(question.choices.iter().map(|touch| touch_words(*touch)))
            .map(SharedString::from)
            .collect(),
        None => Vec::new(),
    };
    let open = edit.state == GroupState::Open;
    BackendGroupItem {
        instance: instance.into(),
        title: fill(labels.backend_group, &[&group.account]).into(),
        touch_shown: touch.is_some(),
        touch_options: options(old.map(|item| &item.touch_options), options_of(&touch)),
        touch_index: touch.as_ref().map_or(0, |(_, edit, _)| index(edit.shown)),
        touch_enabled: touch.as_ref().is_some_and(|(_, edit, _)| edit.enabled),
        touch_note: touch.as_ref().map_or_else(String::new, |(_, _, note)| note.clone()).into(),
        touch_back: edit.touch.as_ref().is_some_and(|row| row.file_value.is_some()),
        touch_back_enabled: edit
            .touch
            .as_ref()
            .and_then(|row| row.file_value)
            .is_some_and(|offer| offer.enabled),
        capture_shown: capture.is_some(),
        capture_options: options(old.map(|item| &item.capture_options), options_of(&capture)),
        capture_index: capture.as_ref().map_or(0, |(_, edit, _)| index(edit.shown)),
        capture_enabled: capture.as_ref().is_some_and(|(_, edit, _)| edit.enabled),
        capture_note: capture
            .as_ref()
            .map_or_else(String::new, |(_, _, note)| note.clone())
            .into(),
        capture_back: edit.capture.as_ref().is_some_and(|row| row.file_value.is_some()),
        capture_back_enabled: edit
            .capture
            .as_ref()
            .and_then(|row| row.file_value)
            .is_some_and(|offer| offer.enabled),
        paired: edit.paired,
        selfcheck: selfcheck.into(),
        selfcheck_warning,
        asking: edit.question.is_some(),
        ask_options: options(old.map(|item| &item.ask_options), ask_words),
        ask_index: edit
            .question
            .as_ref()
            .and_then(|question| question.picked)
            .map_or(0, |picked| index(picked + 1)),
        ask_enabled: open,
        ask_ready: edit.question.as_ref().is_some_and(|question| question.apply.is_some()),
        apply_shown: !matches!(edit.state, GroupState::KindUnread | GroupState::RowUnread),
        apply_ready: edit.apply.is_some(),
        status: status.into(),
        status_failed,
    }
}

/// What the log is to know of a device's backends, once each: a kind of device that does not
/// read, a value in effect without a name here, an online value with no record of it.
fn backend_told(ui: &Ui, backends: &InstanceBackends) {
    let instance = backends.instance_id();
    if backends.kind.is_err() {
        let line = format!(
            "instance {instance}: its kind of device does not read, so its backends are only \
             shown: {}",
            backends.line()
        );
        tell_once(ui, &line, true);
    }
    let touch = backends.touch.as_ref().map(|row| (word_of(&row.value), row_unstamped(row)));
    let capture = backends.capture.as_ref().map(|row| (word_of(&row.value), row_unstamped(row)));
    for (which, row) in [("touch", touch), ("capture", capture)] {
        let Some((word, unstamped)) = row else {
            continue;
        };
        if let Some(word) = word {
            let line = format!("instance {instance}: no words for its {which} backend {word}");
            tell_once(ui, &line, false);
        }
        if unstamped {
            let line = format!(
                "instance {instance}: its {which} backend is set online and host.config_overrides \
                 has no record of it"
            );
            tell_once(ui, &line, false);
        }
    }
}

/// A word in effect without a name here.
fn word_of<T>(value: &InEffect<T>) -> Option<&str> {
    match value {
        InEffect::Word(word) => Some(word.as_str()),
        InEffect::Named(_) => None,
    }
}

/// An online value the snapshot holds no record of.
fn row_unstamped<T>(row: &BackendRow<T>) -> bool {
    row.source == SettingSource::Online && row.online.is_none()
}

/// The words of a box's choices.
fn choice_words<T: Copy>(
    edit: &RowEdit<T>,
    words: impl Fn(T) -> &'static str,
) -> Vec<SharedString> {
    edit.choices
        .iter()
        .map(|choice| match choice {
            Choice::Backend(backend) => SharedString::from(words(*backend)),
            Choice::Word(word) => SharedString::from(word.as_str()),
        })
        .collect()
}

/// A row's choices in words, none for a row that does not show.
fn options_of<E, N>(row: &Option<(Vec<SharedString>, E, N)>) -> Vec<SharedString> {
    row.as_ref().map_or_else(Vec::new, |(options, _, _)| options.clone())
}

/// A box's list: the one it has while its words are the same, so the box keeps its state.
fn options(old: Option<&ModelRc<SharedString>>, words: Vec<SharedString>) -> ModelRc<SharedString> {
    let same = old.is_some_and(|old| {
        old.row_count() == words.len()
            && words.iter().enumerate().all(|(at, word)| old.row_data(at).as_ref() == Some(word))
    });
    match old {
        Some(old) if same => old.clone(),
        Some(_) | None => models(words),
    }
}

/// A box's index as the window types it; a box has a handful of choices.
fn index(at: usize) -> i32 {
    i32::try_from(at).unwrap_or(i32::MAX)
}

/// A row's grey line: where its value comes from, with the value first while the box shows
/// another; the file's value under an online one; that only the file changes it; and that "use
/// the file's value" takes the other row along (`along`).
fn row_note<T: Copy>(
    labels: &Labels,
    row: &BackendRow<T>,
    edit: &RowEdit<T>,
    words: impl Fn(T) -> &'static str + Copy,
    along: &str,
) -> String {
    let source = backend_source(labels, row);
    let mut note = Vec::new();
    if edit.drafted {
        let value = settings::in_effect_words(&row.value, words);
        note.push(fill(labels.concurrency_now, &[&value, &source]));
    } else {
        note.push(fill(labels.setting_source_line, &[&source]));
    }
    if let Some(file) = row.online.as_ref().and_then(|online| online.file_value.as_ref()) {
        note.push(fill(labels.backend_file_value, &[&settings::in_effect_words(file, words)]));
    }
    if !row.settable {
        note.push(labels.setting_file_restart.to_owned());
    }
    if edit.file_value.is_some_and(|offer| offer.both) {
        note.push(along.to_owned());
    }
    note.join(" · ")
}

/// Where a backend's value comes from: the file, or set online, when and through which way.
fn backend_source<T>(labels: &Labels, row: &BackendRow<T>) -> String {
    let online = match (row.source, &row.online) {
        (SettingSource::Online, Some(online)) => online,
        (source, _) => return source_text(labels, source).to_owned(),
    };
    let mut text = match online.set_unix_ms {
        Some(ms) => fill(labels.backend_online_at, &[&format_minute(ms)]),
        None => labels.setting_source_online.to_owned(),
    };
    if let Some(through) = online.through {
        let through = settings::through_words(labels, through);
        text.push(' ');
        text.push_str(&fill(labels.backend_online_through, &[through]));
    }
    text
}

/// A device's self-check in words: each entry with what it came to, and when the newest was.
fn selfcheck_words(labels: &Labels, view: &SelfCheckView) -> String {
    if view.entries.is_empty() {
        return labels.backend_check_none.to_owned();
    }
    let entries: Vec<String> = view
        .entries
        .iter()
        .map(|(which, status)| {
            let template = match which {
                CheckEntry::Touch => labels.backend_check_touch,
                CheckEntry::Capture => labels.backend_check_capture,
                CheckEntry::Nemu => labels.backend_check_nemu,
            };
            let status = match status {
                CheckStatus::Passed => labels.backend_check_passed,
                CheckStatus::Failed => labels.backend_check_failed,
                CheckStatus::Unsure => labels.backend_check_unsure,
                CheckStatus::Unread(_) => labels.backend_check_unread,
            };
            fill(template, &[status])
        })
        .collect();
    let entries = entries.join(" · ");
    match view.at {
        Some(at) => fill(labels.backend_paren, &[&entries, &format_minute(at)]),
        None => entries,
    }
}

/// The evidence days' field, while it shows: the draft, or the value in effect; what it
/// says when empty; and in grey the value too while the field holds something else, where
/// it comes from and when a change takes effect. Whether it and "save" can be pressed.
fn evidence_field(window: &WorkbenchWindow, labels: &Labels, view: Option<&EvidenceView>) {
    window.set_evidence_shown(view.is_some());
    let Some(view) = view else {
        return;
    };
    let source = source_text(labels, view.source);
    let mut note = Vec::new();
    let (text, placeholder) = match &view.draft {
        Some(draft) => {
            let value = match view.days {
                Some(days) => fill(labels.setting_days, &[&days.to_string()]),
                None => labels.setting_unset.to_owned(),
            };
            note.push(fill(labels.concurrency_now, &[&value, source]));
            (draft.clone(), labels.concurrency_empty)
        }
        None => {
            note.push(fill(labels.setting_source_line, &[source]));
            (view.days.map_or_else(String::new, |days| days.to_string()), labels.setting_unset)
        }
    };
    // The field shows only while the Runtime takes a change of the days.
    let effect = settings::effect_words(labels, &SettingName::EvidenceMaxAgeDays, true);
    note.extend(effect.map(str::to_owned));
    // The field is the person's while they type: it is set only when it says otherwise.
    if window.get_evidence_text() != text.as_str() {
        window.set_evidence_text(text.into());
    }
    window.set_evidence_placeholder(placeholder.into());
    window.set_evidence_note(note.join(" · ").into());
    window.set_evidence_writable(view.can_write);
    window.set_evidence_ready(view.can_write && view.can_save);
}

/// What this owner epoch's start report says of the settings: those it reset, on the
/// notice until a person says they read it, and on the settings page; those it kept over a
/// changed default, on the settings page in grey; or, on the settings page in red, that it
/// cannot be read. A setting without a name here goes to the log once, as on the page.
pub fn start(ui: &Ui) {
    let Some(window) = ui.workbench() else {
        return;
    };
    let labels = ui.labels();
    let view = ui.model.borrow().start_view();
    let (alert, note, failed, base) = match &view {
        StartView::Nothing => (String::new(), String::new(), false, String::new()),
        StartView::Unreadable => {
            (String::new(), labels.start_unreadable.to_owned(), true, String::new())
        }
        StartView::Read { dropped, base_changed, acknowledged } => {
            let dropped = reported_text(ui, labels, labels.start_dropped, dropped);
            let base = reported_text(ui, labels, labels.start_base_changed, base_changed);
            let alert = if *acknowledged { String::new() } else { dropped.clone() };
            (alert, dropped, false, base)
        }
    };
    window.set_start_alert(alert.into());
    window.set_start_note(note.into());
    window.set_start_note_failed(failed);
    window.set_start_base_note(base.into());
}

/// Under the Runtime on the settings page, as one of its settings: whether the Runtime is
/// started again when it is gone, in grey that it is only shown, what that does and when;
/// and under it why it cannot be told, red, or why it may not work, amber, both pointing at
/// the log; or, in grey, that it is read again when the page is next gone to. With no state
/// root nothing runs it, and the red bar at the top says why.
pub fn autostart(ui: &Ui) {
    let Some(window) = ui.workbench() else {
        return;
    };
    let labels = ui.labels();
    let view = match &ui.launcher {
        Ok(_) => ui.model.borrow().autostart_view(),
        Err(_) => AutostartView::Unreachable,
    };
    // The value, and the line under it: its words, red, amber.
    let (value, problem, failed, warning) = match view {
        AutostartView::Unreachable => (labels.autostart_unreadable, "", false, false),
        AutostartView::Reading => (labels.autostart_reading, "", false, false),
        AutostartView::LetGo => {
            (labels.autostart_let_go, labels.autostart_let_go_note, false, false)
        }
        AutostartView::On { warned: false } => (labels.autostart_on, "", false, false),
        AutostartView::On { warned: true } => {
            (labels.autostart_on, labels.autostart_reported_note, false, true)
        }
        AutostartView::Off { warned: false } => (labels.autostart_off, "", false, false),
        AutostartView::Off { warned: true } => {
            (labels.autostart_off, labels.autostart_reported_note, false, true)
        }
        AutostartView::Differs => {
            (labels.autostart_differs, labels.autostart_differs_note, false, true)
        }
        AutostartView::Unreadable => {
            (labels.autostart_unreadable, labels.autostart_unreadable_note, true, false)
        }
        AutostartView::NoPaths => {
            (labels.autostart_unreadable, labels.autostart_no_paths, false, true)
        }
        AutostartView::NotInstalledLayout => {
            (labels.autostart_unreadable, labels.autostart_not_installed, false, true)
        }
    };
    window.set_autostart(SettingItem {
        name: labels.autostart.into(),
        value: value.into(),
        note: [labels.setting_view_only, labels.autostart_note].join(" · ").into(),
    });
    window.set_autostart_problem(problem.into());
    window.set_autostart_failed(failed);
    window.set_autostart_warning(warning);
}

/// `template` with the settings `lines` names, each name once; nothing when it names none.
fn reported_text(ui: &Ui, labels: &Labels, template: &str, lines: &[ReportedLine]) -> String {
    if lines.is_empty() {
        return String::new();
    }
    let mut names: Vec<String> = Vec::new();
    for line in lines {
        let name = match &line.name {
            ReportedName::Preset(setting) => preset_setting_name(labels, *setting),
            ReportedName::Setting(name) => {
                if let SettingName::Unnamed(key) = name {
                    tell_once(ui, &format!("no words for setting {key}"), false);
                }
                settings::name_words(labels, name)
            }
        };
        let name = match &line.account {
            Some(ReportedAccount::Alias(alias)) => fill(labels.setting_of_account, &[name, alias]),
            Some(ReportedAccount::NotListed) => {
                fill(labels.setting_of_account, &[name, labels.start_account_gone])
            }
            None => name.to_owned(),
        };
        if !names.contains(&name) {
            names.push(name);
        }
    }
    fill(template, &[&names.join(labels.name_list)])
}

fn setting_items(ui: &Ui, labels: &Labels, lines: &[SettingLine]) -> Vec<SettingItem> {
    lines.iter().map(|line| setting_item(ui, labels, line)).collect()
}

/// One row: its name, with the account of an instance's own setting; its value; and in
/// grey where the value comes from, how it is changed and when a change takes effect.
fn setting_item(ui: &Ui, labels: &Labels, line: &SettingLine) -> SettingItem {
    if let SettingName::Unnamed(key) = &line.name {
        tell_once(ui, &format!("no words for setting {key}"), false);
    }
    let name = settings::name_words(labels, &line.name);
    let name = match &line.account {
        Some(account) => fill(labels.setting_of_account, &[name, account]),
        None => name.to_owned(),
    };
    let mut note = vec![
        fill(labels.setting_source_line, &[source_text(labels, line.source)]),
        settings::change_words(labels, line.change).to_owned(),
    ];
    let settable =
        matches!(line.change, settings::Change::ViewOnly | settings::Change::OnMainPage);
    note.extend(settings::effect_words(labels, &line.name, settable).map(str::to_owned));
    SettingItem {
        name: name.into(),
        value: settings::value_words(labels, &line.name, &line.value).into(),
        note: note.join(" · ").into(),
    }
}

/// `line` into the log, the first time this run: a problem shows where the log is, a gap in
/// the words here does not.
fn tell_once(ui: &Ui, line: &str, problem: bool) {
    if !ui.model.borrow_mut().first_told(line) {
        return;
    }
    if problem {
        report::log(SETTINGS_LOG, line);
    } else {
        report::log_quiet(SETTINGS_LOG, line);
    }
}

fn source_text(labels: &Labels, source: SettingSource) -> &'static str {
    match source {
        SettingSource::File => labels.setting_source_file,
        SettingSource::Default => labels.setting_source_default,
        SettingSource::Discovered => labels.setting_source_discovered,
        SettingSource::Online => labels.setting_source_online,
    }
}

/// What the top bar's two write buttons offer, and what is said about them: the global
/// pause while one holds, the confirmation asked for, and what the last press came to.
/// And, on a line of its own, what the last change of settings from the top bar came to,
/// and on another what the last from the evidence days' field did.
pub fn actions(ui: &Ui) {
    let Some(window) = ui.workbench() else {
        return;
    };
    let labels = ui.labels();
    let view = ui.model.borrow().actions_view();
    window.set_pause_offered(view.can_pause);
    window.set_resume_offered(matches!(
        view.resume,
        ResumeEntry::OnePress(_) | ResumeEntry::TwoSteps(_)
    ));
    let statement = match &view.resume {
        ResumeEntry::Confirming { reason, .. } => {
            fill(labels.resume_all_statement, &[reason.as_str()])
        }
        ResumeEntry::Off | ResumeEntry::OnePress(_) | ResumeEntry::TwoSteps(_) => String::new(),
    };
    window.set_resume_all_statement(statement.into());
    window.set_pause_note(pause_note(labels, &view).into());
    let (line, failed) = match &view.scheduling_said {
        Some(said) => said_text(labels, &view, said),
        None => (String::new(), false),
    };
    window.set_action_line(line.into());
    window.set_action_failed(failed);
    let (line, failed) = settings_text(labels, &view, Field::TopBar);
    window.set_config_write_line(line.into());
    window.set_config_write_failed(failed);
    let (line, failed) = settings_text(labels, &view, Field::EvidenceDays);
    window.set_evidence_write_line(line.into());
    window.set_evidence_write_failed(failed);
    let (line, failed) = settings_text(labels, &view, Field::Backends);
    window.set_backend_write_line(line.into());
    window.set_backend_write_failed(failed);
}

/// What the last change of settings from `field` came to, and whether that is a failure;
/// nothing before one. A restart open across this and the first line is said once, on the
/// first line. The evidence days' line names its setting, and the backends' line the account
/// of the device it switched: the others are the top bar's. A switch says it switches while it
/// is out, that the device is switched once it is, and a refusal of a backend in a switch's
/// own words.
fn settings_text(labels: &Labels, view: &ActionsView, field: Field) -> (String, bool) {
    let said = match field {
        Field::TopBar => &view.settings_said,
        Field::EvidenceDays => &view.evidence_said,
        Field::Backends => &view.backends_said,
        // A rule's line is its panel's, never one of these, and that of every account's quiet
        // times is their editor's.
        Field::Rules(_) | Field::HostQuiet => return (String::new(), false),
    };
    let (text, failed) = if view.reread_failed == Some(field) {
        (labels.config_reread_failed.to_owned(), true)
    } else {
        match (said, &view.scheduling_said) {
            (Some(Said::Restarted), Some(Said::Restarted)) | (None, _) => {
                return (String::new(), false)
            }
            (Some(Said::NotWhole), _) if field == Field::EvidenceDays => {
                (labels.evidence_not_whole.to_owned(), true)
            }
            (Some(said), _) if field == Field::Backends => switch_text(labels, view, said),
            (Some(said), _) => said_text(labels, view, said),
        }
    };
    match field {
        Field::TopBar => (text, failed),
        Field::EvidenceDays => {
            (fill(labels.evidence_said, &[labels.setting_evidence_days, &text]), failed)
        }
        Field::Backends => {
            let account = view.backends_account.as_deref().unwrap_or(labels.start_account_gone);
            (fill(labels.backend_said, &[account, &text]), failed)
        }
        Field::Rules(_) | Field::HostQuiet => (String::new(), false),
    }
}

/// What a switch of a device's backends came to, where that differs from any change's words.
fn switch_text(labels: &Labels, view: &ActionsView, said: &Said) -> (String, bool) {
    match said {
        Said::Sending(_) => (settings::switching_words(labels), false),
        Said::ConfigApplied => (labels.backend_switched.to_owned(), false),
        // Whether the device switched or, the file's value being the one in effect, did not.
        Said::BackToFile => (labels.backend_back_to_file.to_owned(), false),
        Said::ConfigRefused(RefusalWhy::Value { problem: ValueProblem::OutOfRange, .. }) => {
            (labels.backend_out_of_range.to_owned(), true)
        }
        Said::ConfigRefused(RefusalWhy::NotSaved) => (labels.backend_not_saved.to_owned(), true),
        other => said_text(labels, view, other),
    }
}

/// While a global pause holds: what it means, how many tasks still finish when that
/// can be told (`ActionsView::finishing`), and who lifts it when not this window.
fn pause_note(labels: &Labels, view: &ActionsView) -> String {
    let Some(by) = view.holding else {
        return String::new();
    };
    let meaning = match view.finishing {
        None => labels.pause_note.to_owned(),
        Some(0) => labels.pause_note_all_finished.to_owned(),
        Some(finishing) => fill(labels.pause_note_finishing, &[&finishing.to_string()]),
    };
    // The installer's own pause is the update under way, and a Lab rescue's the operation
    // under way: telling the person an update waits for the pause to lift would be the
    // wrong way round.
    let mut note = vec![meaning.as_str()];
    match by {
        Lifter::Setter => note.extend([labels.pause_note_no_update, labels.resume_barred_setter]),
        Lifter::Installer => note.push(labels.resume_barred_installer),
        Lifter::LabRescue => note.push(labels.resume_barred_lab_rescue),
        Lifter::Window => note.push(labels.pause_note_no_update),
    }
    note.join("\n")
}

/// What a press came to in words, and whether that is a failure.
fn said_text(labels: &Labels, view: &ActionsView, said: &Said) -> (String, bool) {
    match said {
        Said::Sending(Write::PauseAll) => (labels.pause_all_sending.to_owned(), false),
        Said::Sending(Write::ResumeAll) => (labels.resume_all_sending.to_owned(), false),
        Said::Sending(Write::Configure { .. }) => (labels.config_sending.to_owned(), false),
        Said::PauseApplied => (labels.pause_all_applied.to_owned(), false),
        Said::AlreadyPaused => (labels.pause_all_already.to_owned(), false),
        Said::Resumed if view.still_paused == 0 => (labels.resume_all_done.to_owned(), false),
        Said::Resumed => {
            let mut text =
                fill(labels.resume_all_still_paused, &[&view.still_paused.to_string()]);
            if view.setter_holds {
                text.push_str(labels.resume_all_setter_lifts);
            }
            (text, false)
        }
        Said::PauseMoved => (labels.resume_all_moved.to_owned(), false),
        Said::Refused { write, code } => {
            let template = match write {
                Write::PauseAll => labels.pause_all_refused,
                Write::ResumeAll => labels.resume_all_refused,
                Write::Configure { .. } => labels.config_refused,
            };
            let words = outcome_short(labels, code)
                .unwrap_or_else(|| labels.outcome_unknown_short.to_owned());
            (fill(template, &[words.as_str()]), true)
        }
        Said::Unsupported => (labels.resume_all_unsupported.to_owned(), true),
        Said::Unconfirmed => (labels.write_unconfirmed.to_owned(), true),
        Said::NotSent => (labels.write_not_sent.to_owned(), true),
        Said::NoThread { error } => {
            let why = fill(labels.thread_failed, &[error.as_str()]);
            (fill(labels.write_no_thread, &[why.as_str()]), true)
        }
        Said::Restarted => (labels.write_restarted.to_owned(), false),
        Said::ConfigApplied => (labels.config_applied.to_owned(), false),
        // Only the backends' line says it (`switch_text`).
        Said::BackToFile => (labels.backend_back_to_file.to_owned(), false),
        Said::ConfigRefused(why) => (refusal_text(labels, why), true),
        Said::PresetGone => (labels.preset_gone.to_owned(), true),
        Said::Rereading(Reread::Conflict) => (labels.config_conflict_rereading.to_owned(), false),
        Said::Rereading(Reread::Unconfirmed { .. }) => {
            (labels.config_unconfirmed_rereading.to_owned(), true)
        }
        Said::ConflictRefreshed => (labels.config_conflict_refreshed.to_owned(), false),
        Said::NotApplied => (labels.config_not_applied.to_owned(), true),
        Said::UnconfirmedChanged => (labels.config_unconfirmed_changed.to_owned(), true),
        Said::UnconfirmedSeeNow => (labels.config_unconfirmed_see_now.to_owned(), true),
        Said::NotWhole => (labels.concurrency_not_whole.to_owned(), true),
        // Only the quiet times' line says it: only their "delete" comes to it.
        Said::QuietTooBig => (labels.rules_delete_too_big.to_owned(), true),
        Said::Rereading(Reread::Switch) => (labels.backend_unconfirmed.to_owned(), true),
        Said::BackendRefused(refusal) => (backend_refusal_text(labels, refusal), true),
        Said::SwitchOver { version, now } => {
            let check = now.as_ref().map_or_else(
                || labels.backend_check_none.to_owned(),
                |now| selfcheck_words(labels, &settings::selfcheck_of(now)),
            );
            let template = match version {
                SwitchVersion::Unchanged => labels.backend_not_applied,
                SwitchVersion::Changed => labels.backend_changed,
                SwitchVersion::Unknown => labels.backend_see_now,
            };
            (fill(template, &[&check]), true)
        }
    }
}

/// Why the Runtime switched nothing, in its own words for each problem and with what the
/// causes say; and, when the new backends opened and the previous ones did not come back after
/// the Runtime took none of it, that the device is unavailable.
fn backend_refusal_text(labels: &Labels, refusal: &BackendRefusal) -> String {
    let why = refusal.why.as_ref().map(|why| match why {
        BackendWhy::KindUnsupported => labels.backend_kind_unsupported.to_owned(),
        BackendWhy::PairRequired => labels.backend_pair_required.to_owned(),
        BackendWhy::NotAlone => labels.backend_not_alone.to_owned(),
        BackendWhy::Switch(refused) => switch_refused_text(labels, refused),
    });
    let not_restored = refusal
        .not_restored
        .as_ref()
        .map(|failure| fill(labels.backend_not_restored, &[&cause_text(labels, failure)]));
    match (why, not_restored) {
        (Some(why), Some(not_restored)) => format!("{why} {not_restored}"),
        (Some(text), None) | (None, Some(text)) => text,
        (None, None) => labels.outcome_unknown_short.to_owned(),
    }
}

fn switch_refused_text(labels: &Labels, refused: &SwitchRefusal) -> String {
    match refused {
        SwitchRefusal::Busy { holder, waited_ms } => {
            let holder = holder.as_ref().map_or(labels.backend_holder_other, |holder| {
                holder_words(labels, holder)
            });
            match waited_ms {
                Some(ms) => {
                    fill(labels.backend_busy, &[holder, &ms.div_ceil(1000).to_string()])
                }
                None => fill(labels.backend_busy_untimed, &[holder]),
            }
        }
        SwitchRefusal::PlayerHeld => labels.backend_player_held.to_owned(),
        SwitchRefusal::Fenced => labels.backend_fenced.to_owned(),
        SwitchRefusal::EndpointUnavailable => labels.backend_endpoint_unavailable.to_owned(),
        SwitchRefusal::OpenFailed(failure) => {
            fill(labels.backend_open_failed, &[&cause_or_unknown(labels, failure.as_ref())])
        }
        SwitchRefusal::RestoreFailed { new, restore } => {
            let new = cause_or_unknown(labels, new.as_ref());
            let restore = cause_or_unknown(labels, restore.as_ref());
            fill(labels.backend_restore_failed, &[&new, &restore])
        }
        // The log has its problem, with the receipt's whole outcome.
        SwitchRefusal::Other(_) => labels.backend_switch_other.to_owned(),
    }
}

/// What held the device a switch waited for; a claim this console has no words for is "other
/// work", which the log has by its word with the refusal.
fn holder_words(labels: &Labels, holder: &Holder) -> &'static str {
    match holder {
        Holder::PolicyDispatch => labels.backend_holder_policy_dispatch,
        Holder::Ladder | Holder::LadderContinuation => labels.backend_holder_ladder,
        Holder::StartupPackage => labels.backend_holder_startup_package,
        Holder::Autostart => labels.backend_holder_autostart,
        Holder::DirectTaskRun => labels.backend_holder_direct_task_run,
        Holder::EmulatorControl => labels.backend_holder_emulator_control,
        Holder::InputLifecycle => labels.backend_holder_input_lifecycle,
        Holder::ResumeReconnect => labels.backend_holder_resume_reconnect,
        Holder::SelfCheck => labels.backend_holder_self_check,
        Holder::BackendSwitch => labels.backend_holder_backend_switch,
        Holder::DaemonStartPreparation => labels.backend_holder_start_preparation,
        Holder::LabOperation => labels.backend_holder_lab_operation,
        Holder::ClientLease => labels.backend_holder_client_lease,
        Holder::ResourceClose => labels.backend_holder_resource_close,
        Holder::Other(_) => labels.backend_holder_other,
    }
}

fn cause_or_unknown(labels: &Labels, failure: Option<&OpenFailure>) -> String {
    match failure {
        Some(failure) => cause_text(labels, failure),
        None => labels.backend_cause_unknown.to_owned(),
    }
}

/// Why backends did not open: the step that failed, why a capture did not open when that is
/// said, and that MuMu's own pair found no MuMu app index when that is under it. A step that is
/// none of the self-check's own takes its code's category words, as any refusal's. Why the app
/// index was not found (#402 HS-3), when the Runtime says so in a word known here, says all: the
/// pair's step and its capture's cause, which then names none, add nothing to it.
fn cause_text(labels: &Labels, failure: &OpenFailure) -> String {
    let app_index = failure.app_index.as_ref().map(|problem| match problem {
        AppIndexProblem::PackageNotInstalled => Some(labels.backend_app_not_installed),
        AppIndexProblem::ApplicationNotRunning => Some(labels.backend_app_not_running),
        AppIndexProblem::InfoUnavailable => Some(labels.backend_app_info_unavailable),
        AppIndexProblem::InfoMalformed => Some(labels.backend_app_info_malformed),
        // No problem said, or one without words here: the log has it, with the receipt's whole
        // outcome.
        AppIndexProblem::Other(_) => None,
    });
    if let Some(Some(words)) = app_index {
        return words.to_owned();
    }
    let step = match &failure.step {
        FailedStep::TouchOpen => labels.backend_cause_touch_open.to_owned(),
        FailedStep::CaptureOpen => labels.backend_cause_capture_open.to_owned(),
        FailedStep::NemuOpen => labels.backend_cause_nemu_open.to_owned(),
        FailedStep::TouchCheck => labels.backend_cause_touch_check.to_owned(),
        FailedStep::CaptureCheck => labels.backend_cause_capture_check.to_owned(),
        FailedStep::Other(code) => {
            outcome_short(labels, code).unwrap_or_else(|| labels.outcome_unknown_short.to_owned())
        }
    };
    let with_why = match (&failure.step, failure.capture) {
        (FailedStep::TouchOpen | FailedStep::TouchCheck | FailedStep::CaptureCheck, _)
        | (_, None) => step,
        (FailedStep::CaptureOpen | FailedStep::NemuOpen | FailedStep::Other(_), Some(why)) => {
            let why = match why {
                CaptureCause::AppNotRunning => labels.backend_why_app_not_running,
                CaptureCause::Unreachable => labels.backend_why_unreachable,
                CaptureCause::Other => labels.backend_why_other,
            };
            fill(labels.backend_paren, &[&step, why])
        }
    };
    if app_index.is_some() {
        [with_why.as_str(), labels.backend_cause_app_index].join(labels.backend_cause_join)
    } else {
        with_why
    }
}

/// Why the Runtime took none of a change of settings. It names no key: only the log does.
fn refusal_text(labels: &Labels, why: &RefusalWhy) -> String {
    match why {
        RefusalWhy::Value { problem, range } => {
            let problem = match problem {
                ValueProblem::WrongType => labels.config_problem_wrong_type,
                ValueProblem::OutOfRange => labels.config_problem_out_of_range,
                ValueProblem::SectionCheck => labels.config_problem_section,
                ValueProblem::RepeatedKey => labels.config_problem_repeated,
                ValueProblem::DiscoveryBindingRequired => labels.config_problem_other,
                ValueProblem::Other => labels.config_problem_other,
            };
            let mut text = fill(labels.config_value_refused, &[problem]);
            if let Some((min, max)) = range {
                let (min, max) = (min.to_string(), max.to_string());
                text.push_str(&fill(labels.config_value_range, &[&min, &max]));
            }
            text
        }
        RefusalWhy::NotSettable => labels.config_key_not_settable.to_owned(),
        RefusalWhy::UnknownKey => labels.config_key_unknown.to_owned(),
        RefusalWhy::BadRequest => labels.config_request_invalid.to_owned(),
        RefusalWhy::NotSaved => labels.config_not_saved.to_owned(),
        RefusalWhy::Unsupported => labels.config_unsupported.to_owned(),
        // `Written::said` keeps these as `Said::Refused`, with their code's words.
        RefusalWhy::Other => labels.outcome_unknown_short.to_owned(),
    }
}

/// Which of the right column's two tabs shows, and under the second the account's daily task
/// list (UI-3f1), from its view: why there are no rows, or above them the account's pause or
/// fence in its card's words, what its switches do or why there are none (UI-3f2), which
/// evaluation they are of, that the names cannot be read and that not every task is listed; the
/// account's own line of tasks without a package, and its tasks while open; then each group's
/// title and its packages, each with its switch, name, badges, where its switch's state comes
/// from, what its last press came to, sentence, other sentences and summary. Under the live view
/// the list holds nothing. Each list changes only where its rows do.
pub fn tasks(ui: &Ui) {
    let Some(window) = ui.workbench() else {
        return;
    };
    let labels = ui.labels();
    let tab = ui.model.borrow().right_tab();
    let shown = match tab {
        RightTab::Picture => WorkbenchTab::Picture,
        RightTab::Tasks => WorkbenchTab::Tasks,
    };
    if window.get_right_tab() != shown {
        window.set_right_tab(shown);
    }
    let view = match tab {
        RightTab::Picture => None,
        RightTab::Tasks => Some(ui.model.borrow().tasks_view(Instant::now(), labels.tongue)),
    };
    let (data, rows, fold) = match &view {
        Some(view) => tasks_data(ui, labels, view),
        None => (TaskListData::default(), Vec::new(), Vec::new()),
    };
    sync(window.get_task_rows(), rows, |rows| window.set_task_rows(rows));
    sync(window.get_task_fold_lines(), fold, |rows| window.set_task_fold_lines(rows));
    if window.get_tasks() != data {
        window.set_tasks(data);
    }
}

/// The list's words above its rows, its rows, and its account's own tasks while their line is
/// open.
fn tasks_data(
    ui: &Ui,
    labels: &Labels,
    view: &TasksView,
) -> (TaskListData, Vec<TaskRowData>, Vec<TaskRowData>) {
    let instead = |text: String, failed: bool, warning: bool| TaskListData {
        instead: text.into(),
        instead_failed: failed,
        instead_warning: warning,
        ..TaskListData::default()
    };
    let data = match view {
        TasksView::Down(down) => {
            instead(fill(labels.tasks_down, &[down_text(labels, *down)]), false, false)
        }
        TasksView::NoAccount => instead(labels.picture_no_account.to_owned(), false, false),
        TasksView::Reading => instead(labels.tasks_reading.to_owned(), false, false),
        TasksView::Unreadable(Unreadable::Failed) => {
            instead(labels.tasks_unreadable.to_owned(), false, true)
        }
        TasksView::Unreadable(Unreadable::TooLarge) => {
            instead(labels.tasks_too_large.to_owned(), true, false)
        }
        TasksView::Unreadable(Unreadable::UnknownInstance) => {
            instead(labels.tasks_instance_unknown.to_owned(), false, true)
        }
        TasksView::NotEvaluated => instead(labels.tasks_not_evaluated.to_owned(), false, false),
        TasksView::NotInTable => instead(labels.tasks_not_in_table.to_owned(), false, false),
        TasksView::Listed(list) => return task_list(ui, labels, list),
    };
    (data, Vec::new(), Vec::new())
}

fn task_list(
    ui: &Ui,
    labels: &Labels,
    list: &TaskList,
) -> (TaskListData, Vec<TaskRowData>, Vec<TaskRowData>) {
    let held = list.held.as_ref().map_or_else(String::new, |line| held_text(ui, labels, line));
    let (switches, switches_warning) = match list.switches {
        SwitchesNote::Reading => ("", false),
        SwitchesNote::Read => (labels.tasks_switches_read, false),
        SwitchesNote::Unsupported => (labels.tasks_switches_unsupported, false),
        SwitchesNote::Unreadable => (labels.tasks_switches_unreadable, true),
    };
    let data = TaskListData {
        instead: SharedString::new(),
        instead_failed: false,
        instead_warning: false,
        held: held.into(),
        switches: switches.into(),
        switches_warning,
        evaluated: fill(labels.tasks_evaluated, &[&format_minute(list.evaluated_at)]).into(),
        names_unreadable: list.names_unreadable,
        cut_short: list.cut_short,
        fold_shown: !list.unmatched.is_empty(),
        fold_open: list.fold_open,
        // The Runtime's own reading task is no package's on any configuration.
        fold_note: list.unmatched.iter().any(|line| matches!(line.task, TaskName::Id(_))),
    };
    let mut rows = Vec::new();
    for group in &list.groups {
        let heading = match &group.title {
            GroupTitle::None => None,
            GroupTitle::Group { game, id } => {
                let model = ui.model.borrow();
                let title = model.group_title(game, id, labels.tongue).map(str::to_owned);
                Some(title.unwrap_or_else(|| id.clone()))
            }
            GroupTitle::Other => Some(labels.tasks_other_group.to_owned()),
        };
        if let Some(heading) = heading {
            rows.push(TaskRowData { heading: heading.into(), ..TaskRowData::default() });
        }
        rows.extend(group.rows.iter().map(|row| pack_row(ui, labels, row)));
    }
    let fold = if list.fold_open {
        list.unmatched.iter().map(|line| fold_line(ui, labels, line)).collect()
    } else {
        Vec::new()
    };
    (data, rows, fold)
}

/// The account's pause or fence above the list's rows, in its card's words but for three pauses
/// whose words would be wrong there: this console's own pause of the account is lifted on its
/// card, not below the list; and another tool's, of the account or of all, goes without the reason
/// it wrote, which may name a person.
fn held_text(ui: &Ui, labels: &Labels, line: &Line) -> String {
    let Line::Paused { everyone, by: Lifter::Window, reason } = line else {
        return line_text(ui, labels, line);
    };
    let said = match (*everyone, reason.as_str()) {
        (true, CONSOLE_PAUSE_ALL) => return line_text(ui, labels, line),
        (true, _) => labels.tasks_paused_other_all,
        (false, CONSOLE_PAUSE_ACCOUNT) => labels.tasks_paused_account,
        (false, _) => labels.tasks_paused_other,
    };
    if *everyone {
        fill(labels.card_all_accounts, &[said])
    } else {
        said.to_owned()
    }
}

/// One package: its switch, its name, its badges, where its switch's state comes from or that it
/// has none here, what the last press of it came to, why it runs or not now, the other reasons of
/// its tasks, and its summary, or that it has none.
fn pack_row(ui: &Ui, labels: &Labels, row: &PackRow) -> TaskRowData {
    let (said, tone) = task_said_text(ui, labels, &row.said);
    let (switch_shown, switch_on, switch_enabled, switch_note, switch_said) = match &row.switch {
        SwitchSlot::Hidden => (false, false, false, String::new(), None),
        SwitchSlot::NotHere { listed } => {
            not_here_told(ui, &row.package_id, *listed);
            (false, false, false, labels.tasks_switch_not_here.to_owned(), None)
        }
        SwitchSlot::Offered(PackSwitch { state, enabled, said }) => {
            let note = pack_source_text(labels, state);
            let said = said.as_ref().map(|said| switch_said_text(ui, labels, said));
            (true, state.on, *enabled, note, said)
        }
    };
    let (switch_said, switch_failed, switch_warning) = match switch_said {
        Some((text, SaidTone::Plain)) => (text, false, false),
        Some((text, SaidTone::Warning)) => (text, false, true),
        Some((text, SaidTone::Failed)) => (text, true, false),
        None => (String::new(), false, false),
    };
    // Each other sentence without its wake, and more of them said to be more, never how many.
    let also = if row.also.is_empty() {
        String::new()
    } else {
        let mut others: Vec<String> =
            row.also.iter().map(|other| task_said_text(ui, labels, other).0).collect();
        if row.more {
            if let Some(last) = others.last_mut() {
                last.push_str(labels.tasks_also_more);
            }
        }
        fill(labels.tasks_also, &[&others.join(labels.tasks_also_join)])
    };
    let summary = {
        let model = ui.model.borrow();
        model.pack_summary(&row.package_id, labels.tongue).map(str::to_owned)
    };
    TaskRowData {
        heading: SharedString::new(),
        name: package_name(ui, labels, &row.package_id).into(),
        irreversible: row.irreversible,
        risky: row.risky,
        said: said.into(),
        tone,
        also: also.into(),
        summary: summary.unwrap_or_else(|| labels.tasks_no_summary.to_owned()).into(),
        package: row.package_id.as_str().into(),
        switch_shown,
        switch_on,
        switch_enabled,
        switch_note: switch_note.into(),
        switch_said: switch_said.into(),
        switch_failed,
        switch_warning,
    }
}

/// A package the list shows without a switch, to the log the first time only: the view does not
/// name it (`listed` false), or decides it by the account's own rows without naming what is below
/// them, which the contract never leaves out.
fn not_here_told(ui: &Ui, package: &str, listed: bool) {
    let alias = ui.model.borrow().picture_account().unwrap_or_default().to_owned();
    let why = if listed {
        "decides it by the account's own rows and names nothing below them"
    } else {
        "does not name it"
    };
    let line = format!("task switches of {alias}: the view {why}; package {package} has no switch");
    if ui.model.borrow_mut().first_told(&line) {
        report::log(TASKS_LOG, &line);
    }
}

/// Where a package's switch's state comes from, in grey under its name, with the times of the row
/// that decided: the account's own rows, its server's, its game's or the pack's default, on or off,
/// and until when when that row ends; with no row, when that changes, or nothing at all, which the
/// list's note says (Workflow #429 plan, 177; the console's own setting off since when it began).
fn pack_source_text(labels: &Labels, state: &PackState) -> String {
    let word = if state.on { labels.tasks_source_on } else { labels.tasks_source_off };
    let layer = match state.source {
        PackSource::Instance if !state.on => {
            let off = fill(labels.tasks_source_own, &[labels.tasks_source_own_off]);
            return match state.row {
                Some(row) => match row.until_ms {
                    Some(until) => fill(labels.tasks_source_until, &[&off, &format_minute(until)]),
                    None => fill(labels.tasks_source_since, &[&off, &format_minute(row.from_ms)]),
                },
                None => off,
            };
        }
        PackSource::Instance => labels.tasks_source_own,
        PackSource::Server => labels.tasks_source_server,
        PackSource::Game => labels.tasks_source_game,
        PackSource::PackDefault => labels.tasks_source_pack_default,
        PackSource::None => {
            return state.next_change_ms.map_or_else(String::new, |at| {
                fill(labels.tasks_source_changes, &[&format_minute(at)])
            });
        }
    };
    let said = fill(layer, &[word]);
    match state.row.and_then(|row| row.until_ms) {
        Some(until) => fill(labels.tasks_source_until, &[&said, &format_minute(until)]),
        None => said,
    }
}

/// The colour of what a press came to: as the grey lines are, amber while the person is the one to
/// look again, red for a failure.
enum SaidTone {
    Plain,
    Warning,
    Failed,
}

/// What the last press of a package's switch came to, in words, with when it landed: never a code,
/// a version or a count, which the log has.
fn switch_said_text(ui: &Ui, labels: &Labels, said: &SwitchSaid) -> (String, SaidTone) {
    let at = |template: &str, at_ms: u64| fill(template, &[&format_second(at_ms)]);
    match said {
        SwitchSaid::Sending => (labels.tasks_switch_sending.to_owned(), SaidTone::Plain),
        SwitchSaid::Conflict => (labels.tasks_switch_conflict.to_owned(), SaidTone::Warning),
        SwitchSaid::NotNewer { at_ms } => {
            (at(labels.tasks_switch_not_newer, *at_ms), SaidTone::Failed)
        }
        SwitchSaid::TooMany { at_ms } => {
            (at(labels.tasks_switch_too_many, *at_ms), SaidTone::Failed)
        }
        SwitchSaid::StoredUnreadable { at_ms } => {
            (at(labels.tasks_switch_stored_unreadable, *at_ms), SaidTone::Failed)
        }
        SwitchSaid::Invalid { at_ms } => (at(labels.tasks_switch_invalid, *at_ms), SaidTone::Failed),
        SwitchSaid::Refused { at_ms, codes } => {
            let codes: Vec<&str> = codes.iter().map(String::as_str).collect();
            let words = switch_words(ui, labels, &codes);
            let text = fill(labels.tasks_switch_refused, &[&words, &format_second(*at_ms)]);
            (text, SaidTone::Failed)
        }
        SwitchSaid::NotSent { at_ms } => {
            (at(labels.tasks_switch_not_sent, *at_ms), SaidTone::Failed)
        }
        SwitchSaid::Unconfirmed { at_ms } => {
            (at(labels.tasks_switch_unconfirmed, *at_ms), SaidTone::Failed)
        }
        SwitchSaid::NoThread { error } => {
            let why = fill(labels.thread_failed, &[error.as_str()]);
            (fill(labels.tasks_switch_no_thread, &[why.as_str()]), SaidTone::Failed)
        }
        SwitchSaid::Restarted => (labels.tasks_switch_restarted.to_owned(), SaidTone::Plain),
    }
}

/// A switch's refusal in plain words, never its code (`codes`, the host's before the Runtime's),
/// as a stop's (`stop_words`); else that it is not known here, which the log hears of once.
fn switch_words(ui: &Ui, labels: &Labels, codes: &[&str]) -> String {
    if let Some(words) = code_words(labels, codes) {
        return words;
    }
    let line = format!("no words for a task switch refused with codes {codes:?}");
    if ui.model.borrow_mut().first_told(&line) {
        report::log(TASKS_LOG, &line);
    }
    labels.rescue_cause_unknown.to_owned()
}

/// One task under the account itself: by its id, the Runtime's reading task by words.
fn fold_line(ui: &Ui, labels: &Labels, line: &TaskLine) -> TaskRowData {
    let name = match &line.task {
        TaskName::Id(task) => task.clone(),
        TaskName::FactRefresh => labels.tasks_fact_refresh.to_owned(),
    };
    let (said, tone) = task_said_text(ui, labels, &line.said);
    TaskRowData { name: name.into(), said: said.into(), tone, ..TaskRowData::default() }
}

/// A row's sentence and its colour: the accent while its package runs or is scheduled to, amber
/// while a person is the one to act. A pause is said above the rows, so a reason of one only
/// says that nothing starts.
fn task_said_text(ui: &Ui, labels: &Labels, said: &TaskSaid) -> (String, TaskTone) {
    match said {
        TaskSaid::Running => (labels.tasks_running.to_owned(), TaskTone::Accent),
        TaskSaid::RunningDirect => (labels.tasks_running_direct.to_owned(), TaskTone::Accent),
        TaskSaid::Scheduled { paused: false } => {
            (labels.tasks_scheduled.to_owned(), TaskTone::Accent)
        }
        TaskSaid::Scheduled { paused: true } => {
            (labels.tasks_scheduled_paused.to_owned(), TaskTone::Plain)
        }
        TaskSaid::Completed => (labels.tasks_completed.to_owned(), TaskTone::Plain),
        TaskSaid::Rejected => (labels.tasks_rejected.to_owned(), TaskTone::Plain),
        TaskSaid::NotDecided => (labels.tasks_not_decided.to_owned(), TaskTone::Plain),
        TaskSaid::Reason { class, why, wake } => {
            let words = match class {
                ReasonClass::ControlGate => labels.tasks_paused.to_owned(),
                _ => reason_words(ui, labels, *class, why),
            };
            let text = match wake {
                Some(wake) => fill(labels.card_with_wake, &[&words, &format_minute(*wake)]),
                None => words,
            };
            let tone = match class.band() {
                Band::Attention => TaskTone::Warning,
                Band::Ok | Band::Offline => TaskTone::Plain,
            };
            (text, tone)
        }
    }
}

/// Puts `items` into the list the window already shows, a row at a time; only the
/// list it starts with is replaced.
fn sync<T: Clone + PartialEq + 'static>(
    current: ModelRc<T>,
    items: Vec<T>,
    replace: impl FnOnce(ModelRc<T>),
) {
    let Some(rows) = current.as_any().downcast_ref::<VecModel<T>>() else {
        replace(models(items));
        return;
    };
    while rows.row_count() > items.len() {
        rows.remove(rows.row_count() - 1);
    }
    for (index, item) in items.into_iter().enumerate() {
        if index == rows.row_count() {
            rows.push(item);
        } else if rows.row_data(index).as_ref() != Some(&item) {
            rows.set_row_data(index, item);
        }
    }
}

fn account_card(ui: &Ui, labels: &Labels, view: &CardView) -> AccountCard {
    let notes: Vec<String> = view.notes.iter().map(|line| line_text(ui, labels, line)).collect();
    // A server remembered from an earlier snapshot says so after it, in the same small print.
    let server = view.server.as_deref().map(|server| {
        if view.server_last_seen {
            fill(labels.card_server_last_seen, &[server])
        } else {
            server.to_owned()
        }
    });
    let game_line: Vec<&str> =
        [view.game.as_deref(), server.as_deref()].into_iter().flatten().collect();
    let (pause_shown, resume_shown, entry_offered) = match &view.account.entry {
        AccountEntry::Off => (false, false, false),
        AccountEntry::Pause { offered } => (true, false, *offered),
        AccountEntry::Resume { offered, .. } => (false, true, *offered),
    };
    let (said, said_failed) = match &view.account.said {
        Some(said) => account_said_text(labels, said, view.account.global_holds),
        None => (String::new(), false),
    };
    AccountCard {
        alias: view.alias.as_str().into(),
        game_line: game_line.join(" · ").into(),
        tone: match view.band {
            Band::Ok => StatusTone::Ok,
            Band::Attention => StatusTone::Attention,
            Band::Offline => StatusTone::Offline,
        },
        main: line_text(ui, labels, &view.main).into(),
        main_failed: matches!(view.main, Line::ScheduleUnreadable),
        note: notes.join(" · ").into(),
        selected: view.selected,
        pause_shown,
        resume_shown,
        entry_offered,
        said: said.into(),
        said_failed,
    }
}

/// What the last press of an account card's own button came to in words, and whether that is
/// a failure. A refusal says its code as the Runtime gave it, then the code's short words, and
/// so does a lift whose reconnect of the device failed. A lift while a global pause holds
/// (`global_holds`) says that the account is still not scheduled, as the top bar's says the
/// pauses that still hold.
fn account_said_text(labels: &Labels, said: &AccountSaid, global_holds: bool) -> (String, bool) {
    let coded = |template: &str, code: &str| {
        let words = outcome_short(labels, code)
            .unwrap_or_else(|| labels.outcome_unknown_short.to_owned());
        fill(template, &[code, words.as_str()])
    };
    match said {
        AccountSaid::Sending(AccountWrite::Pause) => {
            (labels.account_pause_sending.to_owned(), false)
        }
        AccountSaid::Sending(AccountWrite::Resume) => {
            (labels.account_resume_sending.to_owned(), false)
        }
        AccountSaid::Paused => (labels.account_paused.to_owned(), false),
        AccountSaid::AlreadyPaused => (labels.account_pause_already.to_owned(), false),
        AccountSaid::Resumed { device_failure } => {
            let mut text = match device_failure {
                None => labels.account_resumed.to_owned(),
                Some(code) => coded(labels.account_resumed_device_failed, code.as_str()),
            };
            if global_holds {
                text.push_str(labels.account_resumed_global_holds);
            }
            (text, device_failure.is_some())
        }
        AccountSaid::PauseMoved => (labels.resume_all_moved.to_owned(), false),
        AccountSaid::Refused { write: AccountWrite::Pause, code } => {
            (coded(labels.account_pause_refused, code.as_str()), true)
        }
        AccountSaid::Refused { write: AccountWrite::Resume, code } => {
            (coded(labels.account_resume_refused, code.as_str()), true)
        }
        AccountSaid::Unsupported => (labels.resume_all_unsupported.to_owned(), true),
        AccountSaid::Unconfirmed => (labels.write_unconfirmed.to_owned(), true),
        AccountSaid::NotSent => (labels.write_not_sent.to_owned(), true),
        AccountSaid::NoThread { error } => {
            let why = fill(labels.thread_failed, &[error.as_str()]);
            (fill(labels.write_no_thread, &[why.as_str()]), true)
        }
        AccountSaid::Restarted => (labels.write_restarted.to_owned(), false),
    }
}

fn line_text(ui: &Ui, labels: &Labels, line: &Line) -> String {
    match line {
        Line::Down(down) => down_text(labels, *down).to_owned(),
        Line::Draining { package } => {
            let task = match package {
                Some(package) => package_name(ui, labels, package),
                None => labels.card_draining_any.to_owned(),
            };
            fill(labels.card_draining, &[&task])
        }
        Line::HandingBack => labels.card_handing_back.to_owned(),
        Line::Paused { everyone, by, reason } => {
            let said = match by {
                Lifter::Setter => labels.card_paused_setter.to_owned(),
                Lifter::Installer => labels.card_paused_installer.to_owned(),
                Lifter::LabRescue => labels.card_paused_lab_rescue.to_owned(),
                Lifter::Window if *everyone && reason.as_str() == CONSOLE_PAUSE_ALL => {
                    labels.card_paused_console.to_owned()
                }
                Lifter::Window if *everyone => {
                    fill(labels.card_paused_other_all, &[reason.as_str()])
                }
                // This console's own pause of the account, which its card lifts.
                Lifter::Window if reason.as_str() == CONSOLE_PAUSE_ACCOUNT => {
                    labels.card_paused_account.to_owned()
                }
                Lifter::Window => fill(labels.card_paused_other, &[reason.as_str()]),
            };
            if *everyone {
                fill(labels.card_all_accounts, &[&said])
            } else {
                said
            }
        }
        Line::Running { package } => fill(labels.card_running, &[&task_name(ui, labels, package)]),
        Line::Finishing { package } => {
            fill(labels.card_finishing, &[&task_name(ui, labels, package)])
        }
        Line::HostRun => labels.card_host_run.to_owned(),
        Line::Reconnecting => labels.card_reconnecting.to_owned(),
        Line::SelfChecking => labels.card_self_checking.to_owned(),
        Line::ControllingEmulator => labels.card_controlling_emulator.to_owned(),
        Line::Climbing => labels.card_climbing.to_owned(),
        Line::AwaitingPerson { cause } => {
            fill(labels.card_awaiting_person, &[&cause_words(ui, labels, cause)])
        }
        Line::Recovering => labels.card_recovering.to_owned(),
        Line::LadderHold => labels.card_ladder_hold.to_owned(),
        Line::ScheduleUnreadable => labels.card_schedule_unreadable.to_owned(),
        Line::Preparing => labels.card_preparing.to_owned(),
        Line::Reason { class, why, wake } => {
            let words = reason_words(ui, labels, *class, why);
            match wake {
                Some(wake) => fill(labels.card_with_wake, &[&words, &format_minute(*wake)]),
                None => words,
            }
        }
        Line::NotEvaluated => labels.card_not_evaluated.to_owned(),
        Line::NotPlanned => labels.card_not_planned.to_owned(),
    }
}

/// The same words the lamp has for it.
fn down_text(labels: &Labels, down: Down) -> &'static str {
    match down {
        Down::NotYet => labels.launcher_checking,
        Down::NotRunning => labels.launcher_not_running,
        Down::NoAnswer => labels.link_no_answer,
        Down::Trouble => labels.link_trouble,
        Down::NoThread => labels.launcher_check_failed,
    }
}

fn task_name(ui: &Ui, labels: &Labels, package: &Option<String>) -> String {
    match package {
        Some(package) => package_name(ui, labels, package),
        None => labels.card_task_unnamed.to_owned(),
    }
}

/// A package's task name in the language shown, or its id when there is none.
fn package_name(ui: &Ui, labels: &Labels, package: &str) -> String {
    ui.model.borrow().task_title(package, labels.tongue).unwrap_or(package).to_owned()
}

/// A reason in plain words, a card's and the daily task list's alike. One with no sentence of
/// its own still says something, that it is not scheduled now, and the code goes to the log
/// once; so does one the Runtime gave no reason, which says so.
fn reason_words(ui: &Ui, labels: &Labels, class: ReasonClass, why: &str) -> String {
    let words = match class {
        ReasonClass::ControlGate => labels.card_schedule_updating,
        ReasonClass::Held => labels.card_why_held,
        ReasonClass::Unavailable => labels.card_why_unavailable,
        ReasonClass::DeviceBusy => labels.card_why_device_busy,
        ReasonClass::Quiet => labels.card_why_quiet,
        ReasonClass::QuietSoon => labels.card_why_quiet_soon,
        ReasonClass::UpdateWait => labels.card_why_update_wait,
        ReasonClass::PackProblem => labels.card_why_pack_problem,
        ReasonClass::Closed => labels.card_why_closed,
        ReasonClass::Paced => labels.card_why_paced,
        ReasonClass::Queue => labels.card_why_queue,
        ReasonClass::OtherFirst => labels.card_why_other_first,
        ReasonClass::ChainWait => labels.card_why_chain_wait,
        ReasonClass::RemedyWait => labels.card_why_remedy_wait,
        ReasonClass::NeedsChoice => labels.card_why_needs_choice,
        ReasonClass::Suspended => labels.card_why_suspended,
        ReasonClass::BenefitUnknown => labels.card_why_benefit_unknown,
        ReasonClass::NoEffects => labels.card_why_no_effects,
        ReasonClass::EntryPending => labels.card_why_entry_pending,
        ReasonClass::Unvalued => labels.card_why_unvalued,
        ReasonClass::ReadingUnknown => labels.card_why_reading_unknown,
        ReasonClass::BenefitNonpositive => labels.card_why_benefit_nonpositive,
        ReasonClass::NoProgress => labels.card_why_no_progress,
        ReasonClass::NoTask => labels.card_why_no_task,
        ReasonClass::NotMet => labels.card_why_not_met,
        ReasonClass::NotTime => labels.card_why_not_time,
        ReasonClass::Cooling => labels.card_why_cooling,
        ReasonClass::RetryLater => labels.card_why_retry_later,
        ReasonClass::Missed => labels.card_why_missed,
        ReasonClass::Served => labels.card_why_served,
        ReasonClass::Enough => labels.card_why_enough,
        ReasonClass::FactsMissing => labels.card_why_facts_missing,
        ReasonClass::Refreshing => labels.card_why_refreshing,
        ReasonClass::RefreshUnclear => labels.card_why_refresh_unclear,
        ReasonClass::RefreshStuck => labels.card_why_refresh_stuck,
        ReasonClass::RefreshUnbound => labels.card_why_refresh_unbound,
        ReasonClass::Locked => labels.card_why_locked,
        ReasonClass::OutOfHours => labels.card_why_out_of_hours,
        ReasonClass::NotCapable => labels.card_why_not_capable,
        ReasonClass::TurnedOff => labels.card_why_turned_off,
        ReasonClass::NoProfile => labels.card_why_no_profile,
        ReasonClass::HostShort => labels.card_why_host_short,
        ReasonClass::NextRound => labels.card_why_next_round,
        ReasonClass::GateUnmet => labels.card_why_gate_unmet,
        ReasonClass::Reserve => labels.card_why_reserve,
        ReasonClass::HardLine => labels.card_why_hard_line,
        ReasonClass::BetterFirst => labels.card_why_better_first,
        ReasonClass::LinesInvalid => labels.card_why_lines_invalid,
        ReasonClass::LinesUnspendable => labels.card_why_lines_unspendable,
        ReasonClass::DependencyWait => labels.card_why_dependency_wait,
        ReasonClass::WithinTolerance => labels.card_why_within_tolerance,
        ReasonClass::Budget => labels.card_why_budget,
        ReasonClass::ReasonMissing => {
            let line = format!("the Runtime gave a decision no reason: {why}");
            if ui.model.borrow_mut().first_told(&line) {
                report::log("acui-cards", &line);
            }
            labels.card_why_reason_missing
        }
        ReasonClass::Unlisted => {
            unlisted_told(ui, why);
            labels.card_why_unlisted
        }
    };
    words.to_owned()
}

/// Every reason of a task list's answer with no sentence of its own goes to the log once, whether
/// a row says it or not: one another's sentence stands for, one past the most a row says, one of
/// a package that runs or is scheduled, one under the account's own line while that is closed.
pub fn tasks_unlisted(ui: &Ui, read: &TasksRead) {
    let Some(entry) = &read.entry else {
        return;
    };
    for row in &entry.rows {
        let (gap, hold) = (row.gap.as_deref(), row.hold.as_deref());
        if ReasonClass::of(&row.why, gap, hold) == ReasonClass::Unlisted {
            unlisted_told(ui, &reason_logged(&row.why, gap, hold));
        }
    }
}

/// A reason code with no sentence, as `reason_logged` gives it, to the log the first time only.
fn unlisted_told(ui: &Ui, why: &str) {
    let line = format!("no sentence for not-running reason {why}");
    if ui.model.borrow_mut().first_told(&line) {
        report::log_quiet("acui-cards", &line);
    }
}

/// A fence's cause in plain words. The card never shows the code: one the tables do
/// not know gets a general sentence, and goes to the log once.
fn cause_words(ui: &Ui, labels: &Labels, cause: &str) -> String {
    match explain(cause, labels.tongue.outcome_tongue()) {
        Ok(Some(words)) => words.short.to_owned(),
        Ok(None) => {
            let line = format!("no words for fence cause {cause}");
            if ui.model.borrow_mut().first_told(&line) {
                report::log_quiet("acui-cards", &line);
            }
            labels.card_cause_unknown.to_owned()
        }
        Err(error) => fill(labels.outcome_table_unusable, &[&error.to_string()]),
    }
}
