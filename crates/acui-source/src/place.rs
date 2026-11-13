// SPDX-License-Identifier: GPL-3.0-only
//! Where this console belongs, as plain judgment over what was found: which tier its paths
//! come from (an A/B installation's selection, a single-location root, or the per-user
//! acui.toml), which settings file it uses, whether a folder is a single-location root, and
//! whether a running Runtime is one of that root's. Every file looked at, every path resolved
//! and every process asked about is the caller's (the console's `install` and `identity`),
//! handed in as facts and as paths it only passes back: nothing here touches the file system
//! or another process, so each rule reads the same on every platform and is tested as it is
//! (the owner's ruling 181 (A)).
//!
//! The rules are the Runtime's own (contracts/install-cli.md, Layout): a single-location root
//! holds `runtime\BUILD-MANIFEST.json` and no `install\active.json`, its Runtime runs from
//! `runtime\` or from a kept build's `install\builds\<commit>\runtime\`; an A/B installation
//! is marked by `install\active.json`, and its slots `A` and `B` sit right under its root.

/// What the caller found in one folder that may be a single-location root. An
/// `install\active.json` that cannot be looked at counts as there, so that a folder is never
/// taken for a root on a guess; so does a configuration that cannot be looked at, so that what
/// is wrong with it is said.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct RootLook {
    /// `runtime\BUILD-MANIFEST.json` is a file.
    pub manifest: bool,
    /// `actingd.config.json` is there.
    pub config: bool,
    /// `runtime\` is a folder.
    pub runtime: bool,
    /// `install\active.json` is there: the folder is an A/B installation's root.
    pub selection: bool,
    /// The folder above holds `install\active.json`: the folder is a slot of an A/B installation.
    pub selection_above: bool,
}

impl RootLook {
    /// Whether an A/B installation's selection is in it or above it: then it is no
    /// single-location root, whatever else it holds.
    pub fn selected(&self) -> bool {
        self.selection || self.selection_above
    }

    /// Whether the folder this program's `ui\` sits in is the installation folder it belongs
    /// to: a whole root, with the manifest, or half of one, with the configuration or
    /// `runtime\` and no manifest, which is then said rather than passed over. Never a folder an
    /// A/B installation's selection is in or above.
    pub fn holds_console(&self) -> bool {
        !self.selected() && (self.manifest || self.config || self.runtime)
    }

    /// Whether `--root` may name it: an `absolute` path, the manifest there, and no
    /// installation selection in it or above it.
    pub fn named_root(&self, absolute: bool) -> bool {
        absolute && self.manifest && !self.selected()
    }

    /// Whether the selection reader takes this root for a slot: a folder `named_slot` (`A` or
    /// `B`, with this program in its `ui\` spelt exactly so, as that reader compares) that holds
    /// the manifest and has no installation selection above it, so that it is no slot after all.
    pub fn slot_named(&self, named_slot: bool) -> bool {
        named_slot && self.manifest && !self.selection_above
    }
}

/// What reading this process's installation selection came to: the selection (`S`) with its
/// installation's root (`P`), none, or the reader's error (`E`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Selection<S, E, P> {
    Selected { selection: S, root: P },
    Unselected,
    Failed(E),
}

/// `--root`, as the caller found it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GivenRoot<P> {
    pub path: P,
    pub absolute: bool,
    pub look: RootLook,
    /// It names the same place as the selected installation's root; false without one.
    pub same_as_selected: bool,
    /// It names the same place as the folder this program's `ui\` sits in; false without one.
    pub same_as_own: bool,
}

/// Everything `place` judges by.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlaceFacts<S, E, P> {
    pub selection: Selection<S, E, P>,
    /// The folder this program's `ui\` (so named in any case) sits in, and what is in it; `None`
    /// when the program sits in no folder named `ui`.
    pub own: Option<(P, RootLook)>,
    /// `--root`, when the command line was read and gave it.
    pub given: Option<GivenRoot<P>>,
}

/// How a single-location root was found: where this program is, or `--root`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RootBy {
    Exe,
    Arg,
}

/// The tier this run's paths come from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Tier<S, P> {
    /// The installation selection's.
    Selected(S),
    /// A single-location root's, whole or not: what it gives is the caller's to read.
    Root { root: P, by: RootBy },
    /// `--state-root`, else the per-user acui.toml, as ever.
    Legacy,
}

/// Why no tier gives this run paths.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Refused<E, P> {
    /// The installation selection could not be read.
    Selection(E),
    /// `--root` (`given`) names another folder than the one this console belongs to (`root`):
    /// the selected installation's root, or the folder its `ui\` sits in.
    RootDiffers { given: P, root: P },
    /// `--root` names no single-location root.
    RootArgNotRoot(P),
}

/// What `place` decided.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Placed<S, E, P> {
    pub tier: Result<Tier<S, P>, Refused<E, P>>,
    /// The single-location root whose own acui.toml this run may use (`root_settings` says
    /// whether it does); `None`, the per-user acui.toml.
    pub settings_root: Option<P>,
}

/// The tier and the settings, in the order the console's head states: an installation
/// selection first, which `--root` may only repeat; without one, the folder this program's
/// `ui\` sits in when it holds a console, which `--root` may only repeat too; else the folder
/// `--root` names, when it is a root; else acui.toml.
///
/// A console in an installation folder of its own keeps that folder's settings whatever else
/// fails, a selection that could not be read too (a folder named `A` or `B`, or selection
/// variables another installation's process left behind), and never falls back to the
/// per-user file; a slot of an A/B installation is no such folder (`RootLook::holds_console`).
pub fn place<S, E, P: Clone>(facts: PlaceFacts<S, E, P>) -> Placed<S, E, P> {
    let PlaceFacts { selection, own, given } = facts;
    let own = own.filter(|(_, look)| look.holds_console()).map(|(path, _)| path);
    match selection {
        Selection::Failed(error) => Placed { tier: Err(Refused::Selection(error)), settings_root: own },
        Selection::Selected { selection, root } => {
            let tier = match given {
                Some(given) if !given.same_as_selected => {
                    Err(Refused::RootDiffers { given: given.path, root })
                }
                _ => Ok(Tier::Selected(selection)),
            };
            Placed { tier, settings_root: None }
        }
        Selection::Unselected => match (own, given) {
            (Some(own), Some(given)) if !given.same_as_own => Placed {
                settings_root: Some(own.clone()),
                tier: Err(Refused::RootDiffers { given: given.path, root: own }),
            },
            (Some(own), _) => Placed {
                settings_root: Some(own.clone()),
                tier: Ok(Tier::Root { root: own, by: RootBy::Exe }),
            },
            (None, Some(given)) if given.look.named_root(given.absolute) => Placed {
                settings_root: Some(given.path.clone()),
                tier: Ok(Tier::Root { root: given.path, by: RootBy::Arg }),
            },
            (None, Some(given)) => {
                Placed { tier: Err(Refused::RootArgNotRoot(given.path)), settings_root: None }
            }
            (None, None) => Placed { tier: Ok(Tier::Legacy), settings_root: None },
        },
    }
}

/// Which settings file a run in a root uses.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RootSettings {
    /// The root's own `<root>\acui.toml`.
    Own,
    /// The per-user acui.toml.
    Global,
}

/// The root's own file once it is there (`own_there`; a place that cannot be looked at counts
/// as there, so that what is wrong with it is said); before that the per-user file only when it
/// names this root's configuration (`global_names_root`: a root set up through the per-user file
/// before the console was put in it keeps its settings); else the root's own, its defaults until
/// a first save makes it.
pub fn root_settings(own_there: bool, global_names_root: bool) -> RootSettings {
    if !own_there && global_names_root {
        RootSettings::Global
    } else {
        RootSettings::Own
    }
}

/// The folder a single-location root's Runtime runs from, and the two above a kept build's.
const RUNTIME: &str = "runtime";
const INSTALL: &str = "install";
const BUILDS: &str = "builds";

/// Whether a Runtime whose program file is `program` is one of the installation folder
/// `root`'s: `<root>\runtime\<file>`, or a kept build's `<root>\install\builds\<commit>\runtime\<file>`.
/// Both paths as the system resolves them (canonical, links followed, without `\\?\`), compared
/// part by part, `\` and `/` alike, letters A–Z in either case as Windows takes them. A path with
/// no part, or with a `.` or `..` part, is no one's.
pub fn runs_from_root(program: &str, root: &str) -> bool {
    let (Some(program), Some(root)) = (parts(program), parts(root)) else {
        return false;
    };
    if program.len() <= root.len()
        || !program.iter().zip(&root).all(|(part, of_root)| part.eq_ignore_ascii_case(of_root))
    {
        return false;
    }
    match program[root.len()..] {
        [folder, _] => folder.eq_ignore_ascii_case(RUNTIME),
        [install, builds, _, folder, _] => {
            install.eq_ignore_ascii_case(INSTALL)
                && builds.eq_ignore_ascii_case(BUILDS)
                && folder.eq_ignore_ascii_case(RUNTIME)
        }
        _ => false,
    }
}

/// A path's parts, empty ones (a doubled or trailing separator, a UNC path's lead) dropped;
/// `None` for one with none, or with a `.` or `..` part.
fn parts(path: &str) -> Option<Vec<&str>> {
    let parts: Vec<&str> = path.split(['\\', '/']).filter(|part| !part.is_empty()).collect();
    let plain = !parts.is_empty() && parts.iter().all(|part| *part != "." && *part != "..");
    plain.then_some(parts)
}

#[cfg(test)]
mod tests {
    use super::{
        place, root_settings, runs_from_root, GivenRoot, PlaceFacts, Placed, Refused, RootBy,
        RootLook, RootSettings, Selection, Tier,
    };

    const OWN: &str = r"C:\Installs\Ours";
    const OTHER: &str = r"C:\Installs\Other";
    const SELECTED_ROOT: &str = r"C:\Installs\Paired";

    const WHOLE: RootLook =
        RootLook { manifest: true, config: true, runtime: true, selection: false, selection_above: false };
    const HALF: RootLook =
        RootLook { manifest: false, config: true, runtime: false, selection: false, selection_above: false };
    const NOTHING: RootLook =
        RootLook { manifest: false, config: false, runtime: false, selection: false, selection_above: false };
    /// A slot of an A/B installation: `A\` with its `runtime\`, the selection above it.
    const SLOT: RootLook =
        RootLook { manifest: true, config: false, runtime: true, selection: false, selection_above: true };

    type Facts = PlaceFacts<&'static str, &'static str, &'static str>;
    type Decided = Placed<&'static str, &'static str, &'static str>;

    fn facts(
        selection: Selection<&'static str, &'static str, &'static str>,
        own: Option<RootLook>,
        given: Option<GivenRoot<&'static str>>,
    ) -> Facts {
        PlaceFacts { selection, own: own.map(|look| (OWN, look)), given }
    }

    fn given(path: &'static str, look: RootLook) -> GivenRoot<&'static str> {
        GivenRoot { path, absolute: true, look, same_as_selected: false, same_as_own: path == OWN }
    }

    fn decided(tier: Result<Tier<&'static str, &'static str>, Refused<&'static str, &'static str>>, settings_root: Option<&'static str>) -> Decided {
        Placed { tier, settings_root }
    }

    #[test]
    fn a_console_in_a_whole_root_takes_it_and_its_own_settings() {
        let placed = place(facts(Selection::Unselected, Some(WHOLE), None));
        assert_eq!(placed, decided(Ok(Tier::Root { root: OWN, by: RootBy::Exe }), Some(OWN)));
    }

    #[test]
    fn a_console_in_half_a_root_takes_it_too_and_never_the_per_user_file() {
        for half in [HALF, RootLook { runtime: true, ..NOTHING }] {
            let placed = place(facts(Selection::Unselected, Some(half), None));
            assert_eq!(placed, decided(Ok(Tier::Root { root: OWN, by: RootBy::Exe }), Some(OWN)), "{half:?}");
        }
    }

    #[test]
    fn a_console_outside_any_root_goes_the_old_way() {
        for own in [None, Some(NOTHING)] {
            let placed = place(facts(Selection::Unselected, own, None));
            assert_eq!(placed, decided(Ok(Tier::Legacy), None), "{own:?}");
        }
    }

    #[test]
    fn a_folder_with_an_installation_selection_holds_no_console() {
        let ab_root = RootLook { selection: true, ..WHOLE };
        for look in [ab_root, SLOT] {
            assert!(!look.holds_console(), "{look:?}");
            let placed = place(facts(Selection::Unselected, Some(look), None));
            assert_eq!(placed, decided(Ok(Tier::Legacy), None), "{look:?}");
        }
    }

    #[test]
    fn root_arg_names_a_root_for_a_console_outside_one() {
        let placed = place(facts(Selection::Unselected, None, Some(given(OTHER, WHOLE))));
        assert_eq!(placed, decided(Ok(Tier::Root { root: OTHER, by: RootBy::Arg }), Some(OTHER)));
    }

    #[test]
    fn root_arg_in_a_root_may_only_repeat_it() {
        let same = place(facts(Selection::Unselected, Some(WHOLE), Some(given(OWN, WHOLE))));
        assert_eq!(same, decided(Ok(Tier::Root { root: OWN, by: RootBy::Exe }), Some(OWN)));
        let other = place(facts(Selection::Unselected, Some(WHOLE), Some(given(OTHER, WHOLE))));
        assert_eq!(other, decided(Err(Refused::RootDiffers { given: OTHER, root: OWN }), Some(OWN)));
    }

    #[test]
    fn root_arg_with_a_selection_may_only_repeat_its_root() {
        let selected = || Selection::Selected { selection: "slot A", root: SELECTED_ROOT };
        let mut repeated = given(SELECTED_ROOT, NOTHING);
        repeated.same_as_selected = true;
        let same = place(facts(selected(), None, Some(repeated)));
        assert_eq!(same, decided(Ok(Tier::Selected("slot A")), None));
        let other = place(facts(selected(), None, Some(given(OTHER, WHOLE))));
        assert_eq!(other, decided(Err(Refused::RootDiffers { given: OTHER, root: SELECTED_ROOT }), None));
    }

    #[test]
    fn root_arg_relative_or_without_the_manifest_is_no_root() {
        let relative = GivenRoot { absolute: false, ..given(OTHER, WHOLE) };
        for arg in [relative, given(OTHER, HALF), given(OTHER, NOTHING)] {
            let placed = place(facts(Selection::Unselected, None, Some(arg.clone())));
            assert_eq!(placed, decided(Err(Refused::RootArgNotRoot(OTHER)), None), "{arg:?}");
        }
    }

    /// The second review point: `--root` takes no folder an installation selection is in or above.
    #[test]
    fn root_arg_with_an_installation_selection_is_no_root() {
        let ab_root = RootLook { selection: true, ..WHOLE };
        for look in [ab_root, SLOT] {
            let placed = place(facts(Selection::Unselected, None, Some(given(OTHER, look))));
            assert_eq!(placed, decided(Err(Refused::RootArgNotRoot(OTHER)), None), "{look:?}");
        }
    }

    /// The first review point: a selection that could not be read keeps a root's own settings.
    #[test]
    fn a_selection_that_failed_in_a_root_keeps_its_own_settings() {
        for own in [WHOLE, HALF] {
            let placed = place(facts(Selection::Failed("lock missing"), Some(own), None));
            assert_eq!(placed, decided(Err(Refused::Selection("lock missing")), Some(OWN)), "{own:?}");
        }
    }

    #[test]
    fn a_selection_that_failed_in_a_slot_or_outside_a_root_keeps_the_per_user_file() {
        for own in [Some(SLOT), Some(RootLook { selection: true, ..WHOLE }), Some(NOTHING), None] {
            let placed = place(facts(Selection::Failed("lock missing"), own, None));
            assert_eq!(placed, decided(Err(Refused::Selection("lock missing")), None), "{own:?}");
        }
    }

    #[test]
    fn a_selection_is_first_and_keeps_the_per_user_file() {
        let selected = Selection::Selected { selection: "slot B", root: SELECTED_ROOT };
        let placed = place(facts(selected, Some(WHOLE), None));
        assert_eq!(placed, decided(Ok(Tier::Selected("slot B")), None));
    }

    /// The third review point: a selection above that cannot be looked at counts as there.
    #[test]
    fn a_root_named_like_a_slot_is_said_only_without_a_selection_above() {
        assert!(WHOLE.slot_named(true));
        assert!(!WHOLE.slot_named(false));
        assert!(!SLOT.slot_named(true));
        assert!(!HALF.slot_named(true));
    }

    #[test]
    fn a_root_uses_its_own_settings_once_there_else_the_per_user_file_naming_it() {
        assert_eq!(root_settings(true, true), RootSettings::Own);
        assert_eq!(root_settings(true, false), RootSettings::Own);
        assert_eq!(root_settings(false, true), RootSettings::Global);
        assert_eq!(root_settings(false, false), RootSettings::Own);
    }

    #[test]
    fn a_runtime_runs_from_its_root_or_one_of_its_kept_builds() {
        for program in [
            r"C:\Installs\Ours\runtime\actingcommand-actingd.exe",
            r"c:\installs\ours\Runtime\actingcommand-actingd.exe",
            r"C:\Installs\Ours\runtime\actingcommand-actingd.exe.old-1794532592345",
            r"C:\Installs\Ours\install\builds\0123456789abcdef0123456789abcdef01234567\runtime\actingcommand-actingd.exe",
            "C:/Installs/Ours/runtime/actingcommand-actingd.exe",
        ] {
            assert!(runs_from_root(program, OWN), "{program}");
        }
        assert!(runs_from_root(r"\\host\share\Ours\runtime\actingd.exe", r"\\host\share\Ours"));
        assert!(runs_from_root(r"C:\runtime\actingd.exe", r"C:\"));
    }

    #[test]
    fn a_runtime_elsewhere_is_not_the_roots() {
        for program in [
            r"C:\Installs\Other\runtime\actingcommand-actingd.exe",
            r"C:\Installs\Ours2\runtime\actingcommand-actingd.exe",
            r"C:\Installs\runtime\actingcommand-actingd.exe",
            r"C:\Installs\Ours\actingcommand-actingd.exe",
            r"C:\Installs\Ours\tools\actingcommand-actingd.exe",
            r"C:\Installs\Ours\runtime\bin\actingcommand-actingd.exe",
            r"C:\Installs\Ours\ui\runtime\actingcommand-actingd.exe",
            r"C:\Installs\Ours\install\builds\runtime\actingcommand-actingd.exe",
            r"C:\Installs\Ours\install\kept\0123\runtime\actingcommand-actingd.exe",
            r"C:\Installs\Ours\runtime\..\..\Other\runtime\actingcommand-actingd.exe",
            r"C:\Installs\Ours\runtime",
            r"C:\Installs\Ours",
            "",
        ] {
            assert!(!runs_from_root(program, OWN), "{program}");
        }
        assert!(!runs_from_root(r"C:\Installs\Ours\runtime\actingd.exe", ""));
        assert!(!runs_from_root(r"C:\Installs\Ours\runtime\actingd.exe", r"C:\Installs\.\Ours"));
    }
}
