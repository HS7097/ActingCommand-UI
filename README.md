# ActingCommand UI

Standalone Web UI shell for ActingCommand.

This repository contains only the user-facing UI prototype. It does not own the automation runtime lifecycle and must not directly execute game automation logic.

## Responsibility

- render Overview, profile detail, and Settings
- connect to `AliceRuntimeOrchestrator` over localhost HTTP and WebSocket
- send user commands to the runtime API
- display runtime state, logs, resource history, and acquisition metadata
- show visible degraded-state errors when the runtime is unavailable

## Local preview

```powershell
.\scripts\serve-desktop.ps1 -Port 5177
```

Then open:

```text
http://127.0.0.1:5177/#overview
```

The UI expects the runtime API at:

```text
http://127.0.0.1:8765
ws://127.0.0.1:8766/events
```

Use query parameters for temporary overrides:

```text
?runtime=http://127.0.0.1:8765&events=ws://127.0.0.1:8766/events
```

## License

ActingCommand UI is planned under `AGPL-3.0-only`.

Do not copy upstream source code or assets into this repository until license conditions, attribution, source-availability, and modification-record obligations are verified.
