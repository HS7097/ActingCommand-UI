// SPDX-License-Identifier: GPL-3.0-only
//! The run's key salt: 32 bytes of the operating system's random source as 64 hex digits, one per
//! run of the wizard, minted when it starts and held in its memory alone. The phone page's
//! device list and the install's bind each hand it to the Lab (`devices --key-salt-env`,
//! `devices bind --key-salt-env`: `devices`), so that the Lab names a phone by the same keyed
//! digest of its serial both times, and by no key another run could match.
//!
//! It is never written to disk or to the log: its `Debug` and `Display` write [`REDACTED`], and
//! it leaves the wizard only as a variable of one child's environment ([`KeySalt::pass_in_env`]),
//! a secret one, which is on no command line (a command line is visible to the user's other
//! processes and to a system log that records command lines; Runtime
//! `contracts/device-binding.md`, "Device keys") and which the child module writes [`REDACTED`]
//! for in the spec's `Debug` and wherever the child's output repeats it
//! (`child::Spec::secret_env`).

use std::fmt;

use crate::child::{Spec, REDACTED};
use crate::config::mint_salt;
use crate::text::Said;

/// The run's key salt (see the module's description).
#[derive(Clone)]
pub struct KeySalt(String);

impl KeySalt {
    /// A new salt from the operating system's random source.
    pub fn mint() -> Result<Self, Said> {
        mint_salt().map(Self)
    }

    /// `spec` with the salt as the value of its child's variable `variable`, a secret one; the
    /// command line names the variable alone (`--key-salt-env <variable>`).
    pub fn pass_in_env(&self, spec: Spec, variable: &str) -> Spec {
        spec.secret_env(variable, &self.0)
    }
}

impl fmt::Debug for KeySalt {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(REDACTED)
    }
}

impl fmt::Display for KeySalt {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(REDACTED)
    }
}
