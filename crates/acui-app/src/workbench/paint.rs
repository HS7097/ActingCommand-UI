// SPDX-License-Identifier: GPL-3.0-only
//! The account cards, from their views and the table of the language shown, into
//! the list the workbench already shows: a row changes only when its words do, and
//! rows come and go only when the accounts do, so a read every few seconds costs
//! nothing and moves nothing. And the top bar's writes, its performance preset and the
//! stepper beside it, from their views.

use std::time::Instant;

use acui_rows::format_minute;
use acui_rows::outcome::explain;
use acui_source::workbench::{
    PresetMember, PresetSetting, RefusalWhy, SettingSource, SettingValue, ValueProblem,
    CONSOLE_PAUSE_ALL, MODE_FULL, MODE_OBSERVE, MODE_PACE,
};
use slint::{Model as _, ModelRc, VecModel};

use super::actions::{
    ActionsView, PresetState, PresetView, Reread, ResumeEntry, Said, StepperView, Write,
    PRESET_ADAPTIVE, PRESET_LIGHT, PRESET_STANDARD,
};
use super::card::{Band, CardView, Down, Line, ReasonClass};
use crate::ledger::paint::models;
use crate::ledger::resume::Lifter;
use crate::report;
use crate::strings::{fill, outcome_short, Labels};
use crate::ui::Ui;
use crate::{AccountCard, PresetChoice, StatusTone};

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

/// The top bar's second row: the presets, the one in effect marked, what is in effect in
/// words, and whether they can be pressed; nothing while there is no Runtime to say it of.
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
    let name = match member.setting {
        PresetSetting::Mode => labels.setting_mode,
        PresetSetting::MaxConcurrentRuns => labels.setting_concurrent_runs,
        PresetSetting::HeavyDispatchLimit => labels.setting_heavy_runs,
        PresetSetting::ParallelEnabled => labels.setting_parallel,
    };
    let value = match &member.value {
        None => labels.setting_unset.to_owned(),
        Some(value) => value_text(labels, value),
    };
    fill(labels.preset_member, &[name, &value, source_text(labels, member.source)])
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
    let (line, failed) = said_text(labels, &view);
    window.set_action_line(line.into());
    window.set_action_failed(failed);
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
    let mut note = vec![meaning.as_str(), labels.pause_note_no_update];
    match by {
        Lifter::Alice => note.push(labels.resume_barred_alice),
        Lifter::Installer => note.push(labels.resume_barred_installer),
        Lifter::Window => {}
    }
    note.join("\n")
}

/// What the last press came to, and whether that is a failure; nothing before one.
fn said_text(labels: &Labels, view: &ActionsView) -> (String, bool) {
    if view.reread_failed {
        return (labels.config_reread_failed.to_owned(), true);
    }
    let Some(said) = &view.said else {
        return (String::new(), false);
    };
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
    let game_line: Vec<&str> =
        [view.game.as_deref(), view.server.as_deref()].into_iter().flatten().collect();
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
