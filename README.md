<p align="right">🌐 <b>English</b> · <a href="./README.zh-CN.md">简体中文</a></p>

# ActingCommand Console

**⚠️ Pre-release, debug phase: interfaces, configuration and file formats still change, and only the newest version is supported.**

This repository holds ActingCommand's **human console** `acui` and its **setup wizard** `acsetup`, both native programs. On one Runtime state root the console opens the ledger's official read face and renders the view pages the ledger itself hands out into a three-column interface (instance card, timeline, detail pane); it reads the ledger and never writes it. It is not part of the Runtime; it is the Runtime's external, detachable client. Like the Runtime, these programs know no game: they contain no game logic and no game data, and a game comes in through the resource packs the Runtime loads.

## Current state

Written on 2026-11-26, for the newest release, **UI v0.12.34**.

- **Release.** The newest release is UI v0.12.34, on this repository's [Releases](https://github.com/HS7097/ActingCommand-UI/releases) page: `acui-windows-<sha>.zip` and `SHA256SUMS`. The zip holds `acui.exe` (the console), `acsetup.exe` (the setup wizard), `LICENSE`, `README.md`, and the `BUILD-MANIFEST.json` that binds them to their commit. Every release is a pre-release, so GitHub marks none as "Latest"; take the one with the newest tag.
- **Pairing.** v0.12.34 is built on the Runtime main commit 83b57cab, whose ledger revision 10 Runtime v0.12.86 releases, and declares `actingd-config` [5, 32], `ledger` [3, 10], `runtime-client` [12, 14] and `runtime-outcomes` [1, 1]. It fits Runtime v0.12.73 to v0.12.86 and does not fit v0.12.72 or earlier. From v0.12.86 on, the Runtime writes ledger revision 10, which UI v0.12.33 and earlier cannot read offline. Whether two versions fit is decided by the interfaces both sides declare, not by version numbers. The 0.11 series is no longer maintained.
- **What the console does today.** It reads the ledger in three columns (instance card, timeline, detail pane), and the main page's right column has three tabs, Live View, Daily Tasks and Goals. Through the Runtime's `ui` channel it also pauses and resumes scheduling (Pause All and Put All on Duty in the top bar, and Pause This Instance's Scheduling), sets goals on the Goals tab, and starts the Runtime or asks it to close. The interface comes in Chinese and English, with three text sizes and two themes.
- **What the setup wizard does today.** From an install set, `acsetup` installs the Runtime, the tools and the console into one new folder and connects the games that are signed in on MuMu emulators or on Android handsets connected over USB; it uses no network and downloads nothing. A handset can be connected and its screen shown, while running tasks on a handset waits for a later version. An install set is put together separately (`tools/make_install_set.py`), and no install set is published at present.
- **Not there yet.** The console has no instance configuration window, and its Add Instance button is greyed out; instances are added with `actinglab instance add` from the Runtime's Tools.
- **Real-device status.** On the project's own machine the console is used with the Runtime of the 0.12 series; the Runtime there schedules six MuMu instances, and routine batches run every day by catalog with four standard packs. The standard packs are not published.

## Documents

- [REFERENCE.md](./REFERENCE.md): the detailed reference moved out of this page (how the versions fit together, the data source, running, the settings file, the launcher, the setup wizard, the four layers and four crates, what the read face blocks); it is not updated with every release.
- What each version changed: the notes of each release on the [Releases](https://github.com/HS7097/ActingCommand-UI/releases) page.
- [model/README.md](./model/README.md): the offline modelling tool `acmodel`, which lives in this repository.

## License

`GPL-3.0-only`. The repository includes the full LICENSE text; the workspace `license` field and the SPDX header of every `.rs` / `.slint` file agree with it. The interface is rendered by [Slint](https://slint.dev), used under its GPLv3 licensing option. The Runtime crates depended on (contract / display-adapter / ledger / ledger-forensics / runtime-client / runtime-config) are `AGPL-3.0-only`, and the two combine under GPLv3 section 13.

No source code of another automation project is in this repository. The upstream license matrix of the whole family is in the umbrella repository's [LICENSE_POLICY.md](https://github.com/HS7097/ActingCommand/blob/main/LICENSE_POLICY.md).
