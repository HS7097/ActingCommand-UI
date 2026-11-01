// SPDX-License-Identifier: GPL-3.0-only
//! The account cards, from their views and the table of the language shown, into
//! the list the workbench already shows: a row changes only when its words do, and
//! rows come and go only when the accounts do, so a read every few seconds costs
//! nothing and moves nothing.

use std::time::Instant;

use acui_rows::format_minute;
use acui_rows::outcome::explain;
use slint::{Model as _, VecModel};

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
            let task = package.as_deref().unwrap_or(labels.card_draining_any);
            fill(labels.card_draining, &[task])
        }
        Line::HandingBack => labels.card_handing_back.to_owned(),
        Line::Paused { everyone, by, reason } => {
            let said = match by {
                Lifter::Alice => labels.card_paused_alice.to_owned(),
                Lifter::Installer => labels.card_paused_installer.to_owned(),
                Lifter::Window => fill(labels.card_paused_other, &[reason]),
            };
            if *everyone {
                fill(labels.card_all_accounts, &[&said])
            } else {
                said
            }
        }
        Line::Running { package } => fill(labels.card_running, &[task_name(labels, package)]),
        Line::Finishing { package } => fill(labels.card_finishing, &[task_name(labels, package)]),
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

fn task_name<'a>(labels: &Labels, package: &'a Option<String>) -> &'a str {
    package.as_deref().unwrap_or(labels.card_task_unnamed)
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
                report::log("acui-cards", &line);
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
                report::log("acui-cards", &line);
            }
            labels.card_cause_unknown.to_owned()
        }
        Err(error) => fill(labels.outcome_table_unusable, &[&error.to_string()]),
    }
}
