# AGENTS.md

## Repository scope

This repository is `HS7097/ActingCommand-UI`.

UI work should keep planning and checkpoint files inside this repository. Do not rely on the umbrella repository for routine UI task tracking.

Before making UI changes, read these files if they exist:

- `PLANS.md`
- `CHECKPOINT.md`
- `LICENSE_POLICY.md`
- `NOTICE.md`

## UI direction

- The UI is a standalone client shell.
- The UI must not own Runtime process lifetime.
- Runtime communication should happen through the approved local API or IPC boundary.
- The UI should present state, commands, errors, logs, resource history, and acquisition/capture references.
- The UI should not import Runtime internals or upstream automation source directly.

## Error handling

- Severe UI/runtime connection or data-integrity errors must never silently fail.
- Runtime-unavailable states must be visible to the user.
- Transient connection problems may show degraded state, but the path must be logged and visible.

## Planning and checkpoint updates

For each UI task:

- update this repository's `PLANS.md` when phase, scope, boundaries, or next steps change;
- update this repository's `CHECKPOINT.md` with changed files, commands run, validation, blockers, and next steps;
- commit `PLANS.md` and `CHECKPOINT.md` in the same UI commit or same UI task branch as the source changes;
- push the UI repository after the task is completed and verified unless the user explicitly says not to push.

Do not mirror planning files into `HS7097/ActingCommand` after routine UI tasks. Use the umbrella repository only for umbrella-level planning, cross-repository policy, or meta-documentation.

## Current boundaries

- Do not implement Runtime orchestration logic in the UI.
- Do not implement device control, capture, OCR, recognition, SQLite, scheduler, or game logic in the UI.
- Keep UI changes focused on presentation and user command surfaces unless a plan explicitly expands the scope.
