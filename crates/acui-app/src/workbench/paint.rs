// SPDX-License-Identifier: GPL-3.0-only
//! The account cards, from their views and the table of the language shown, into
//! the list the workbench already shows: a row changes only when its words do, and
//! rows come and go only when the accounts do, so a read every few seconds costs
//! nothing and moves nothing. And the top bar's writes, from their view.

use std::time::Instant;

use acui_rows::format_minute;
use acui_rows::outcome::explain;
use acui_source::workbench::CONSOLE_PAUSE_ALL;
use slint::{Model as _, VecModel};

use super::actions::{ActionsView, ResumeEntry, Said, Write};
use super::card::{Band, CardView, Down, Line, ReasonClass};
use crate::ledger::paint::models;
use crate::ledger::resume::Lifter;
use crate::report;
use crate::strings::{fill, outcome_short, Labels};
use crate::ui::Ui;
use crate::{AccountCard, StatusTone, WorkbenchWindow};

/// What the newest reads say of every account.
pub fn cards(ui: &Ui) {
    let Some(window) = ui.workbench() else {
        return;
    };
    let labels = ui.labels();
    let views = ui.model.borrow().cards(Instant::now());
    let cards = views.iter().map(|view| account_card(ui, labels, view)).collect();
    sync(&window, cards);
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

/// While a global pause holds: what it means, and who lifts it when not this window.
/// `Status` has no member for how many runs still finish, so no number here.
fn pause_note(labels: &Labels, view: &ActionsView) -> String {
    let Some(by) = view.holding else {
        return String::new();
    };
    let mut note = vec![labels.pause_note, labels.pause_note_no_update];
    match by {
        Lifter::Alice => note.push(labels.resume_barred_alice),
        Lifter::Installer => note.push(labels.resume_barred_installer),
        Lifter::Window => {}
    }
    note.join("\n")
}

/// What the last press came to, and whether that is a failure; nothing before one.
fn said_text(labels: &Labels, view: &ActionsView) -> (String, bool) {
    let Some(said) = &view.said else {
        return (String::new(), false);
    };
    match said {
        Said::Sending(Write::PauseAll) => (labels.pause_all_sending.to_owned(), false),
        Said::Sending(Write::ResumeAll) => (labels.resume_all_sending.to_owned(), false),
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
    }
}

fn sync(window: &WorkbenchWindow, cards: Vec<AccountCard>) {
    let current = window.get_account_cards();
    let Some(rows) = current.as_any().downcast_ref::<VecModel<AccountCard>>() else {
        window.set_account_cards(models(cards));
        return;
    };
    while rows.row_count() > cards.len() {
        rows.remove(rows.row_count() - 1);
    }
    for (index, card) in cards.into_iter().enumerate() {
        if index == rows.row_count() {
            rows.push(card);
        } else if rows.row_data(index).as_ref() != Some(&card) {
            rows.set_row_data(index, card);
        }
    }
}

fn account_card(ui: &Ui, labels: &Labels, view: &CardView) -> AccountCard {
    let notes: Vec<String> = view.notes.iter().map(|line| line_text(ui, labels, line)).collect();
    AccountCard {
        alias: view.alias.as_str().into(),
        game_line: view.game.as_deref().unwrap_or_default().into(),
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
