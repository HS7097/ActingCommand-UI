// SPDX-License-Identifier: GPL-3.0-only
//! The Composition environment of the windows' thread: made when the first
//! window builds its glass, shared by all of them, and kept to the end of the
//! process. It is never released: the thread's end is too late to call into
//! COM, and nothing here calls `RoUninitialize` either.

use std::cell::OnceCell;
use std::mem::{size_of, ManuallyDrop};

use windows::core::Result as WinResult;
use windows::System::{DispatcherQueue, DispatcherQueueController};
use windows::UI::Composition::Compositor;
use windows::Win32::Foundation::RPC_E_CHANGED_MODE;
use windows::Win32::System::WinRT::{
    CreateDispatcherQueueController, DispatcherQueueOptions, RoInitialize, DQTAT_COM_NONE, DQTYPE_THREAD_CURRENT,
    RO_INIT_SINGLETHREADED,
};

use crate::{GlassError, Step};

struct Composition {
    compositor: Compositor,
    /// Composition needs a dispatcher queue on its thread as long as it runs.
    _queue: Option<DispatcherQueueController>,
}

thread_local! {
    // ManuallyDrop: with nothing to drop, the thread's end does not touch it.
    static COMPOSITION: OnceCell<ManuallyDrop<Composition>> = const { OnceCell::new() };
}

fn create() -> WinResult<Composition> {
    // SAFETY: takes no pointer, and is paired with no uninitialize: the
    // apartment stays to the end of the process.
    match unsafe { RoInitialize(RO_INIT_SINGLETHREADED) } {
        // S_FALSE, already initialized this way, comes back as Ok too.
        Ok(()) => {}
        // winit has already made this thread a COM apartment of another kind.
        Err(error) if error.code() == RPC_E_CHANGED_MODE => {}
        Err(error) => return Err(error),
    }
    let queue = if DispatcherQueue::GetForCurrentThread().is_ok() {
        None
    } else {
        let options = DispatcherQueueOptions {
            dwSize: size_of::<DispatcherQueueOptions>() as u32,
            threadType: DQTYPE_THREAD_CURRENT,
            apartmentType: DQTAT_COM_NONE,
        };
        // SAFETY: `options` is passed by value, its size set as the call asks;
        // the controller is kept with the compositor.
        Some(unsafe { CreateDispatcherQueueController(options) }?)
    };
    Ok(Composition { compositor: Compositor::new()?, _queue: queue })
}

/// This thread's compositor. What failed is not remembered: the next window, or
/// the next theme picked, tries again.
pub fn compositor() -> Result<Compositor, GlassError> {
    COMPOSITION.with(|cell| {
        if let Some(composition) = cell.get() {
            return Ok(composition.compositor.clone());
        }
        let made = create().map_err(|error| GlassError { step: Step::Composition, detail: error.to_string() })?;
        Ok(cell.get_or_init(|| ManuallyDrop::new(made)).compositor.clone())
    })
}
