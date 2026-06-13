# AzurPilot UI References

This prototype follows the layout direction of AzurPilot while staying runnable as a standalone local web page.

Reference points inspected:

- `webapp/packages/renderer/src/components/AppHeader.vue`
  - Electron window shell and top header behavior.
- `webapp/packages/renderer/src/components/Alas.vue`
  - AzurPilot desktop app embeds the Python WebUI through an iframe.
- `module/webui/app.py`
  - Main sidebar, instance switching, overview, config panels, logs, and scheduler sections.
- `webapp/resource_chart.js`
  - Resource dashboard visualization patterns.

Implementation note:

The current GachaPilot prototype does not depend on AzurPilot's Electron/Vue build pipeline. It is intentionally a local browser UI first, so backend API and plugin contracts can be developed before packaging.

