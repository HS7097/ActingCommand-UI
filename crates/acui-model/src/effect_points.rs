// SPDX-License-Identifier: GPL-3.0-only
//! Where an effect intent's input lands on its frame (UI-PT1). The effect intent keeps the input
//! the package asked for in base pixels, while its `frame_extent`, the frame and the recognition's
//! boxes are the device frame's (Runtime 945727d4 `contracts/display-adapter.md:458-464`). The
//! scale between the two is the one of the run's `task.geometry_observed` record in effect (its
//! `display_map`), looked up as `actingledger task-evidence` looks it up, and the input is put on
//! the device frame by the product's own conversion, the display adapter's
//! `DisplayMap::admit_input` and `DisplayMap::device_input`, as that command converts it
//! (`crates/ledger-forensics/src/input_pairing.rs:78-155`; `contracts/display-adapter.md:471-484`).
//! Nothing here converts a coordinate by itself.
//!
//! The `Ui` projection the console reads carries no sampling regions: they are a member of the
//! full payload (`crates/actingcommand-contract/src/event/payload.rs:5357-5365`), and the public
//! summary keeps the task fact alone (same file, 14043). So the input is converted without its
//! boxes. Under the scales the input side sends under, those `admit_input` lets through, no legal
//! point is refused for its box, and the point is the same with its box as without
//! (`contracts/display-adapter.md:439-452`).

use std::collections::BTreeMap;

use acui_rows::{
    code, CaptureExtent, InputAction, ProjectedEvent, TaskGeometryDisplayMap, TaskSemanticFact,
};
use actingcommand_display_adapter::{DisplayMap, DisplayObservation, InputCoordinates};

use crate::task_fact;

/// What the console draws of an effect intent's input on its device frame.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DrawnInput {
    /// The input as asked: no frame size, no map in effect, or a map that is the plain identity,
    /// under which base and device are one space.
    AsAsked,
    /// The input as the product sends it, in device-frame pixels.
    Converted(InputAction),
    /// An input the product refuses under the map in effect (a scale it does not send under, a
    /// point outside the device frame; Runtime 945727d4
    /// `crates/display-adapter/src/lib.rs:975-1004, 1046-1058`): it pairs with nothing, and none
    /// of its points is drawn.
    NotConverted,
}

/// `action`, an effect intent's input in base pixels, as the console draws it on the effect
/// intent's device frame (`frame_extent`) under `display_map`, the run's map in effect
/// (`RunMaps::in_effect`). Step for step the conversion of `sent_as_asked` (Runtime 945727d4
/// `crates/ledger-forensics/src/input_pairing.rs:106-155`): without the frame's size or a map the
/// input as asked; else the map of the record's base on that frame, the frame its own display,
/// with no rotation, crop or insets and no rule, a map that does not build refusing the input;
/// under the plain identity the input as asked; else the input side's admission, then the input
/// for a backend that takes display coordinates. Its boxes are none (module documentation).
///
/// A limit, shared with that reading: the map is rebuilt with no rule, while the host builds the
/// run's map with the game's declared layout (Runtime 945727d4
/// `crates/runtime-host/src/host/contained_task.rs:2990-2997`; the reading's own build,
/// `crates/ledger-forensics/src/input_pairing.rs:116-130`). For a game that declares a layout the
/// two differ on some displays within a pixel of its base's shape: those where the rule's map is a
/// pure scale (the other side, scaled, rounds back to the base's;
/// `crates/display-adapter/src/lib.rs:697-703`) by one factor the input side sends under (twice it
/// is whole), while without the rule the other axis's own factor is not such a factor. A
/// 1280x720 base scaled by its width on a 2560x1439 display is one factor, 2/1, on both axes
/// under the rule, and the host sends the input (`lib.rs:636-660, 995`), but without a rule each
/// axis has its own factor, 2/1 across and 1439/720 down (`lib.rs:624-634`), the latter's double
/// not whole: here the input is not converted, and `actingledger task-evidence` says its input
/// intent's source step is `source_mismatch` (`crates/ledger-forensics/src/lib.rs:1010-1035,
/// 1207-1208`). On the others the host refuses the input too: on 1334x750 and 1366x768, say, the
/// rule's one factor by width, 667/640 or 683/640, is not one it sends under, and by height the
/// rule's map is a translation (`contracts/display-adapter.md:691-694, 747-749`;
/// `crates/display-adapter/src/lib.rs:993-995`). The console keeps that one reading, and draws no
/// point of an input it does
/// not convert.
pub fn drawn_input(
    action: &InputAction,
    frame_extent: Option<CaptureExtent>,
    display_map: Option<&TaskGeometryDisplayMap>,
) -> DrawnInput {
    let (Some(frame), Some(recorded)) = (frame_extent, display_map) else {
        return DrawnInput::AsAsked;
    };
    let map = CaptureExtent::new(recorded.base_width, recorded.base_height).and_then(|base| {
        DisplayMap::build(
            base,
            DisplayObservation { frame, display: frame, rotation: None, crop: None, insets: None },
            None,
        )
        .ok()
    });
    let Some(map) = map else {
        return DrawnInput::NotConverted;
    };
    if map.is_plain_identity() {
        return DrawnInput::AsAsked;
    }
    if map.admit_input(action).is_err() {
        return DrawnInput::NotConverted;
    }
    match map.device_input(action, &[], InputCoordinates::Display) {
        Ok(device) => DrawnInput::Converted(device),
        Err(_) => DrawnInput::NotConverted,
    }
}

/// The display map in effect for an effect intent, as far as the records read of its run say.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InEffect<'a> {
    /// The run's last `task.geometry_observed` record before it names this map.
    Map(&'a TaskGeometryDisplayMap),
    /// The plain identity: that record names no map, or the run has no such record before it.
    Identity,
    /// Not known: the run's records are not read up to the effect intent, or not read whole.
    Unknown,
}

/// The `task.geometry_observed` records read of some runs, by run (its id as `code` writes it):
/// each record's position and the `display_map` it names, and up to where every record of the
/// run is among them.
#[derive(Debug, Clone, Default)]
pub struct RunMaps {
    runs: BTreeMap<String, RunRecords>,
}

#[derive(Debug, Clone, Default)]
struct RunRecords {
    records: BTreeMap<u64, Option<TaskGeometryDisplayMap>>,
    /// Every record of the run at or below this position is in `records`: they were read from
    /// the run's start up to there.
    whole_to: Option<u64>,
}

impl RunMaps {
    /// Takes `event` in when it is a run's `task.geometry_observed` record: whether it is one.
    pub fn take(&mut self, event: &ProjectedEvent) -> bool {
        let Some(TaskSemanticFact::GeometryObserved { observation }) = task_fact(event) else {
            return false;
        };
        let Some(run) = event.links.run_id() else {
            return false;
        };
        self.add(&code(run), event.sequence, observation.display_map);
        true
    }

    /// One geometry record of `run`, at `sequence`, naming `display_map`.
    pub fn add(&mut self, run: &str, sequence: u64, display_map: Option<TaskGeometryDisplayMap>) {
        self.runs.entry(run.to_owned()).or_default().records.insert(sequence, display_map);
    }

    /// Every record of `run` at or below `position` has been taken in: a read of them from the
    /// run's start up to there came back whole.
    pub fn read_to(&mut self, run: &str, position: u64) {
        let records = self.runs.entry(run.to_owned()).or_default();
        records.whole_to = records.whole_to.max(Some(position));
    }

    /// Up to where every record of `run` is known, if it is.
    pub fn whole_to(&self, run: &str) -> Option<u64> {
        self.runs.get(run)?.whole_to
    }

    /// Takes in what `other` read: its records, and how far each of its runs was read whole.
    pub fn absorb(&mut self, other: RunMaps) {
        for (run, theirs) in other.runs {
            let ours = self.runs.entry(run).or_default();
            ours.records.extend(theirs.records);
            ours.whole_to = ours.whole_to.max(theirs.whole_to);
        }
    }

    /// `page`, the records of a page read whole that holds every geometry record after `after`
    /// up to `snapshot`: each run known up to `after` at least takes the page's records of it in
    /// and is known up to `snapshot`. Its records of the other runs are not kept: what came
    /// before them is not known, so no lookup could use them.
    pub fn follow(&mut self, page: &RunMaps, after: u64, snapshot: u64) {
        for (run, ours) in &mut self.runs {
            if !ours.whole_to.is_some_and(|to| to >= after) {
                continue;
            }
            if let Some(theirs) = page.runs.get(run) {
                ours.records.extend(theirs.records.iter().map(|(sequence, map)| (*sequence, *map)));
            }
            ours.whole_to = ours.whole_to.max(Some(snapshot));
        }
    }

    /// The map in effect for an effect intent of `run` at `effect`, its position: the
    /// `display_map` of the run's last geometry record before it, as `last_display_map` takes it
    /// (Runtime 945727d4 `crates/ledger-forensics/src/input_pairing.rs:78-97`), and never one
    /// further back when that record names none. Known only once every record of the run before
    /// the effect intent is.
    pub fn in_effect(&self, run: &str, effect: u64) -> InEffect<'_> {
        let Some(records) = self.runs.get(run) else {
            return InEffect::Unknown;
        };
        if !records.whole_to.is_some_and(|to| to.saturating_add(1) >= effect) {
            return InEffect::Unknown;
        }
        match records.records.range(..effect).next_back() {
            Some((_, Some(map))) => InEffect::Map(map),
            Some((_, None)) | None => InEffect::Identity,
        }
    }

    /// How many runs this holds anything of.
    pub fn run_count(&self) -> usize {
        self.runs.len()
    }

    /// Keeps only the runs `keep` says, by their ids as `code` writes them.
    pub fn retain(&mut self, mut keep: impl FnMut(&str) -> bool) {
        self.runs.retain(|run, _| keep(run));
    }
}

#[cfg(test)]
mod tests {
    //! The contract's examples at the console's pure level (`contracts/display-adapter.md`
    //! sections 10 and 13; Runtime 945727d4 `crates/ledger-forensics/src/input_pairing.rs:209-387`).
    //! Every expected device point is worked out by hand from the rounding table
    //! (`round_half_up(v · s)`), never by the code under test.

    use super::*;
    use crate::frame_group::points;
    use acui_rows::{DisplayArea, DisplayMapKind, DisplayScaleBy, InputPathPoint};

    fn extent(width: u32, height: u32) -> CaptureExtent {
        CaptureExtent::new(width, height).expect("a test extent has two positive sides")
    }

    /// The `display_map` a geometry record names for an undeclared pure scale `num / den` of a
    /// `base` package, as `input_pairing.rs:174-189` builds it.
    fn scale_record(base: (u32, u32), num: u32, den: u32) -> TaskGeometryDisplayMap {
        TaskGeometryDisplayMap {
            kind: DisplayMapKind::Scale,
            scale_num: num,
            scale_den: den,
            scale_width: base.0,
            scale_height: base.1,
            base_width: base.0,
            base_height: base.1,
            scale_by: DisplayScaleBy::Width,
            scale_area: DisplayArea::Screen,
            anchor_area: DisplayArea::Screen,
            insets_source: None,
            declared: false,
        }
    }

    fn tap(x: i32, y: i32) -> InputAction {
        InputAction::Tap { x, y }
    }

    fn swipe(x1: i32, y1: i32, x2: i32, y2: i32) -> InputAction {
        InputAction::Swipe { x1, y1, x2, y2, duration_ms: 300 }
    }

    /// The kernel's vertical-brake drag from the press to the corner, its brake of 100 straight
    /// above the corner, with the interface's times and slopes (`input_pairing.rs:331-346`).
    fn vertical(x1: i32, y1: i32, x2: i32, y2: i32) -> InputAction {
        InputAction::SingleTouchDragWithVerticalBrakeV1 {
            x1,
            y1,
            x2,
            y2,
            x3: x2,
            y3: y2 - 100,
            horizontal_duration_ms: 200,
            corner_hold_ms: 150,
            brake_distance_px: 100,
            brake_duration_ms: 200,
            slope_in: 2,
            slope_out: 0,
        }
    }

    /// A drag with a pause and a brake through the press, the corner and the brake's end: drag
    /// 200 ms, pause 150 ms, brake 200 ms (`input_pairing.rs:350-362`).
    fn paused((x1, y1): (i32, i32), (x2, y2): (i32, i32), (x3, y3): (i32, i32)) -> InputAction {
        InputAction::SingleTouchDragWithPauseAndBrakeV1 {
            x1,
            y1,
            x2,
            y2,
            drag_duration_ms: 200,
            pause_ms: 150,
            x3,
            y3,
            brake_duration_ms: 200,
        }
    }

    /// The path drag in the box (2, 2, 12, 6) through (3, 3) and (13, 7), its one segment 600 ms
    /// and its corner hold 40 ms, every coordinate `k` times (`input_pairing.rs:241-255`).
    fn path(k: i32) -> InputAction {
        InputAction::SingleTouchPathDragV1 {
            box_x: 2 * k,
            box_y: 2 * k,
            box_width: 12 * k,
            box_height: 6 * k,
            points: vec![
                InputPathPoint { x: 3 * k, y: 3 * k },
                InputPathPoint { x: 13 * k, y: 7 * k },
            ],
            segment_duration_ms: vec![600],
            corner_hold_ms: 40,
        }
    }

    /// At s = 2 (16x9 on 32x18) the tap (5, 3) is drawn at (10, 6), and the swipe (1, 5) →
    /// (11, 5) at (2, 10) → (22, 10), its 300 ms kept; at s = 3/2 (1280x720 on 1920x1080) the
    /// tap (1, 0) at (2, 0), `round_half_up(1.5)` = 2.
    #[test]
    fn a_scaled_input_is_drawn_where_the_product_sends_it() {
        let double = scale_record((16, 9), 2, 1);
        let device = Some(extent(32, 18));
        assert_eq!(
            drawn_input(&tap(5, 3), device, Some(&double)),
            DrawnInput::Converted(tap(10, 6))
        );
        let swiped = drawn_input(&swipe(1, 5, 11, 5), device, Some(&double));
        assert_eq!(swiped, DrawnInput::Converted(swipe(2, 10, 22, 10)));
        let DrawnInput::Converted(sent) = swiped else {
            unreachable!("asserted converted above");
        };
        assert_eq!(points(&sent), vec![(2.0, 10.0), (22.0, 10.0)]);
        assert_eq!(
            drawn_input(&tap(1, 0), Some(extent(1920, 1080)), Some(&scale_record((1280, 720), 3, 2))),
            DrawnInput::Converted(tap(2, 0))
        );
    }

    /// At s = 5/4 (16x12 on 20x15), a scale the input side does not send under, the tap (1, 0)
    /// is not converted, though `round_half_up(1.25)` would put it at (1, 0).
    #[test]
    fn an_input_the_product_refuses_is_not_converted() {
        assert_eq!(
            drawn_input(&tap(1, 0), Some(extent(20, 15)), Some(&scale_record((16, 12), 5, 4))),
            DrawnInput::NotConverted
        );
    }

    /// Section 13 (`contracts/display-adapter.md:691-694, 732-737`): on a 1280x720 base a
    /// 1334x750 display is a pure scale of 667/640 across and 25/24 down, whose doubles are not
    /// whole, so the input side refuses every press there: the tap (1, 0) is not converted. Its
    /// record names the width's factor (Runtime 945727d4 `crates/display-adapter/src/lib.rs:814-836`).
    #[test]
    fn a_1334x750_display_on_a_1280x720_base_is_not_converted() {
        assert_eq!(
            drawn_input(
                &tap(1, 0),
                Some(extent(1334, 750)),
                Some(&scale_record((1280, 720), 667, 640))
            ),
            DrawnInput::NotConverted
        );
    }

    /// Section 13, as above: a 1366x768 display on a 1280x720 base, 683/640 across and 16/15
    /// down; the tap (1, 0) is not converted.
    #[test]
    fn a_1366x768_display_on_a_1280x720_base_is_not_converted() {
        assert_eq!(
            drawn_input(
                &tap(1, 0),
                Some(extent(1366, 768)),
                Some(&scale_record((1280, 720), 683, 640))
            ),
            DrawnInput::NotConverted
        );
    }

    /// At s = 3 (16x9 on 48x27) the path drag in (2, 2, 12, 6) through (3, 3) and (13, 7) is
    /// drawn as the product sends it, its box and its points three times
    /// (`input_pairing.rs:241-262`), so its points are (9, 9) and (39, 21); as asked, its points
    /// are its own.
    #[test]
    fn a_scaled_path_drag_is_drawn_through_its_points() {
        let drawn =
            drawn_input(&path(1), Some(extent(48, 27)), Some(&scale_record((16, 9), 3, 1)));
        assert_eq!(drawn, DrawnInput::Converted(path(3)));
        let DrawnInput::Converted(sent) = drawn else {
            unreachable!("asserted converted above");
        };
        assert_eq!(points(&sent), vec![(9.0, 9.0), (39.0, 21.0)]);
        assert_eq!(points(&path(1)), vec![(3.0, 3.0), (13.0, 7.0)]);
    }

    /// With no record of the run, a record that names no map, no frame size, or a record whose
    /// base is the frame's own size (a map that builds as the plain identity), the tap (5, 3) is
    /// drawn as asked.
    #[test]
    fn under_the_identity_the_input_is_drawn_as_asked() {
        let double = scale_record((16, 9), 2, 1);
        assert_eq!(drawn_input(&tap(5, 3), Some(extent(32, 18)), None), DrawnInput::AsAsked);
        assert_eq!(drawn_input(&tap(5, 3), None, Some(&double)), DrawnInput::AsAsked);
        assert_eq!(drawn_input(&tap(5, 3), Some(extent(16, 9)), Some(&double)), DrawnInput::AsAsked);
        let mut runs = RunMaps::default();
        runs.read_to("run_a", 100);
        assert_eq!(runs.in_effect("run_a", 50), InEffect::Identity);
        runs.add("run_a", 10, None);
        assert_eq!(runs.in_effect("run_a", 50), InEffect::Identity);
        assert_eq!(points(&tap(5, 3)), vec![(5.0, 3.0)]);
    }

    /// A scaled run's vertical-brake step is drawn as the drag with a pause and a brake the
    /// product sends (`input_pairing.rs:364-387`): at s = 2 (320x180 on 640x360) the drag (300,
    /// 150) → (20, 150) as (600, 300) → (40, 300) → (40, 100), its times 200, 150 and 200; at
    /// s = 3/2 (1280x720 on 1920x1080) the drag (201, 601) → (901, 501) as (302, 902) →
    /// (1352, 752) → (1352, 602) (301.5, 901.5, 1351.5 and 751.5 up, its brake 150).
    #[test]
    fn a_scaled_vertical_brake_step_is_drawn_as_the_drag_sent() {
        assert_eq!(
            drawn_input(
                &vertical(300, 150, 20, 150),
                Some(extent(640, 360)),
                Some(&scale_record((320, 180), 2, 1))
            ),
            DrawnInput::Converted(paused((600, 300), (40, 300), (40, 100)))
        );
        let drawn = drawn_input(
            &vertical(201, 601, 901, 501),
            Some(extent(1920, 1080)),
            Some(&scale_record((1280, 720), 3, 2)),
        );
        assert_eq!(drawn, DrawnInput::Converted(paused((302, 902), (1352, 752), (1352, 602))));
        let DrawnInput::Converted(sent) = drawn else {
            unreachable!("asserted converted above");
        };
        assert_eq!(points(&sent), vec![(302.0, 902.0), (1352.0, 752.0), (1352.0, 602.0)]);
    }

    /// The map in effect is the run's last record before the effect intent: not another run's,
    /// not one after it, and not an earlier one's map when the last names none.
    #[test]
    fn the_map_in_effect_is_the_runs_last_record_before_the_intent() {
        let double = scale_record((16, 9), 2, 1);
        let triple = scale_record((16, 9), 3, 1);
        let mut runs = RunMaps::default();
        runs.add("run_a", 10, Some(double));
        runs.add("run_a", 20, Some(triple));
        runs.add("run_a", 40, Some(double));
        runs.add("run_b", 30, Some(double));
        runs.read_to("run_a", 100);
        runs.read_to("run_b", 100);
        assert_eq!(runs.in_effect("run_a", 35), InEffect::Map(&triple));
        assert_eq!(runs.in_effect("run_a", 15), InEffect::Map(&double));
        assert_eq!(runs.in_effect("run_a", 5), InEffect::Identity);
        runs.add("run_a", 25, None);
        assert_eq!(runs.in_effect("run_a", 35), InEffect::Identity);
        assert_eq!(runs.in_effect("run_a", 50), InEffect::Map(&double));
        assert_eq!(runs.in_effect("run_c", 50), InEffect::Unknown);
    }

    /// A run whose records are not read up to the effect intent is not known there, nor is one
    /// whose read was not whole; a whole page after what is known carries a run on, and not one
    /// known only up to before it.
    #[test]
    fn a_run_not_read_up_to_the_intent_is_unknown() {
        let double = scale_record((16, 9), 2, 1);
        let mut runs = RunMaps::default();
        runs.add("run_a", 10, Some(double));
        assert_eq!(runs.in_effect("run_a", 20), InEffect::Unknown);
        runs.read_to("run_a", 50);
        assert_eq!(runs.in_effect("run_a", 51), InEffect::Map(&double));
        assert_eq!(runs.in_effect("run_a", 60), InEffect::Unknown);
        runs.read_to("run_b", 30);
        let mut page = RunMaps::default();
        page.add("run_a", 55, None);
        page.add("run_b", 55, Some(double));
        runs.follow(&page, 50, 80);
        assert_eq!(runs.in_effect("run_a", 60), InEffect::Identity);
        assert_eq!(runs.in_effect("run_b", 60), InEffect::Unknown);
        assert_eq!(runs.whole_to("run_b"), Some(30));
    }
}
