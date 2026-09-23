// SPDX-License-Identifier: GPL-3.0-only
//! ActingCommand 监控台 / ActingCommand Console: a read-only window over one
//! Runtime state root, opened through the formal ledger read face — offline
//! over the files, or online through the typed client of the running Runtime —
//! with a launcher block that starts the daemon detached and asks it, through
//! the same typed client, to shut down. A second window lists and edits the
//! `instances` of the actingd configuration that block starts the daemon with.

mod instances;
mod launcher;
mod settings;
mod strings;

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use acui_model::{
    tab_from_name, DisplayRow, FrameTarget, InstanceCard, Overlay, QueryError, RecoveryGroup,
    ViewModel, ALL_TABS, FILL_ROWS, FILL_WINDOWS, WINDOW,
};
use acui_rows::{
    code, event_type_names, format_bytes, format_clock, format_full, links_named, module_names,
    seconds_since, short_id, ArtifactEvictionObservation, EventQuery, EventSeverity, LedgerCount,
    LedgerView, OriginModule, PortBindings, PortEntry, ProjectedEvent, RuntimeEventQueryPage,
    Sensitivity, WriterFacts, MAX_RUNTIME_EVENT_QUERY_EVENTS,
};
use acui_source::{
    InstanceFacts, MaterialOutcome, OfflineFacts, OfflineReason, ReadSource, RuntimeInstance,
    Session, SourceMode, MAX_FRAME_BYTES,
};
use anyhow::{bail, Result};
use settings::TextSize;
use slint::{
    Image, ModelRc, Rgba8Pixel, SharedPixelBuffer, SharedString, Timer, TimerMode, VecModel,
};
use strings::{fill, Labels, Language};

slint::include_modules!();

#[derive(Default)]
struct Args {
    state_root: Option<PathBuf>,
    tab: Option<LedgerView>,
    /// `--lang` overrides the settings file for this run only.
    language: Option<Language>,
    /// `--source`: `auto` unless told otherwise.
    source: SourceMode,
    help: bool,
}

fn parse_args() -> Result<Args> {
    let mut args = Args::default();
    let mut argv = std::env::args().skip(1);
    while let Some(flag) = argv.next() {
        match flag.as_str() {
            "--state-root" => match argv.next() {
                Some(path) => args.state_root = Some(PathBuf::from(path)),
                None => bail!("--state-root 缺少路径 / --state-root needs a path"),
            },
            "--tab" => {
                let name = match argv.next() {
                    Some(name) => name,
                    None => bail!("--tab 缺少取值 / --tab needs a value"),
                };
                args.tab = match tab_from_name(&name) {
                    Some(view) => Some(view),
                    None => bail!("未知的 --tab 取值 / unknown --tab value: {name}"),
                };
            }
            "--lang" => {
                let name = match argv.next() {
                    Some(name) => name,
                    None => bail!("--lang 缺少取值 / --lang needs a value"),
                };
                args.language = match Language::from_wire(&name) {
                    Some(language) => Some(language),
                    None => bail!("未知的 --lang 取值 / unknown --lang value: {name}"),
                };
            }
            "--source" => {
                let name = match argv.next() {
                    Some(name) => name,
                    None => bail!("--source 缺少取值 / --source needs a value"),
                };
                args.source = match SourceMode::from_wire(&name) {
                    Some(mode) => mode,
                    None => bail!("未知的 --source 取值 / unknown --source value: {name}"),
                };
            }
            "--help" | "-h" => args.help = true,
            other => bail!("未知参数 / unknown argument: {other}"),
        }
    }
    Ok(args)
}

/// One state root (its read face open or unopened), its view model when open,
/// the chosen language, and the frame read in flight.
struct App {
    source: Session,
    /// `None` while the read face is unopened: there is no page to model.
    model: Option<RefCell<ViewModel>>,
    labels: &'static Labels,
    /// The module option list as the box currently shows it, and what each
    /// entry after the first one means. Rebuilt from every page.
    module_options: RefCell<Vec<SharedString>>,
    module_choices: RefCell<Vec<OriginModule>>,
    /// Instance identity by ADB port, read once at the pinned snapshot.
    port_map: PortMap,
    /// The port option list, fixed for the session like the map it comes
    /// from, and the port each entry after the first one means. Empty
    /// choices mean the box is disabled and its one item says why.
    port_options: Vec<SharedString>,
    port_choices: Vec<u16>,
    /// The frame request the pane has made, if any. Shared with the read's
    /// completion, which runs on the event loop but has to be `Send`.
    frame: Arc<Mutex<Option<FrameRequest>>>,
    /// Bumped per request so a slow read can never paint a stale frame, and so
    /// a superseded read can see that it has been superseded.
    generation: Arc<AtomicU64>,
    /// One material worker at a time.
    worker: Arc<Mutex<()>>,
    /// The state root and the two launcher paths, as this run resolved them.
    launcher: launcher::Launcher,
    /// Runs the follow ticks while "follow latest" is on.
    follow: Timer,
    /// Reloads once the time cursor's handle rests.
    cursor_rest: Timer,
}

/// The port map this session filters by: the offline read face derives it
/// once from the binding facts through the pinned snapshot.
enum PortMap {
    /// Online: the Runtime does not derive bindings through its page.
    Online,
    /// The read face is unopened: nothing was read.
    Unopened,
    /// The read face refused, with its typed code as the outermost context.
    Failed(anyhow::Error),
    Read(PortBindings),
}

impl App {
    fn port_bindings(&self) -> Option<&PortBindings> {
        match &self.port_map {
            PortMap::Read(bindings) => Some(bindings),
            PortMap::Online | PortMap::Unopened | PortMap::Failed(_) => None,
        }
    }

    fn port_entry(&self, port: u16) -> Option<&PortEntry> {
        self.port_bindings()?.ports.iter().find(|entry| entry.port == port)
    }

    fn frame_request(&self) -> MutexGuard<'_, Option<FrameRequest>> {
        self.frame.lock().unwrap_or_else(|held| held.into_inner())
    }

    /// The pixel size the current request's verified frame decoded to, when
    /// that request was made for this event.
    fn decoded_size(&self, sequence: u64) -> Option<(f32, f32)> {
        let request = (*self.frame_request())?;
        let (width, height) = request.decoded.filter(|_| request.sequence == sequence)?;
        Some((width as f32, height as f32))
    }
}

/// One frame read: the event it is for and its generation, and once that read
/// is verified and decoded, the pixel size. A switch, a clear or a failure
/// drops the size with the request it belongs to.
#[derive(Clone, Copy)]
struct FrameRequest {
    sequence: u64,
    generation: u64,
    decoded: Option<(u32, u32)>,
}

fn main() -> Result<()> {
    let args = parse_args()?;
    let stored = settings::load();
    let language = args.language.unwrap_or(stored.language);
    let labels = language.labels();
    if args.help {
        println!("{}", labels.usage);
        return Ok(());
    }
    // `--state-root` is this run's; the file's `state_root` stands in for it.
    // Neither is a silent default: with both missing the usage is the answer.
    let state_root = match args.state_root {
        Some(root) => root,
        None => match stored.state_root.clone() {
            Some(root) if root.is_absolute() => root,
            Some(root) => bail!(
                "acui.toml 的 state_root 不是绝对路径 / state_root in acui.toml is not absolute: {}",
                root.display()
            ),
            None => bail!("{}", labels.usage),
        },
    };

    let source = Session::open(&state_root, args.source)?;
    let (model, port_map) = match &source {
        Session::Open(source) => {
            let port_map = match source.instance_bindings() {
                Ok(Some(bindings)) => PortMap::Read(bindings),
                Ok(None) => PortMap::Online,
                Err(error) => PortMap::Failed(error),
            };
            let span = time_span(source);
            let mut model = ViewModel::new(
                source.open_report(),
                source.snapshot_position(),
                state_root.display().to_string(),
                span,
            );
            model.tab = args.tab.unwrap_or(LedgerView::Events);
            (Some(RefCell::new(model)), port_map)
        }
        // Nothing is asked of a face that is not open.
        Session::Unopened { .. } => (None, PortMap::Unopened),
    };
    let (port_options, port_choices) = port_options(labels, &port_map);
    let app = Rc::new(App {
        source,
        model,
        labels,
        module_options: RefCell::new(Vec::new()),
        module_choices: RefCell::new(Vec::new()),
        port_map,
        port_options,
        port_choices,
        frame: Arc::new(Mutex::new(None)),
        generation: Arc::new(AtomicU64::new(0)),
        worker: Arc::new(Mutex::new(())),
        launcher: launcher::Launcher::new(
            state_root,
            stored.actingd_config.clone(),
            stored.actingd_exe.clone(),
        ),
        follow: Timer::default(),
        cursor_rest: Timer::default(),
    });
    reload(&app, false);

    let window = AppWindow::new()?;
    install_strings(&window, labels, matches!(app.source, Session::Open(ReadSource::Online(_))));
    window.global::<Scale>().set_factor(stored.text_size.factor());
    window.set_text_size_index(stored.text_size.index());
    window.set_language_index(if language == Language::Zh { 0 } else { 1 });
    install_callbacks(&window, &app);
    launcher::install(&window, &app);
    let config_window = ConfigWindow::new()?;
    instances::install(&window, &config_window, &app);
    refresh(&window, &app);
    launcher::refresh_status(&window, &app);
    window.run()?;
    Ok(())
}

/// Fills the `Strings` global once. There is no second call: the language
/// switch takes effect when the program is started again.
fn install_strings(window: &AppWindow, labels: &'static Labels, online: bool) {
    let global = window.global::<Strings>();
    global.set_window_title(labels.window_title.into());
    global.set_data_source(
        if online { labels.data_source_online } else { labels.data_source }.into(),
    );
    global.set_ledger_dir(labels.ledger_dir.into());
    global.set_storage_format(labels.storage_format.into());
    global.set_show_up_to(labels.show_up_to.into());
    global.set_text_size(labels.text_size.into());
    global.set_language(labels.language.into());
    global.set_restart_note(labels.restart_note.into());
    global.set_text_size_options(shared(&labels.text_sizes));
    global.set_language_options(shared(&labels.languages));
    global.set_severity_min_options(shared(&labels.severity_min));
    global.set_severity_max_options(shared(&labels.severity_max));
    global.set_id_placeholder(labels.id_placeholder.into());
    global.set_continue_reading(labels.continue_reading.into());
    global.set_show_performance(labels.show_performance.into());
    global.set_jump_latest(labels.jump_latest.into());
    global.set_follow_latest(labels.follow_latest.into());
    global.set_card_title(labels.card_title.into());
    global.set_card_note(labels.card_note.into());
    global.set_detail_title(labels.detail_title.into());
    global.set_artifacts_title(labels.artifacts_title.into());
    global.set_overlays_title(labels.overlays_title.into());
    global.set_raw_data_title(labels.raw_data_title.into());
    global.set_col_seq(labels.columns[0].into());
    global.set_col_time(labels.columns[1].into());
    global.set_col_level(labels.columns[2].into());
    global.set_col_module(labels.columns[3].into());
    global.set_col_event(labels.columns[4].into());
    global.set_col_link(labels.columns[5].into());
    global.set_col_port(labels.columns[6].into());
    global.set_launcher_title(labels.launcher_title.into());
    global.set_unlock_owner(labels.unlock_owner.into());
    global.set_unlock_statement(labels.unlock_statement.into());
    global.set_unlock_confirm(labels.unlock_confirm.into());
    global.set_start(labels.start.into());
    global.set_request_shutdown(labels.request_shutdown.into());
}

/// The port box's items: the first means every instance, then one per port,
/// ascending, naming the port and the member its latest binding is for, with
/// ` +n` for the further members of the same port. A map with nothing to
/// pick — no binding facts, facts that name no port, online, an unopened
/// face, or a refused read — gives one item that says which, and no choices,
/// so the box is disabled.
fn port_options(labels: &Labels, port_map: &PortMap) -> (Vec<SharedString>, Vec<u16>) {
    let bindings = match port_map {
        PortMap::Read(bindings) if !bindings.ports.is_empty() => bindings,
        PortMap::Read(bindings) if !bindings.unported.is_empty() => {
            return (vec![labels.all_instances.into()], Vec::new())
        }
        PortMap::Read(_) => return (vec![labels.no_binding_records.into()], Vec::new()),
        PortMap::Online => return (vec![labels.port_online_unsupported.into()], Vec::new()),
        PortMap::Unopened => return (vec![labels.ledger_unopened.into()], Vec::new()),
        PortMap::Failed(error) => return (vec![error.to_string().into()], Vec::new()),
    };
    let mut options = vec![SharedString::from(labels.all_instances)];
    options.extend(bindings.ports.iter().map(|entry| {
        let mut text = fill(
            labels.port_option,
            &[&entry.port.to_string(), &short_id(&code(&entry.latest_instance_id))],
        );
        if entry.members.len() > 1 {
            text.push_str(&format!(" +{}", entry.members.len() - 1));
        }
        SharedString::from(text)
    }));
    (options, bindings.ports.iter().map(|entry| entry.port).collect())
}

/// The committed time span, read as the first and the last event of the snapshot.
fn time_span(source: &ReadSource) -> Option<(u64, u64)> {
    let events = EventQuery { view: Some(LedgerView::Events), ..EventQuery::default() };
    let first = source.query(&events, 1, None).ok()?;
    let last = source
        .query(
            &EventQuery { from_sequence: Some(source.snapshot_position()), ..events },
            1,
            None,
        )
        .ok()?;
    Some((
        first.events().first()?.timestamp_unix_ms,
        last.events().first()?.timestamp_unix_ms,
    ))
}

/// How long one fill may keep reading windows after the first.
const FILL_BUDGET: Duration = Duration::from_secs(1);
/// Windows of slack above a time bound's estimated position, and the most
/// probes that check it.
const TIME_SLACK_WINDOWS: u64 = 2;
const TIME_PROBES: usize = 3;
/// How long the time cursor's handle must rest before the view reloads.
const CURSOR_REST: Duration = Duration::from_millis(400);

/// Fills the timeline from the latest end at the pinned snapshot: backward
/// windows, each read whole, until `FILL_ROWS` more rows show, position 1 is
/// read, `FILL_WINDOWS` windows were read, or `FILL_BUDGET` has passed — at
/// least one window each time. Every page query costs the Runtime a read and
/// verification of the whole ledger (online, on its writer), so what the
/// budget leaves is for "read earlier"; the top bar states what was read.
/// `earlier` continues below what is loaded; otherwise the view starts over
/// from the pin, or from where a time bound is estimated to fall.
fn reload(app: &Rc<App>, earlier: bool) {
    let (Session::Open(source), Some(model)) = (&app.source, &app.model) else {
        return;
    };
    let mut model = model.borrow_mut();
    if !earlier {
        // This reload applies the bound the time cursor holds now.
        app.cursor_rest.stop();
        let snapshot = source.snapshot_position();
        let upper = match model.filters.to_timestamp_unix_ms {
            None => Ok(snapshot),
            Some(bound) => {
                let estimate = upper_for_time(model.span(), snapshot, bound);
                checked_upper(source, &model, estimate, snapshot)
            }
        };
        *app.frame_request() = None;
        match upper {
            Ok(upper) => model.begin(upper),
            Err(error) => {
                model.begin(0);
                model.query_error = Some(error);
                return;
            }
        }
    }
    let (target, started) = (model.row_count() + FILL_ROWS, Instant::now());
    model.query_error = None;
    for read in 0..FILL_WINDOWS {
        if read > 0 && (model.row_count() >= target || started.elapsed() >= FILL_BUDGET) {
            break;
        }
        let Some(window) = model.next_window() else {
            break;
        };
        let pages = model
            .window_query(window)
            .and_then(|query| {
                read_window(source, &query)
                    .map_err(|error| QueryError::ReadFailed(error.to_string()))
            });
        match pages {
            Ok(pages) => model.apply_window(window, &pages),
            Err(error) => {
                model.query_error = Some(error);
                break;
            }
        }
    }
}

/// One window read whole: its pages, following the cursor when the reply's
/// byte limit splits it.
fn read_window(source: &ReadSource, query: &EventQuery) -> Result<Vec<RuntimeEventQueryPage>> {
    let mut pages = Vec::new();
    let mut cursor = None;
    loop {
        let page = source.query(query, MAX_RUNTIME_EVENT_QUERY_EVENTS, cursor)?;
        cursor = page.next_cursor().cloned();
        pages.push(page);
        if cursor.is_none() {
            return Ok(pages);
        }
    }
}

/// Where a time bound is estimated to fall, from the committed span without any
/// read: the position it would take if events were even in time, plus
/// `TIME_SLACK_WINDOWS` windows, capped at the pin; 0 for a bound before the
/// first event. `checked_upper` makes the estimate safe.
fn upper_for_time(span: Option<(u64, u64)>, snapshot: u64, bound: u64) -> u64 {
    let Some((first, last)) = span else {
        return snapshot;
    };
    if bound >= last {
        return snapshot;
    }
    if bound < first {
        return 0;
    }
    let ratio = (bound - first) as f64 / (last - first) as f64;
    let estimate = (ratio * snapshot as f64) as u64;
    (estimate + TIME_SLACK_WINDOWS * WINDOW).min(snapshot)
}

/// Makes an estimated start safe, since reading only goes down from it: one
/// probe with the view's own query — its time bound included — asks for the
/// first matching event above the start. None: nothing above could show, and
/// reading starts there. One found: the start moves above it by a step that
/// doubles each time, for at most `TIME_PROBES` probes and within the fill
/// budget; past either, the pin, which is always safe.
fn checked_upper(
    source: &ReadSource,
    model: &ViewModel,
    estimate: u64,
    snapshot: u64,
) -> Result<u64, QueryError> {
    let (mut upper, mut step, started) = (estimate, TIME_SLACK_WINDOWS * WINDOW, Instant::now());
    for probe in 0..TIME_PROBES {
        if upper >= snapshot || (probe > 0 && started.elapsed() >= FILL_BUDGET) {
            return Ok(snapshot);
        }
        let mut query = model.query()?;
        query.from_sequence = Some(upper + 1);
        query
            .validate()
            .map_err(|error| QueryError::Rejected(error.code().to_string()))?;
        let page = source
            .query(&query, 1, None)
            .map_err(|error| QueryError::ReadFailed(error.to_string()))?;
        let Some(found) = page.events().first() else {
            return Ok(upper);
        };
        upper = found.sequence + step;
        step *= 2;
    }
    Ok(snapshot)
}

fn install_callbacks(window: &AppWindow, app: &Rc<App>) {
    macro_rules! on {
        ($setter:ident, |$app:ident, $model:ident, $value:ident| $body:expr) => {{
            let weak = window.as_weak();
            let $app = Rc::clone(app);
            window.$setter(move |$value| {
                // Unopened there is no model, and every control that calls these is off.
                let Some($model) = &$app.model else {
                    return;
                };
                $body;
                if let Some(window) = weak.upgrade() {
                    refresh(&window, &$app);
                }
            });
        }};
    }

    on!(on_tab_changed, |app, model, index| {
        model.borrow_mut().tab = ALL_TABS[index.clamp(0, ALL_TABS.len() as i32 - 1) as usize];
        reload(&app, false)
    });
    on!(on_minimum_severity_changed, |app, model, index| {
        model.borrow_mut().filters.minimum_severity = match index {
            1 => Some(EventSeverity::Warning),
            2 => Some(EventSeverity::Error),
            3 => Some(EventSeverity::Fatal),
            _ => None,
        };
        reload(&app, false)
    });
    on!(on_maximum_severity_changed, |app, model, index| {
        model.borrow_mut().filters.maximum_severity = match index {
            1 => Some(EventSeverity::Info),
            2 => Some(EventSeverity::Warning),
            3 => Some(EventSeverity::Error),
            _ => None,
        };
        reload(&app, false)
    });
    // By name against the list as it stands now, never by a position taken from
    // a list that the next page may have rebuilt.
    on!(on_module_changed, |app, model, name| {
        let module = app
            .module_options
            .borrow()
            .iter()
            .position(|option| *option == name)
            .and_then(|index| index.checked_sub(1))
            .and_then(|index| app.module_choices.borrow().get(index).copied());
        model.borrow_mut().filters.origin_module = module;
        reload(&app, false)
    });
    // A port is one instance: its whole set of ids goes into the query, or
    // nothing does. The first item, and any name not on the list, clears both.
    on!(on_port_changed, |app, model, name| {
        let entry = app
            .port_options
            .iter()
            .position(|option| *option == name)
            .and_then(|index| index.checked_sub(1))
            .and_then(|index| app.port_choices.get(index))
            .and_then(|port| app.port_entry(*port));
        {
            let mut model = model.borrow_mut();
            model.filters.port = entry.map(|entry| entry.port);
            model.filters.instance_ids =
                entry.map(|entry| entry.members.clone()).unwrap_or_default();
        }
        reload(&app, false)
    });
    // Shown or hidden again from the pin: the performance monitor's events are
    // dropped as each window is read, not from rows already held.
    on!(on_show_performance_changed, |app, model, shown| {
        model.borrow_mut().filters.show_performance = shown;
        reload(&app, false)
    });
    on!(on_id_changed, |app, model, text| {
        model.borrow_mut().filters.id_text = text.to_string();
        reload(&app, false)
    });
    // The bound and its label follow the handle at once; the reload waits
    // until the handle rests, since each one reads the whole ledger.
    {
        let weak = window.as_weak();
        let app = Rc::clone(app);
        window.on_cursor_changed(move |value| {
            let Some(model) = &app.model else {
                return;
            };
            let bound = model.borrow().span().and_then(|(from, to)| {
                let ratio = (value as f64 / 1000.0).clamp(0.0, 1.0);
                (ratio < 1.0).then(|| from + ((to - from) as f64 * ratio) as u64)
            });
            model.borrow_mut().filters.to_timestamp_unix_ms = bound;
            // A time bound reads down from the past; following stops at once.
            if bound.is_some() {
                app.follow.stop();
                if let Some(window) = weak.upgrade() {
                    window.set_following(false);
                }
            }
            let (rested, held) = (weak.clone(), Rc::downgrade(&app));
            app.cursor_rest.start(TimerMode::SingleShot, CURSOR_REST, move || {
                if let (Some(window), Some(app)) = (rested.upgrade(), held.upgrade()) {
                    reload(&app, false);
                    refresh(&window, &app);
                }
            });
            if let Some(window) = weak.upgrade() {
                refresh(&window, &app);
            }
        });
    }
    on!(on_row_clicked, |app, model, index| {
        let mut model = model.borrow_mut();
        let selected = match model.display_rows().get(index.max(0) as usize) {
            Some(DisplayRow::Event { event, .. }) => Some(event.sequence),
            _ => None,
        };
        if selected.is_some() {
            model.selected_sequence = selected;
        }
    });
    {
        let weak = window.as_weak();
        let app = Rc::clone(app);
        window.on_jump_latest(move || {
            if let Some(window) = weak.upgrade() {
                jump_latest(&window, &app);
            }
        });
    }
    {
        let weak = window.as_weak();
        let app = Rc::clone(app);
        window.on_follow_changed(move |on| {
            if let Some(window) = weak.upgrade() {
                set_following(&window, &app, on);
            }
        });
    }
    {
        let weak = window.as_weak();
        let app = Rc::clone(app);
        window.on_load_more(move || {
            // A bound the time cursor set but has not applied yet goes first:
            // no window of it is appended below rows read under the old one.
            let pending = app.cursor_rest.running();
            reload(&app, !pending);
            if let Some(window) = weak.upgrade() {
                refresh(&window, &app);
            }
        });
    }
    {
        // Text size applies now, through the one scale property. Only the text
        // size is written: the language is the file's own, or a `--lang`
        // override that this run must not save over it.
        let weak = window.as_weak();
        window.on_text_size_changed(move |index| {
            let size = TextSize::from_index(index);
            settings::save_text_size(size);
            if let Some(window) = weak.upgrade() {
                window.global::<Scale>().set_factor(size.factor());
            }
        });
    }
    // Language is written now and read at the next start; the note beside the
    // box says so.
    window.on_language_changed(move |index| {
        settings::save_language(if index == 1 { Language::En } else { Language::Zh });
    });
}

/// How often "follow latest" asks where the ledger is: one fact snapshot,
/// which reads no ledger and writes nothing. A page query follows only when the
/// ledger moved; each one costs the Runtime a read of the whole ledger.
const FOLLOW_INTERVAL: Duration = Duration::from_secs(5);

/// A person's jump to the latest: the pin moves to the Runtime's latest
/// position, status and facts are read again — the status read leaves one
/// observation event in the ledger — and the view starts over from the new
/// pin, with no time bound.
fn jump_latest(window: &AppWindow, app: &Rc<App>) {
    let (Session::Open(source), Some(model)) = (&app.source, &app.model) else {
        return;
    };
    match source.repin() {
        Err(error) => {
            model.borrow_mut().query_error = Some(QueryError::ReadFailed(error.to_string()));
        }
        Ok(_) => {
            source.refresh_instances();
            {
                let mut model = model.borrow_mut();
                model.set_pin(source.open_report(), source.snapshot_position());
                model.set_span(time_span(source));
                model.filters.to_timestamp_unix_ms = None;
            }
            window.set_cursor_value(1000.0);
            reload(app, false);
        }
    }
    refresh(window, app);
}

/// Turning following on catches up like a jump to the latest, without the
/// status read: the pin moves to the latest position, any time bound is
/// dropped and the view starts over from the new pin, however far the ledger
/// has moved since. Then a tick runs every `FOLLOW_INTERVAL`. Turning it off
/// stops the ticks and leaves the view as it is.
fn set_following(window: &AppWindow, app: &Rc<App>, on: bool) {
    if !on {
        app.follow.stop();
        return;
    }
    let (Session::Open(source), Some(model)) = (&app.source, &app.model) else {
        return;
    };
    match source.repin() {
        Err(error) => {
            model.borrow_mut().query_error = Some(QueryError::ReadFailed(error.to_string()));
            window.set_following(false);
            refresh(window, app);
            return;
        }
        Ok(_) => {
            {
                let mut model = model.borrow_mut();
                model.set_pin(source.open_report(), source.snapshot_position());
                model.set_span(time_span(source));
                model.filters.to_timestamp_unix_ms = None;
            }
            window.set_cursor_value(1000.0);
            reload(app, false);
            refresh(window, app);
        }
    }
    let (weak, held) = (window.as_weak(), Rc::downgrade(app));
    app.follow.start(TimerMode::Repeated, FOLLOW_INTERVAL, move || {
        if let (Some(window), Some(app)) = (weak.upgrade(), held.upgrade()) {
            follow_tick(&window, &app);
        }
    });
}

/// One follow tick. One fact snapshot says where the ledger is and brings the
/// task facts; only when the pin moved, or an earlier tick left windows unread,
/// are the newer windows read onto the top of the view. No status is read and
/// nothing is written to the ledger. A time bound set meanwhile stops the
/// following.
fn follow_tick(window: &AppWindow, app: &Rc<App>) {
    let (Session::Open(source), Some(model)) = (&app.source, &app.model) else {
        return;
    };
    if model.borrow().filters.to_timestamp_unix_ms.is_some() {
        app.follow.stop();
        window.set_following(false);
        return;
    }
    let moved = match source.poll() {
        Ok(moved) => moved,
        Err(error) => {
            model.borrow_mut().query_error = Some(QueryError::ReadFailed(error.to_string()));
            refresh(window, app);
            return;
        }
    };
    {
        let mut model = model.borrow_mut();
        if moved {
            model.set_pin(source.open_report(), source.snapshot_position());
        }
        if !moved && model.next_newer_window().is_none() {
            return;
        }
        model.query_error = None;
        let started = Instant::now();
        for read in 0..FILL_WINDOWS {
            if read > 0 && started.elapsed() >= FILL_BUDGET {
                break;
            }
            let Some(window) = model.next_newer_window() else {
                break;
            };
            let pages = model.window_query(window).and_then(|query| {
                read_window(source, &query)
                    .map_err(|error| QueryError::ReadFailed(error.to_string()))
            });
            match pages {
                Ok(pages) => model.apply_newer_window(window, &pages),
                Err(error) => {
                    model.query_error = Some(error);
                    break;
                }
            }
        }
    }
    refresh(window, app);
}

fn refresh(window: &AppWindow, app: &Rc<App>) {
    let labels = app.labels;
    let (Session::Open(source), Some(model)) = (&app.source, &app.model) else {
        paint_unopened(window, app);
        return;
    };
    let model = model.borrow();
    let card = model.instance_card(app.port_bindings());

    window.set_ledger_dir_value(card.state_root.as_str().into());
    window.set_storage_format_value(backend_text(labels, &card.backend).into());
    let scope = model.read_scope();
    window.set_read_up_to_text(
        match scope.range {
            Some((low, high)) => {
                let mut text = fill(labels.read_range, &[&low.to_string(), &high.to_string()]);
                if !scope.read_complete {
                    text.push_str(" · ");
                    text.push_str(labels.source_incomplete);
                }
                text
            }
            None => labels.read_up_to_unknown.to_string(),
        }
        .into(),
    );
    let mut loaded = fill(labels.loaded_rows_top, &[&card.loaded_count.to_string()]);
    if model.hidden_performance() > 0 {
        let hidden = model.hidden_performance().to_string();
        loaded.push_str(&fill(labels.performance_hidden, &[&hidden]));
    }
    window.set_loaded_rows_text(loaded.into());
    window.set_show_performance(model.filters.show_performance);
    window.set_follow_available(source.follows());
    window.set_cursor_label(
        model
            .filters
            .to_timestamp_unix_ms
            .map(|bound| format!("≤ {}", format_full(bound)))
            .unwrap_or_else(|| labels.all.to_string())
            .into(),
    );

    let mut lines = vec![
        // Which face answered, and for an `auto` pick, why: never a silent choice.
        field(
            labels,
            "source",
            source_text(labels, &app.source),
            if app.source.offline_reason().is_some() { "offline" } else { "online" },
        ),
        field(labels, "latest_sequence", card.latest_sequence.to_string(), "latest_sequence"),
        field(labels, "event_count", count_text(labels, card.event_count), "event_count"),
        field(labels, "loaded_rows", card.loaded_count.to_string(), ""),
        field(labels, "first_event", stamp(labels, card.first_timestamp_unix_ms), ""),
        field(labels, "latest_event", stamp(labels, card.last_timestamp_unix_ms), ""),
        field(
            labels,
            "age",
            card.last_timestamp_unix_ms
                .map(|stamp| age_text(labels, stamp))
                .unwrap_or_else(|| labels.none.to_string()),
            "",
        ),
        field(labels, "integrity", integrity_text(labels, &card), ""),
        field(labels, "repair_count", count_text(labels, card.repair_count), "repair_count"),
        field(labels, "writer", writer_text(labels, &card.writer), ""),
    ];
    // The port map could not be read: the code, and behind it the read
    // face's own words. The box beside the list carries the same code.
    if let PortMap::Failed(error) = &app.port_map {
        lines.push(field(
            labels,
            "port_bindings",
            fill(labels.read_failed, &[&error.to_string()]),
            error.root_cause().to_string(),
        ));
    }
    lines.extend(instance_lines(labels, source));
    // The picked port: its latest binding's facts and the size of its set.
    // The counts below still come from the re-queried page.
    if let Some(entry) = &card.port {
        lines.push(field(
            labels,
            "instance_alias",
            entry.latest_alias.clone(),
            code(&entry.latest_instance_id),
        ));
        lines.push(field(labels, "adb_port", entry.port.to_string(), ""));
        lines.push(field(
            labels,
            "provenance",
            labels.provenance(&entry.latest_provenance).to_string(),
            entry.latest_provenance.clone(),
        ));
        lines.push(field(labels, "bound_ids", entry.members.len().to_string(), ""));
        lines.push(field(labels, "latest_binding", entry.latest_sequence.to_string(), ""));
    }
    for (severity, count) in &card.severity_counts {
        lines.push(FieldLine {
            label: labels.levels[severity_index(*severity)].into(),
            value: count.to_string().into(),
            raw: severity.as_str().into(),
            wrap: false,
        });
    }
    // Last line of the card, and the only one that may run to several lines.
    lines.push(FieldLine {
        label: labels.field("modules").into(),
        value: card
            .modules
            .iter()
            .map(|module| module_text(labels, *module))
            .collect::<Vec<_>>()
            .join(" · ")
            .into(),
        raw: SharedString::new(),
        wrap: true,
    });
    window.set_card_lines(models(lines));

    // Only the tab a person is looking at carries a number, and the number is
    // the rows this view has loaded — never a count taken from another view.
    let active = ALL_TABS.iter().position(|tab| *tab == model.tab).unwrap_or(0);
    window.set_tab_labels(models(
        labels
            .tabs
            .iter()
            .enumerate()
            .map(|(index, name)| {
                if index == active {
                    let badge = fill(labels.loaded_badge, &[&card.loaded_count.to_string()]);
                    SharedString::from(format!("{name} · {badge}"))
                } else {
                    SharedString::from(*name)
                }
            })
            .collect::<Vec<_>>(),
    ));
    window.set_active_tab(active as i32);

    sync_modules(window, app, &model.filters.origin_module.clone(), card.modules);
    sync_ports(window, app, model.filters.port);

    let display = model.display_rows();
    window.set_filter_error(query_error_text(labels, model.query_error.as_ref()).into());
    window.set_has_more(model.has_earlier());
    let rows: Vec<RowItem> = display
        .iter()
        .map(|row| match row {
            DisplayRow::Recovery(group) => RowItem {
                recovery: true,
                note: recovery_text(labels, group).into(),
                ..RowItem::default()
            },
            DisplayRow::Event { event, folded } => {
                let raw_type = code(&event.event_type);
                RowItem {
                    recovery: false,
                    folded: *folded,
                    sequence_text: event.sequence.to_string().into(),
                    clock: format_clock(event.timestamp_unix_ms).into(),
                    level: labels.levels[severity_index(event.severity)].into(),
                    level_tone: severity_tone(event.severity),
                    module: module_text(labels, event.origin.module()).into(),
                    event_name: event_text(labels, &raw_type).into(),
                    event_raw: raw_type.into(),
                    // The port the map gives the linked instance; the id itself
                    // when the map gives none — never a port the ledger did not state.
                    port: match event.links.instance_id() {
                        None => labels.host.to_string(),
                        Some(id) => match app.port_bindings().and_then(|map| map.port_of.get(id)) {
                            Some(port) => port.to_string(),
                            None => short_id(&code(id)),
                        },
                    }
                    .into(),
                    note: if *folded { labels.recovered_badge } else { "" }.into(),
                    link_id: links_named(&event.links)
                        .first()
                        .map(|(_, value)| short_id(value))
                        .unwrap_or_default()
                        .into(),
                }
            }
        })
        .collect();
    window.set_rows(models(rows));
    window.set_selected_index(
        display
            .iter()
            .position(|row| match row {
                DisplayRow::Event { event, .. } => {
                    Some(event.sequence) == model.selected_sequence
                }
                DisplayRow::Recovery(_) => false,
            })
            .map(|index| index as i32)
            .unwrap_or(-1),
    );

    match model.detail() {
        Some(detail) => {
            window.set_detail_lines(models(detail_lines(labels, detail.event)));
            window.set_artifacts(models(
                detail
                    .event
                    .artifacts
                    .iter()
                    .map(|artifact| {
                        let kind = code(&artifact.kind);
                        let media = code(&artifact.media_type);
                        ArtifactItem {
                            artifact_id: code(&artifact.artifact_id).into(),
                            kind: format!(
                                "{} · {} · {}",
                                labels.artifact_kind(&kind),
                                labels.media_type(&media),
                                format_bytes(artifact.byte_count)
                            )
                            .into(),
                            detail: format!("{kind} · {media} · {}", artifact.byte_count).into(),
                            sha256_label: labels.checksum.into(),
                            sha256: artifact.sha256.as_str().into(),
                        }
                    })
                    .collect::<Vec<_>>(),
            ));
            let target = model.frame_target();
            let sequence = target.as_ref().map(|target| target.event.sequence);
            update_frame(window, app, source, target, detail.frame_size);
            // The size the event states, else what this event's verified frame
            // decoded to, else the overlays' own extent.
            let frame_size = detail
                .frame_size
                .or_else(|| sequence.and_then(|sequence| app.decoded_size(sequence)));
            let (canvas_width, canvas_height) = canvas_size(&detail.overlays, frame_size);
            window.set_canvas_width(canvas_width);
            window.set_canvas_height(canvas_height);
            window.set_frame_size_text(frame_size_text(labels, frame_size).into());
            window.set_overlays(models(
                detail
                    .overlays
                    .iter()
                    .map(|overlay| OverlayItem {
                        kind: overlay.kind.as_str().into(),
                        x: overlay.x,
                        y: overlay.y,
                        width: overlay.width,
                        height: overlay.height,
                        point: overlay.is_point(),
                    })
                    .collect::<Vec<_>>(),
            ));
            window.set_payload_json(detail.pretty_payload_json.into());
        }
        None => {
            window.set_detail_lines(models(vec![FieldLine {
                label: labels.not_selected.into(),
                value: SharedString::new(),
                raw: SharedString::new(),
                wrap: false,
            }]));
            window.set_artifacts(models(Vec::<ArtifactItem>::new()));
            window.set_overlays(models(Vec::<OverlayItem>::new()));
            window.set_canvas_width(1.0);
            window.set_canvas_height(1.0);
            window.set_frame_size_text(SharedString::new());
            window.set_payload_json(SharedString::new());
            clear_frame(window, app, labels.no_event_selected.to_string());
        }
    }
}

/// The window over an unopened read face, painted once: nothing was read, so
/// the card has the read-face line only, and the list, the boxes and the frame
/// pane say the ledger is not opened instead of standing empty, which would
/// read as an empty ledger. The controls that would query it are off.
fn paint_unopened(window: &AppWindow, app: &Rc<App>) {
    let labels = app.labels;
    window.set_ledger_dir_value(app.source.state_root().display().to_string().into());
    window.set_storage_format_value(labels.none.into());
    window.set_read_up_to_text(labels.ledger_unopened.into());
    // The card's only line, and so the one that may wrap.
    window.set_card_lines(models(vec![FieldLine {
        label: labels.field("source").into(),
        value: source_text(labels, &app.source).into(),
        raw: "offline".into(),
        wrap: true,
    }]));
    window.set_tab_labels(shared(&labels.tabs));
    window.set_active_tab(-1);
    window.set_module_options(models(vec![SharedString::from(labels.ledger_unopened)]));
    sync_ports(window, app, None);
    window.set_unopened_note(labels.list_unopened.into());
    window.set_frame_note(labels.list_unopened.into());
}

/// Rebuilds the module option list from the loaded rows and puts the box back
/// on the module it is filtering by, found by name in the list that now exists.
fn sync_modules(
    window: &AppWindow,
    app: &Rc<App>,
    picked: &Option<OriginModule>,
    mut choices: Vec<OriginModule>,
) {
    // A picked module stays on the list even when the loaded rows show none of it,
    // so the box never displays a module the console is not filtering by.
    if let Some(module) = picked {
        if !choices.contains(module) {
            choices.push(*module);
            choices.sort();
        }
    }
    let mut options = vec![SharedString::from(app.labels.all_modules)];
    options.extend(
        choices
            .iter()
            .map(|module| SharedString::from(module_text(app.labels, *module))),
    );
    let index = picked
        .and_then(|module| choices.iter().position(|option| *option == module))
        .map(|index| index as i32 + 1)
        .unwrap_or(0);
    *app.module_choices.borrow_mut() = choices;
    *app.module_options.borrow_mut() = options.clone();
    window.set_module_options(models(options));
    window.set_module_index(index);
}

/// Puts the port box back on the port the console is filtering by, found in
/// the session's fixed list; the first item when there is none.
fn sync_ports(window: &AppWindow, app: &Rc<App>, picked: Option<u16>) {
    let index = picked
        .and_then(|port| app.port_choices.iter().position(|choice| *choice == port))
        .map(|index| index as i32 + 1)
        .unwrap_or(0);
    window.set_port_options(models(app.port_options.clone()));
    window.set_port_enabled(!app.port_choices.is_empty());
    window.set_port_index(index);
}

fn detail_lines(labels: &Labels, event: &ProjectedEvent) -> Vec<FieldLine> {
    let raw_type = code(&event.event_type);
    let raw_source = code(&event.origin.source());
    let raw_module = event.origin.module().as_str();
    let mut lines = vec![
        field(labels, "sequence", event.sequence.to_string(), ""),
        field(labels, "event_id", code(&event.event_id), ""),
        field(labels, "occurred_at", format_full(event.timestamp_unix_ms), ""),
        field(labels, "event_type", event_text(labels, &raw_type), raw_type.clone()),
        field(
            labels,
            "severity",
            labels.levels[severity_index(event.severity)].to_string(),
            event.severity.as_str(),
        ),
        field(
            labels,
            "sensitivity",
            labels.sensitivities[sensitivity_index(event.sensitivity)].to_string(),
            code(&event.sensitivity),
        ),
        field(
            labels,
            "origin",
            format!(
                "{} · {} · {}",
                labels.source(&raw_source),
                module_text(labels, event.origin.module()),
                code(&event.origin.actor())
            ),
            format!("{raw_source} · {raw_module}"),
        ),
        field(labels, "views", views_text(labels, event), ""),
        field(labels, "payload_schema", event.payload_schema.clone(), ""),
    ];
    lines.extend(
        links_named(&event.links)
            .into_iter()
            .map(|(key, value)| field(labels, key, value, "")),
    );
    lines
}

/// The frame pane. A frame is read on demand for the selected event only, and
/// only after the ledger says the material was not evicted.
fn update_frame(
    window: &AppWindow,
    app: &Rc<App>,
    source: &ReadSource,
    target: Option<FrameTarget>,
    payload_frame_size: Option<(f32, f32)>,
) {
    let labels = app.labels;
    let Some(target) = target else {
        clear_frame(window, app, labels.frame_no_artifact.to_string());
        return;
    };
    if let Some(eviction) = &target.eviction {
        clear_frame(window, app, eviction_text(labels, eviction));
        return;
    }
    if target.artifact.byte_count > MAX_FRAME_BYTES {
        clear_frame(
            window,
            app,
            fill(
                labels.frame_too_large,
                &[&target.artifact.byte_count.to_string(), &MAX_FRAME_BYTES.to_string()],
            ),
        );
        return;
    }
    let generation = {
        let mut request = app.frame_request();
        if request.is_some_and(|request| request.sequence == target.event.sequence) {
            return;
        }
        let generation = app.generation.fetch_add(1, Ordering::SeqCst) + 1;
        *request =
            Some(FrameRequest { sequence: target.event.sequence, generation, decoded: None });
        generation
    };
    window.set_frame_ready(false);
    window.set_frame_note(
        fill(labels.frame_reading, &[&target.artifact.byte_count.to_string()]).into(),
    );

    let shared = Arc::clone(&app.generation);
    let frame = Arc::clone(&app.frame);
    let worker = Arc::clone(&app.worker);
    let reader = source.material_reader();
    let snapshot = source.snapshot_position();
    let weak = window.as_weak();
    let started = launcher::spawn_worker("acui-frame", move || {
        // One worker at a time. A whole read cannot be stopped midway on either
        // face, so a superseded one runs to its end, bounded by the byte cap and
        // the deadline, and its result is dropped; one queued behind the slot
        // that is superseded by then never starts.
        let _slot = worker.lock().unwrap_or_else(|held| held.into_inner());
        let still_wanted = || shared.load(Ordering::SeqCst) == generation;
        if !still_wanted() {
            return;
        }
        let read = reader.read(target.event, &target.artifact, snapshot, &still_wanted);
        let painted = match read {
            Err(error) => Err(error.to_string()),
            Ok(None) => return,
            Ok(Some(outcome)) => match &outcome.bytes {
                Some(bytes) => decode_png(labels, bytes),
                None => Err(outcome_text(labels, &outcome)),
            },
        };
        let _ = slint::invoke_from_event_loop(move || {
            if shared.load(Ordering::SeqCst) != generation {
                return;
            }
            // The decoded size belongs to this request; a failure leaves it unknown.
            if let Some(request) = frame
                .lock()
                .unwrap_or_else(|held| held.into_inner())
                .as_mut()
                .filter(|request| request.generation == generation)
            {
                request.decoded = painted.as_ref().ok().map(|(width, height, _)| (*width, *height));
            }
            let Some(window) = weak.upgrade() else {
                return;
            };
            match painted {
                Ok((width, height, rgba)) => {
                    let mut buffer = SharedPixelBuffer::<Rgba8Pixel>::new(width, height);
                    buffer.make_mut_bytes().copy_from_slice(&rgba);
                    window.set_frame_image(Image::from_rgba8(buffer));
                    window.set_frame_ready(true);
                    window.set_frame_note(
                        fill(labels.frame_verified, &[&width.to_string(), &height.to_string()])
                            .into(),
                    );
                    // The payload did not state a size; the verified frame does.
                    if payload_frame_size.is_none() {
                        window.set_canvas_width(width as f32);
                        window.set_canvas_height(height as f32);
                        window.set_frame_size_text(
                            fill(labels.frame_size, &[&width.to_string(), &height.to_string()])
                                .into(),
                        );
                    }
                }
                Err(text) => {
                    window.set_frame_ready(false);
                    window.set_frame_note(text.into());
                }
            }
        });
    });
    // No read runs. The request is dropped, so choosing the event again retries.
    if let Err(error) = started {
        clear_frame(window, app, fill(labels.thread_failed, &[&error.to_string()]));
    }
}

fn clear_frame(window: &AppWindow, app: &Rc<App>, note: String) {
    *app.frame_request() = None;
    app.generation.fetch_add(1, Ordering::SeqCst);
    window.set_frame_ready(false);
    window.set_frame_note(note.into());
}

fn decode_png(labels: &Labels, bytes: &[u8]) -> Result<(u32, u32, Vec<u8>), String> {
    let decoded = image::load_from_memory_with_format(bytes, image::ImageFormat::Png)
        .map_err(|error| fill(labels.frame_decode_failed, &[&error.to_string()]))?
        .to_rgba8();
    Ok((decoded.width(), decoded.height(), decoded.into_raw()))
}

fn outcome_text(labels: &Labels, outcome: &MaterialOutcome) -> String {
    let mut text = fill(labels.frame_unavailable, &[&code(&outcome.state)]);
    if let Some(limit) = &outcome.limit {
        text.push_str(&format!(" · {}", code(limit)));
    }
    if let Some(eviction) = &outcome.eviction {
        text.push_str(&format!(" · {}", eviction_text(labels, eviction)));
    }
    if let Some(failure) = &outcome.failure {
        text.push_str(&format!(" · {failure}"));
    }
    text
}

fn eviction_text(labels: &Labels, eviction: &ArtifactEvictionObservation) -> String {
    fill(
        labels.frame_evicted,
        &[
            &eviction
                .disposition
                .as_ref()
                .map(code)
                .unwrap_or_else(|| labels.none.to_string()),
            &eviction.intent.sequence.to_string(),
            &eviction
                .outcome
                .map(|outcome| format!(" → #{}", outcome.sequence))
                .unwrap_or_default(),
            &eviction.through_sequence.to_string(),
        ],
    )
}

fn recovery_text(labels: &Labels, group: &RecoveryGroup) -> String {
    let evidence = group
        .evidence
        .iter()
        .map(|(failure, success)| match success {
            Some(success) => fill(
                labels.failed_recovered,
                &[&failure.to_string(), &success.to_string()],
            ),
            None => fill(labels.failed_only, &[&failure.to_string()]),
        })
        .collect::<Vec<_>>()
        .join(labels.join);
    let mut text = format!(
        "{}{} · {} · {evidence}",
        labels.run_prefix,
        short_id(&group.run_id),
        labels.recovery_states[recovery_index(group.state)]
    );
    if !group.gaps.is_empty() {
        text.push_str(labels.gap_prefix);
        text.push_str(
            &group
                .gaps
                .iter()
                .map(|gap| labels.recovery_gap(&code(gap)).to_string())
                .collect::<Vec<_>>()
                .join(labels.join),
        );
    }
    text
}

fn integrity_text(labels: &Labels, card: &InstanceCard) -> String {
    match (&card.corrupt_tail, card.read_complete) {
        (None, true) => labels.integrity_ok.to_string(),
        (tail, complete) => fill(
            labels.integrity_bad,
            &[&complete.to_string(), tail.as_deref().unwrap_or(labels.none)],
        ),
    }
}

fn count_text(labels: &Labels, count: LedgerCount) -> String {
    match count {
        LedgerCount::Counted(count) => count.to_string(),
        LedgerCount::VerifiedPrefix(count) => {
            fill(labels.count_verified_prefix, &[&count.to_string()])
        }
        LedgerCount::NoRepairLog => labels.count_no_repair_log.to_string(),
        LedgerCount::NotStatedByRuntime => labels.count_not_stated.to_string(),
    }
}

fn writer_text(labels: &Labels, writer: &WriterFacts) -> String {
    match writer {
        WriterFacts::Absent => labels.writer_absent.to_string(),
        WriterFacts::Locked { byte_count } => {
            fill(labels.writer_locked, &[&byte_count.to_string()])
        }
        WriterFacts::Readable { owner_id, pid, active, started_at_unix_ms } => fill(
            if *active { labels.writer_running } else { labels.writer_stopped },
            &[owner_id, &pid.to_string(), &format_full(*started_at_unix_ms)],
        ),
        WriterFacts::Runtime { pid, owner_epoch, started_at_unix_ms } => fill(
            labels.writer_runtime,
            &[&pid.to_string(), owner_epoch, &format_full(*started_at_unix_ms)],
        ),
    }
}

fn source_text(labels: &Labels, source: &Session) -> String {
    let mut text = match source.offline_reason() {
        None => labels.source_online.to_string(),
        Some(OfflineReason::Requested) => labels.source_offline_requested.to_string(),
        Some(OfflineReason::RuntimeInfoAbsent) => labels.source_offline_absent.to_string(),
        Some(OfflineReason::ConnectFailed { code, operation }) => {
            fill(labels.source_offline_failed, &[code, operation])
        }
    };
    // The ledger's own words, verbatim, and its io kind: only `not_found`
    // says the state root has no ledger yet.
    if let Session::Unopened { failure, .. } = source {
        let detail = failure.detail.as_deref().unwrap_or(labels.none);
        text.push_str(" · ");
        text.push_str(&fill(labels.source_unopened, &[failure.code, failure.operation, detail]));
        if let Some(kind) = &failure.io_kind {
            text.push_str(&fill(labels.io_kind_text, &[kind]));
        }
        if failure.code == "ledger_io" && failure.io_kind.as_deref() == Some("not_found") {
            text.push_str(" · ");
            text.push_str(labels.source_unopened_hint);
        }
    }
    text
}

fn age_text(labels: &Labels, timestamp_unix_ms: u64) -> String {
    let seconds = seconds_since(timestamp_unix_ms).max(0);
    let (template, value) = if seconds >= 86_400 {
        (labels.age_days, seconds / 86_400)
    } else if seconds >= 3_600 {
        (labels.age_hours, seconds / 3_600)
    } else if seconds >= 60 {
        (labels.age_minutes, seconds / 60)
    } else {
        (labels.age_seconds, seconds)
    };
    fill(template, &[&value.to_string()])
}

fn frame_size_text(labels: &Labels, frame_size: Option<(f32, f32)>) -> String {
    match frame_size {
        Some((width, height)) => fill(
            labels.frame_size,
            &[&(width as u32).to_string(), &(height as u32).to_string()],
        ),
        None => labels.frame_size_missing.to_string(),
    }
}

fn query_error_text(labels: &Labels, error: Option<&QueryError>) -> String {
    match error {
        None => String::new(),
        Some(QueryError::IdNotCanonical) => labels.filter_id_error.to_string(),
        Some(QueryError::InstanceFilterConflict) => labels.filter_instance_conflict.to_string(),
        Some(QueryError::Rejected(reason)) => fill(labels.filter_rejected, &[reason]),
        Some(QueryError::ReadFailed(reason)) => fill(labels.read_failed, &[reason]),
    }
}

fn backend_text(labels: &Labels, backend: &str) -> String {
    match backend {
        "segment" => labels.storage_segments.to_string(),
        "sqlite" => labels.storage_sqlite.to_string(),
        "runtime" => labels.storage_runtime.to_string(),
        other => other.to_string(),
    }
}

fn module_text(labels: &Labels, module: OriginModule) -> String {
    let raw = module.as_str();
    match module_names(raw) {
        Some((zh, en)) => pick(labels, zh, en).to_string(),
        None => raw.to_string(),
    }
}

fn event_text(labels: &Labels, raw: &str) -> String {
    match event_type_names(raw) {
        Some((zh, en)) => pick(labels, zh, en).to_string(),
        None => raw.to_string(),
    }
}

const fn pick(labels: &Labels, zh: &'static str, en: &'static str) -> &'static str {
    match labels.tongue {
        Language::Zh => zh,
        Language::En => en,
    }
}

fn views_text(labels: &Labels, event: &ProjectedEvent) -> String {
    event
        .views
        .iter()
        .map(|view| {
            ALL_TABS
                .iter()
                .position(|tab| tab == view)
                .map(|index| labels.tabs[index])
                .unwrap_or_default()
        })
        .collect::<Vec<_>>()
        .join(" · ")
}

fn stamp(labels: &Labels, timestamp_unix_ms: Option<u64>) -> String {
    timestamp_unix_ms
        .map(format_full)
        .unwrap_or_else(|| labels.none.to_string())
}

const fn severity_index(severity: EventSeverity) -> usize {
    match severity {
        EventSeverity::Debug => 0,
        EventSeverity::Info => 1,
        EventSeverity::Warning => 2,
        EventSeverity::Error => 3,
        EventSeverity::Fatal => 4,
    }
}

const fn severity_tone(severity: EventSeverity) -> i32 {
    match severity {
        EventSeverity::Debug => 0,
        EventSeverity::Info => 1,
        EventSeverity::Warning => 2,
        EventSeverity::Error | EventSeverity::Fatal => 3,
    }
}

const fn sensitivity_index(sensitivity: Sensitivity) -> usize {
    match sensitivity {
        Sensitivity::Public => 0,
        Sensitivity::Internal => 1,
        Sensitivity::Sensitive => 2,
        Sensitivity::Secret => 3,
    }
}

const fn recovery_index(state: acui_rows::LedgerRecoveryState) -> usize {
    match state {
        acui_rows::LedgerRecoveryState::Recovered => 0,
        acui_rows::LedgerRecoveryState::Unresolved => 1,
        acui_rows::LedgerRecoveryState::Unknown => 2,
    }
}

/// The coordinate space the overlays and the frame share.
fn canvas_size(overlays: &[Overlay], frame_size: Option<(f32, f32)>) -> (f32, f32) {
    match frame_size {
        Some(size) => size,
        None => {
            let width = overlays
                .iter()
                .map(|overlay| overlay.x + overlay.width)
                .fold(0.0_f32, f32::max)
                .max(1.0);
            let height = overlays
                .iter()
                .map(|overlay| overlay.y + overlay.height)
                .fold(0.0_f32, f32::max)
                .max(1.0);
            (width * 1.1, height * 1.1)
        }
    }
}

/// The card's instance lines. Online they come from the status read and fact
/// snapshot taken once, right after the pin, at the positions the first two
/// lines state: state at open, not state at the pinned snapshot, and never
/// refreshed. Offline they are the facts the read face replays at the pinned
/// position itself; there is no status offline. One short value per line,
/// since a line in the middle of the card cannot wrap: a failure is split into
/// its parts.
fn instance_lines(labels: &Labels, source: &ReadSource) -> Vec<FieldLine> {
    let online = match source.instance_facts() {
        InstanceFacts::Offline(facts) => return offline_instance_lines(labels, facts),
        InstanceFacts::Online(online) => online,
    };
    let read = match &*online {
        Err(failure) => {
            let text = labels.instances_unread.to_string();
            let mut lines = vec![field(labels, "runtime_instances", text, "")];
            if let Some(runtime_code) = &failure.runtime_code {
                lines.push(field(labels, "runtime_refusal", runtime_code.clone(), ""));
            }
            let code = failure.code.to_string();
            lines.push(field(labels, "client_error", code, failure.operation));
            if let Some((host_code, operation)) = &failure.host {
                lines.push(field(labels, "host_failure", host_code.clone(), operation.as_str()));
            }
            return lines;
        }
        Ok(read) => read,
    };
    let mut lines = vec![
        field(labels, "instances_status_at", read.status_sequence.to_string(), ""),
        field(labels, "instances_facts_at", read.facts_position.to_string(), ""),
    ];
    if read.instances.is_empty() {
        lines.push(field(labels, "runtime_instances", labels.instances_none.to_string(), ""));
    }
    for instance in &read.instances {
        let id = instance.instance_id.as_str();
        match &instance.status {
            Some(live) => {
                lines.push(field(labels, "instance_alias", live.alias.clone(), id));
                let port =
                    live.adb_port.map_or_else(|| labels.none.to_string(), |port| port.to_string());
                lines.push(field(labels, "adb_port", port, ""));
                let lease = match (live.lease_active, live.takeover_cooldown_active) {
                    (true, _) => labels.lease_active,
                    (false, true) => labels.lease_cooldown,
                    (false, false) => labels.lease_idle,
                };
                let lease = match live.queued_request_count {
                    0 => lease.to_string(),
                    queued => fill(labels.lease_queued, &[lease, &queued.to_string()]),
                };
                lines.push(field(labels, "lease", lease, ""));
            }
            None => {
                let text = labels.not_registered.to_string();
                lines.push(field(labels, "instance_alias", text, id));
            }
        }
        lines.extend(task_fact_lines(labels, instance));
    }
    lines
}

/// The offline instance lines: the pinned position, that there is no lease
/// offline, then each instance's facts; or the read face's own reason or error.
fn offline_instance_lines(labels: &Labels, facts: &OfflineFacts) -> Vec<FieldLine> {
    match facts {
        OfflineFacts::NoEvents => {
            vec![field(labels, "instance_facts", labels.facts_no_events.to_string(), "0")]
        }
        OfflineFacts::Available { position, instances } => {
            let mut header = position.to_string();
            if instances.is_empty() {
                header.push_str(" · ");
                header.push_str(labels.facts_none);
            }
            let lease = labels.offline_not_provided.to_string();
            let mut lines = vec![
                field(labels, "instance_facts", header, "offline"),
                field(labels, "lease", lease, ""),
            ];
            for instance in instances {
                let id = instance.instance_id.as_str();
                lines.push(field(labels, "instance_id", short_id(id), id));
                lines.extend(task_fact_lines(labels, instance));
            }
            lines
        }
        OfflineFacts::NotAvailable { position, reason, .. } => {
            let text = fill(labels.facts_not_available, &[reason]);
            vec![field(labels, "instance_facts", text, position.to_string())]
        }
        OfflineFacts::Failed { position, code, operation, detail, io_kind } => {
            let unread = labels.instances_unread.to_string();
            let mut lines = vec![
                field(labels, "instance_facts", unread, position.to_string()),
                field(labels, "facts_error", code.to_string(), *operation),
            ];
            if let Some(kind) = io_kind {
                lines.push(field(labels, "io_kind", kind.clone(), ""));
            }
            lines.push(field(labels, "facts_detail", detail.clone(), ""));
            lines
        }
    }
}

/// An instance's three task facts, or "not recorded".
fn task_fact_lines(labels: &Labels, instance: &RuntimeInstance) -> [FieldLine; 3] {
    let fact = |value: &Option<String>| {
        value.clone().unwrap_or_else(|| labels.fact_unrecorded.to_string())
    };
    [
        field(labels, "task_game", fact(&instance.game), "task.game"),
        field(labels, "task_server", fact(&instance.server), "task.server"),
        field(labels, "task_page", fact(&instance.page), "task.page"),
    ]
}

fn field(
    labels: &Labels,
    key: &str,
    value: String,
    raw: impl Into<SharedString>,
) -> FieldLine {
    FieldLine {
        label: labels.field(key).into(),
        value: value.into(),
        raw: raw.into(),
        wrap: false,
    }
}

fn shared(items: &[&'static str]) -> ModelRc<SharedString> {
    models(items.iter().map(|item| SharedString::from(*item)).collect())
}

fn models<T: Clone + 'static>(items: Vec<T>) -> ModelRc<T> {
    ModelRc::new(VecModel::from(items))
}
