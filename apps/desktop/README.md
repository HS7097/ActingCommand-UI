# ActingCommand Desktop UI Prototype

Standalone desktop UI preview for the ActingCommand shell.

Open `index.html` directly or serve this directory with a local static server.

The UI is a client of `AliceRuntimeOrchestrator`. If the runtime is not
available, the page shows a visible degraded-state banner and uses preview data.

Current scope:

- AzurPilot-style dark shell
- Enlarged instance sidebar with game/server labels
- Runtime availability banner
- Main resource dashboard for Azur.jp, Ark.cn, and BA.jp
- Resource history chart and recent acquisition screenshot index area
- Compact scheduler summary instead of deep scheduler internals
- Runtime data facade that can use either mock data or `AliceRuntimeOrchestrator`

Preview routes:

- `/#overview`
- `/#alasr`
- `/#maab`
- `/#baasjp`
- `/#settings`

Runtime defaults:

- HTTP API: `http://127.0.0.1:8765`
- WebSocket events: `ws://127.0.0.1:8766/events`

Override them with query parameters:

- `?runtime=http://127.0.0.1:8765`
- `?events=ws://127.0.0.1:8766/events`
