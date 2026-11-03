// SPDX-License-Identifier: GPL-3.0-only
//! The account cards, from their views and the table of the language shown, into
//! the list the workbench already shows: a row changes only when its words do, and
//! rows come and go only when the accounts do, so a read every few seconds costs
//! nothing and moves nothing. And the top bar's writes, its performance preset and the
//! stepper under it, from their views; which page shows; the settings page's rows of
//! the Runtime's settings, the same way as the cards, with the field of the evidence days
//! among them and each device's group of its backends after them; what the start report says
//! of the settings; and whether the Runtime is started again when it is gone. And the right
//! column's live view, from its view, the frame's pixels once when it lands; and under it the
//! picture's refresh, painted with it.

use std::time::{Instant, SystemTime, UNIX_EPOCH};

use acui_rows::outcome::explain;
use acui_rows::{format_minute, format_second};
use acui_source::workbench::backends::{
    unnamed_backend_words, AppIndexProblem, BackendRefusal, BackendRow, BackendWhy, CaptureCause,
    CheckStatus, FailedStep, Holder, InEffect, InstanceBackends, OpenFailure, SwitchRefusal,
    SwitchVersion,
};
use acui_source::workbench::preview::Overlay;
use acui_source::workbench::{
    PresetMember, PresetSetting, RefusalWhy, ReportedName, SettingName, SettingSource,
    SettingValue, ValueProblem, CONSOLE_PAUSE_ALL, MODE_FULL, MODE_OBSERVE, MODE_PACE,
};
use slint::{Image, Model as _, ModelRc, Rgba8Pixel, SharedPixelBuffer, SharedString, VecModel};

use super::actions::{
    ActionsView, Choice, EvidenceView, Field, GroupState, PresetState, PresetView, Reread,
    ResumeEntry, RowEdit, Said, StepperView, Write, PRESET_ADAPTIVE, PRESET_LIGHT,
    PRESET_STANDARD,
};
use super::card::{Band, CardView, Down, Line, ReasonClass};
use super::model::{Fault, Page};
use super::picture::{
    DimWhy, Doing, EmptyWhy, Offline, Origin, OverlayView, PictureView, RefreshHold, RefreshWhyNot,
    Refreshed, Unread,
};
use super::preview;
use super::settings::{
    self, AutostartView, BackendGroup, CheckEntry, ReportedAccount, ReportedLine, SelfCheckView,
    SettingLine, SettingsView, StartView,
};
use crate::ledger::paint::models;
use crate::ledger::resume::Lifter;
use crate::report;
use crate::strings::{fill, outcome_short, Labels};
use crate::ui::Ui;
use crate::{
    AccountCard, BackendGroupItem, OverlayItem, PresetChoice, SettingItem, StatusTone, TargetItem,
    WorkbenchPage, WorkbenchWindow,
};

/// What the technical log's lines about the settings page are under.
const SETTINGS_LOG: &str = "acui-settings";
/// And those about the live view, and its refresh.
const PICTURE_LOG: &str = preview::THREAD;
const REFRESH_LOG: &str = super::refresh::THREAD;

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
/// picture (`refresh`).
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
/// What a refresh costs is the window's own, in grey.
pub fn refresh(ui: &Ui) {
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

/// The same words the card has for it.
fn doing_text(labels: &Labels, doing: Doing) -> &'static str {
    match doing {
        Doing::Reconnecting => labels.card_reconnecting,
        Doing::SelfChecking => labels.card_self_checking,
        Doing::ControllingEmulator => labels.card_controlling_emulator,
        Doing::Climbing => labels.card_climbing,
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

/// The settings page's rows of the Runtime's settings, in their two groups, with the field of
/// the evidence days in its place among the advanced ones (`evidence_field`), and what is said
/// instead of them, or besides. A setting without a name here and a snapshot that reports no
/// configuration each go to the log once; rows that do not read, each by its key and why,
/// once for each owner epoch.
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
        SettingsView::Failed => (labels.settings_table_unreadable.to_owned(), true),
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
    window.set_runtime_settings_note(note.into());
    window.set_runtime_settings_failed(failed);
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
        | GroupState::OtherWrite => (
            fill(labels.backend_selfcheck, &[&selfcheck_words(labels, &selfcheck_view)]),
            selfcheck_view.entries.iter().any(|(_, status)| *status == CheckStatus::Failed),
        ),
    };
    let (status, status_failed) = match edit.state {
        GroupState::Open => ("", false),
        GroupState::KindUnread => (labels.backend_kind_unread, true),
        GroupState::RowUnread => (labels.backend_row_unread, true),
        GroupState::Switching => (labels.backend_switching, false),
        GroupState::Unconfirmed { over: false } => (labels.backend_unconfirmed, true),
        GroupState::Unconfirmed { over: true } | GroupState::Rereading => {
            (labels.backend_rereading, false)
        }
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
    let effect = settings::effect_words(labels, &SettingName::EvidenceMaxAgeDays);
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
    note.extend(settings::effect_words(labels, &line.name).map(str::to_owned));
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
    }
}

/// What a switch of a device's backends came to, where that differs from any change's words.
fn switch_text(labels: &Labels, view: &ActionsView, said: &Said) -> (String, bool) {
    match said {
        Said::Sending(_) => (labels.backend_switching.to_owned(), false),
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
    // The installer's own pause is the update under way: telling the person an update
    // waits for the pause to lift would be the wrong way round.
    let mut note = vec![meaning.as_str()];
    match by {
        Lifter::Alice => note.extend([labels.pause_note_no_update, labels.resume_barred_alice]),
        Lifter::Installer => note.push(labels.resume_barred_installer),
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
            if view.alice_holds {
                text.push_str(labels.resume_all_alice_decides);
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
                Lifter::Alice => labels.card_paused_alice.to_owned(),
                Lifter::Installer => labels.card_paused_installer.to_owned(),
                Lifter::Window if *everyone && reason.as_str() == CONSOLE_PAUSE_ALL => {
                    labels.card_paused_console.to_owned()
                }
                Lifter::Window if *everyone => {
                    fill(labels.card_paused_other_all, &[reason.as_str()])
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

/// A reason in plain words. One the card has no sentence for still says something:
/// its category's words, or the unknown code's, and the code goes to the log once.
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
        ReasonClass::RemedyWait => labels.card_why_remedy_wait,
        ReasonClass::NeedsChoice => labels.card_why_needs_choice,
        ReasonClass::Suspended => labels.card_why_suspended,
        ReasonClass::BenefitUnknown => labels.card_why_benefit_unknown,
        ReasonClass::BenefitNonpositive => labels.card_why_benefit_nonpositive,
        ReasonClass::NoProgress => labels.card_why_no_progress,
        ReasonClass::NoTask => labels.card_why_no_task,
        ReasonClass::Unlisted => {
            let line = format!("no sentence for not-running reason {why}");
            if ui.model.borrow_mut().first_told(&line) {
                report::log_quiet("acui-cards", &line);
            }
            return outcome_short(labels, why)
                .unwrap_or_else(|| labels.outcome_unknown_short.to_owned());
        }
    };
    words.to_owned()
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
