// SPDX-License-Identifier: GPL-3.0-only
//! The 0.12 setup wizard's back end (Workflow #429 UI-W2a): it installs one 0.12 Runtime, its
//! Tools and the console from an install set into one new folder, a single-location root
//! (Runtime `contracts/install-cli.md`, "single-location root"), and hands the root to the
//! Runtime's own installer (`adopt`). The window (UI-W2b) and the command line (`acsetup`)
//! share everything here.
//!
//! What the Runtime 0.12.58 has no command for, the wizard does itself, each step checked by a
//! Runtime program afterwards: the first configuration (`actingd check-config`), the cold
//! layout of the standard packages (`actinglab standard-pack check`), the offline emulator
//! discovery (`MuMuManager.exe info -v all`, read only), the vision files (each against its
//! pinned SHA-256) and the first start's pause (a pause the console can lift), set before the
//! instances are bound (the Runtime is then closed, its `config replace` puts them in place, and
//! its next start restores the pause before any instance is prepared).
//!
//! It never writes the console's global settings file, never touches another installation,
//! and never deletes anything: a failed step stops the install and leaves what it wrote, and
//! says what to stop and remove before the folder is deleted ([`cleanup`]).

pub mod alias;
pub mod apps;
pub mod archive;
pub mod child;
pub mod config;
pub mod console;
pub mod digest;
pub mod engine;
pub mod layout;
pub mod log;
pub mod manifest;
pub mod mumu;
pub mod occupancy;
pub mod plan;
pub mod platform;
pub mod root;
pub mod runtime;
pub mod set;
pub mod text;
pub mod winpath;

pub use engine::{cleanup, install, Done, Event, Failure, Left, Request, Step};
pub use text::{Lang, Said};
