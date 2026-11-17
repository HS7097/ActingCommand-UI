// SPDX-License-Identifier: GPL-3.0-only
//! The run's key salt: 32 bytes of the operating system's random source as 64 hex digits, one per
//! run of the wizard, minted when it starts and held in its memory alone. The phone page's
//! device list and the install's bind are each to hand it to the Lab (`devices --key-salt`), so
//! that the Lab names a phone by the same keyed digest of its serial both times, and by no key
//! another run could match.
//!
//! It is never written to disk or to the log: its `Debug` and `Display` write [`REDACTED`], and
//! it leaves the wizard only as a child's argument ([`KeySalt::pass_to`]), a secret one, which
//! the child module writes [`REDACTED`] for wherever it shows the command line and wherever the
//! child's output repeats it (`child::Spec::secret_arg`).

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

    /// `spec` with the salt as its next argument, a secret one; the option's name goes before it
    /// as a plain argument.
    pub fn pass_to(&self, spec: Spec) -> Spec {
        spec.secret_arg(&self.0)
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
