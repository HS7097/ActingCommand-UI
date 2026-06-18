# PLANS.md

## Repository goal

`ActingCommand-UI` is the standalone UI shell for ActingCommand.

The UI should remain a client of the Runtime, not an owner of Runtime lifecycle or automation internals.

## Current direction

- Present overview, profile status, settings, logs, resource history, and acquisition/capture references.
- Connect to Runtime through local API or IPC contracts.
- Show visible degraded state when Runtime is unavailable.
- Keep Runtime execution, device control, capture, recognition, scheduler, SQLite, and game logic outside the UI repository.

## Repo-local planning policy

UI planning and checkpoint records live in this repository.

For UI tasks, update `PLANS.md` and `CHECKPOINT.md` here and commit them with the UI source changes. Do not mirror UI task planning files into the umbrella repository by default.

## Active boundaries

- UI must not own Runtime process lifetime.
- UI must not import upstream automation modules.
- UI must not silently hide Runtime or API failures.
- UI must not implement device, capture, OCR, recognition, SQLite, scheduler, or game automation logic.

## Next steps

1. Align UI API assumptions with the Runtime contracts.
2. Keep visible degraded-state handling for unavailable Runtime paths.
3. Keep `CHECKPOINT.md` updated with every completed UI task.
