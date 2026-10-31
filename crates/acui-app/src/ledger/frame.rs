// SPDX-License-Identifier: GPL-3.0-only
//! The frame pane's reads, one slot per window (`FrameSlot`): a frame is read
//! on a worker of its own and its PNG decoded there. The worker never holds a
//! language table; what it has to say goes back as a `Phrase`, and the event
//! loop paints it (`paint::frame_read`) in the language shown then.

use std::io;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};

use acui_model::FrameTarget;
use acui_source::MaterialReader;
use slint::ComponentHandle;

use crate::ledger::paint;
use crate::phrase::{verbatim, Phrase};
use crate::{launcher, report, ui, AppWindow};

/// One frame read: the event it is for and its generation, what the pane says
/// about it — reading, verified, or why not — and once that read is verified
/// and decoded, the pixel size. A switch, a clear or a failure drops the size
/// with the request it belongs to.
pub struct FrameRequest {
    pub sequence: u64,
    pub generation: u64,
    pub decoded: Option<(u32, u32)>,
    pub note: Phrase,
}

/// One window's frame reads: the request its pane has made, if any; the
/// generation, bumped per request and per clear, so a slow read can never
/// paint a stale frame, and so a superseded read can see that it has been
/// superseded; and its one material worker at a time.
#[derive(Default)]
pub struct FrameSlot {
    /// Shared with the read's completion, which runs on the event loop but
    /// has to be `Send`.
    request: Arc<Mutex<Option<FrameRequest>>>,
    generation: Arc<AtomicU64>,
    worker: Arc<Mutex<()>>,
}

impl FrameSlot {
    pub fn request(&self) -> MutexGuard<'_, Option<FrameRequest>> {
        self.request.lock().unwrap_or_else(|held| held.into_inner())
    }

    /// The pixel size the current request's verified frame decoded to, when
    /// that request was made for this event.
    pub fn decoded_size(&self, sequence: u64) -> Option<(f32, f32)> {
        let held = self.request();
        let request = held.as_ref()?;
        let (width, height) = request.decoded.filter(|_| request.sequence == sequence)?;
        Some((width as f32, height as f32))
    }

    /// The generation of a new request: every read started before it is
    /// superseded.
    pub fn next_generation(&self) -> u64 {
        self.generation.fetch_add(1, Ordering::SeqCst) + 1
    }

    /// No request, and no read in flight still wanted.
    pub fn clear(&self) {
        *self.request() = None;
        self.generation.fetch_add(1, Ordering::SeqCst);
    }

    /// No request: the next paint of a frame asks again.
    pub fn forget(&self) {
        *self.request() = None;
    }

    /// Reads `target`'s frame through `reader` at `snapshot`, for the request
    /// of `generation`, on `acui-frame`. One worker at a time. A whole read
    /// cannot be stopped midway on either face, so a superseded one runs to
    /// its end, bounded by the byte cap and the deadline, and its result is
    /// dropped; one queued behind the slot that is superseded by then never
    /// starts. A read that left the material connection latched, or a reader
    /// whose rebuild had failed, hands `rebuild` to the event loop, superseded
    /// or not, so the window's reader makes the next one. A thread the system
    /// refuses comes back as the error.
    pub fn read(
        &self,
        window: &AppWindow,
        reader: MaterialReader,
        target: FrameTarget,
        snapshot: u64,
        generation: u64,
        payload_frame_size: Option<(f32, f32)>,
        rebuild: impl FnOnce() + Send + 'static,
    ) -> io::Result<()> {
        let shared = Arc::clone(&self.generation);
        let frame = Arc::clone(&self.request);
        let worker = Arc::clone(&self.worker);
        let weak = window.as_weak();
        launcher::spawn_worker("acui-frame", move || {
            let _slot = worker.lock().unwrap_or_else(|held| held.into_inner());
            let still_wanted = || shared.load(Ordering::SeqCst) == generation;
            if !still_wanted() {
                return;
            }
            let read = reader.read(target.event, &target.artifact, snapshot, &still_wanted);
            if needs_rebuild(&reader) {
                report::post("acui-frame", rebuild);
            }
            let painted: Result<(u32, u32, Vec<u8>), Phrase> = match read {
                Err(error) => Err(verbatim(error.to_string())),
                Ok(None) => return,
                Ok(Some(outcome)) => match &outcome.bytes {
                    Some(bytes) => decode_png(bytes),
                    None => Err(paint::outcome_phrase(&outcome)),
                },
            };
            let note = match &painted {
                Ok((width, height, _)) => paint::verified_phrase(*width, *height),
                Err(text) => Arc::clone(text),
            };
            report::post("acui-frame", move || {
                if shared.load(Ordering::SeqCst) != generation {
                    return;
                }
                // The decoded size and the note belong to this request; a
                // failure leaves the size unknown.
                if let Some(request) = frame
                    .lock()
                    .unwrap_or_else(|held| held.into_inner())
                    .as_mut()
                    .filter(|request| request.generation == generation)
                {
                    request.decoded =
                        painted.as_ref().ok().map(|(width, height, _)| (*width, *height));
                    request.note = Arc::clone(&note);
                }
                let (Some(window), Some(ui)) = (weak.upgrade(), ui::current()) else {
                    return;
                };
                paint::frame_read(&window, &ui, note, painted, payload_frame_size);
            });
        })
    }
}

/// Whether the next frame read needs a new material reader: the online one
/// is latched (a read failed fatally on its connection), or the rebuild that
/// made this one had failed.
fn needs_rebuild(reader: &MaterialReader) -> bool {
    match reader {
        MaterialReader::Offline(_) => false,
        MaterialReader::Online { latched, .. } => latched.load(Ordering::SeqCst),
        MaterialReader::Unavailable(_) => true,
    }
}

fn decode_png(bytes: &[u8]) -> Result<(u32, u32, Vec<u8>), Phrase> {
    let decoded = image::load_from_memory_with_format(bytes, image::ImageFormat::Png)
        .map_err(|error| paint::decode_failed(error.to_string()))?
        .to_rgba8();
    Ok((decoded.width(), decoded.height(), decoded.into_raw()))
}
