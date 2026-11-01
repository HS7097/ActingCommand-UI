// SPDX-License-Identifier: GPL-3.0-only
//! The account cards, from their views and the table of the language shown, into
//! the list the workbench already shows: a row changes only when its words do, and
//! rows come and go only when the accounts do, so a read every few seconds costs
//! nothing and moves nothing.

use acui_rows::outcome::explain;
use slint::{Model as _, VecModel};

use super::card::{Band, CardView, Down, Line};
use crate::ledger::paint::models;
use crate::ledger::resume::Lifter;
use crate::report;
use crate::strings::{fill, Labels};
use crate::ui::Ui;
use crate::{AccountCard, StatusTone, WorkbenchWindow};

/// What the newest reads say of every account.
pub fn cards(ui: &Ui) {
    let Some(window) = ui.workbench() else {
        return;
    };
    let labels = ui.labels();
    let views = ui.model.borrow().cards();
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
        // None of these lines says a failure.
        main_failed: false,
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
        Line::Nothing => String::new(),
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

/// A fence's cause in plain words. The card never shows the code: one the tables do
/// not know gets a general sentence, and goes to the log once.
fn cause_words(ui: &Ui, labels: &Labels, cause: &str) -> String {
    match explain(cause, labels.tongue.outcome_tongue()) {
        Ok(Some(words)) => words.short.to_owned(),
        Ok(None) => {
            if ui.model.borrow_mut().first_told(cause) {
                report::log("acui-cards", &format!("no words for fence cause {cause}"));
            }
            labels.card_cause_unknown.to_owned()
        }
        Err(error) => fill(labels.outcome_table_unusable, &[&error.to_string()]),
    }
}
