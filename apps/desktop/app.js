// SPDX-License-Identifier: AGPL-3.0-only
const appState = {
  snapshot: window.GachaPilotMockData.snapshot("正在连接 AliceRuntimeOrchestrator"),
  currentProfileId: "alasr",
  currentView: "overview",
  eventStreamError: "",
};

const $ = (selector) => document.querySelector(selector);

function escapeHtml(value) {
  return String(value ?? "")
    .replaceAll("&", "&amp;")
    .replaceAll("<", "&lt;")
    .replaceAll(">", "&gt;")
    .replaceAll('"', "&quot;")
    .replaceAll("'", "&#039;");
}

function formatTime(timestamp) {
  if (!timestamp) return "未知";
  const date = new Date(timestamp);
  if (Number.isNaN(date.valueOf())) return timestamp;
  return date.toLocaleTimeString("zh-CN", { hour12: false });
}

function currentProfiles() {
  return appState.snapshot.profiles || [];
}

function currentProfile() {
  return currentProfiles().find((profile) => profile.id === appState.currentProfileId) || currentProfiles()[0];
}

function statusText(snapshot = appState.snapshot) {
  if (snapshot.degraded) return "运行时不可用";
  const runtime = snapshot.status?.runtime;
  if (runtime?.automationAlive) return "自动化运行中";
  return "运行时已连接";
}

function severityClass(severity) {
  const value = String(severity || "info").toLowerCase();
  if (value === "fatal" || value === "error") return "error";
  if (value === "warning") return "warning";
  return "info";
}

function renderRuntimeBanner(targetId) {
  const target = $(`#${targetId}`);
  if (!target) return;
  const snapshot = appState.snapshot;
  const severity = severityClass(snapshot.status?.runtime?.lastSeverity);
  const eventError = appState.eventStreamError ? `；事件流：${appState.eventStreamError}` : "";
  target.className = `runtime-banner ${target.classList.contains("compact") ? "compact" : ""} ${snapshot.degraded ? "warning" : severity}`;
  target.innerHTML = `
    <strong>${escapeHtml(statusText(snapshot))}</strong>
    <span>${snapshot.degraded ? escapeHtml(snapshot.degradedReason || "无法连接本地运行时，正在使用预览数据") : `HTTP ${escapeHtml(window.GachaPilotRuntimeClient.httpBase)}，事件 ${escapeHtml(window.GachaPilotRuntimeClient.wsBase)}`}${escapeHtml(eventError)}</span>
  `;
}

function renderInstances() {
  $("#instanceList").innerHTML = currentProfiles()
    .map((profile) => {
      const active = appState.currentView === "instance" && profile.id === appState.currentProfileId;
      return `
        <button class="instance-button ${escapeHtml(profile.state)} ${active ? "active" : ""}" data-instance="${escapeHtml(profile.id)}">
          <span class="instance-icon" style="background:${escapeHtml(profile.color)}">${escapeHtml(profile.short)}</span>
          <span class="instance-text">
            <strong>${escapeHtml(profile.name)}</strong>
            <small>${escapeHtml(profile.gameServerLabel)}</small>
          </span>
        </button>
      `;
    })
    .join("");
}

function trendPoints(values, width = 420, height = 150) {
  if (!values?.length) return "";
  const min = Math.min(...values);
  const max = Math.max(...values);
  const range = Math.max(max - min, 1);
  const step = values.length > 1 ? width / (values.length - 1) : width;
  return values
    .map((value, index) => {
      const x = index * step;
      const y = height - ((value - min) / range) * (height - 22) - 11;
      return `${x.toFixed(1)},${y.toFixed(1)}`;
    })
    .join(" ");
}

function renderChart(profile) {
  const history = profile?.resourceHistorySummary || { labels: [], values: [] };
  const points = trendPoints(history.values || []);
  const color = profile?.color || "#8b82ef";
  return `
    <svg class="line-chart" viewBox="0 0 420 170" role="img" aria-label="resource trend">
      <line x1="0" y1="150" x2="420" y2="150"></line>
      <line x1="0" y1="98" x2="420" y2="98"></line>
      <line x1="0" y1="46" x2="420" y2="46"></line>
      <polyline points="${escapeHtml(points)}" style="stroke:${escapeHtml(color)}"></polyline>
      ${(history.values || [])
        .map((value, index) => {
          const pair = points.split(" ")[index] || "0,0";
          const [x, y] = pair.split(",");
          return `<circle cx="${escapeHtml(x)}" cy="${escapeHtml(y)}" r="4" style="fill:${escapeHtml(color)}"><title>${escapeHtml(value)}</title></circle>`;
        })
        .join("")}
    </svg>
    <div class="chart-labels">${(history.labels || []).map((label) => `<span>${escapeHtml(label)}</span>`).join("")}</div>
  `;
}

function renderResourceLines(profile) {
  return (profile.resourceSnapshot || [])
    .map((resource) => {
      const delta = String(resource.delta || "0");
      return `
        <div class="resource-line">
          <span class="resource-dot" style="background:${escapeHtml(resource.color)}"></span>
          <span class="resource-name">${escapeHtml(resource.label)}</span>
          <strong>${escapeHtml(resource.value)}</strong>
          <small>${escapeHtml(resource.updatedAgo)}</small>
          <em class="${delta.startsWith("-") ? "down" : "up"}">${escapeHtml(delta)}</em>
        </div>
      `;
    })
    .join("");
}

function renderAcquisitions(profileId = null) {
  const items = (appState.snapshot.acquisitions || []).filter((item) => !profileId || item.profileId === profileId);
  if (!items.length) {
    return `<div class="empty-state compact-empty">暂无截图索引</div>`;
  }
  return items
    .map((item) => {
      const profile = currentProfiles().find((candidate) => candidate.id === item.profileId);
      return `
        <div class="acquisition-card">
          <div class="capture-thumb" style="--thumb-color:${escapeHtml(profile?.color || "#746bd6")}">
            <span>${escapeHtml(profile?.short || "GP")}</span>
            <small>${escapeHtml(item.imageReference || "无截图引用")}</small>
          </div>
          <strong>${escapeHtml((item.labels || []).join(" / ") || "未识别")}</strong>
          <small>${escapeHtml(item.sourceTask || "未知来源")} · ${escapeHtml(formatTime(item.timestamp))}</small>
        </div>
      `;
    })
    .join("");
}

function renderLogRows(limit = 30) {
  const logs = (appState.snapshot.logs || []).slice(-limit);
  if (!logs.length) return "暂无日志";
  return logs
    .map((log) => {
      const level = escapeHtml(log.level || "INFO");
      const levelClass = level === "WARNING" ? "warning" : level === "ERROR" || level === "FATAL" ? "error" : "info";
      return `<span class="${levelClass}">${level}</span> <span class="time">${escapeHtml(formatTime(log.timestamp))}</span> | [${escapeHtml(log.source)}] ${escapeHtml(log.message)}`;
    })
    .join("\n");
}

function renderOverview() {
  renderRuntimeBanner("runtimeBanner");
  const profiles = currentProfiles();
  const selected = currentProfile();
  const scheduler = appState.snapshot.status?.scheduler || selected?.scheduler || {};
  $("#overviewResources").innerHTML = `
    <div class="instance-strip">
      ${profiles
        .map(
          (profile) => `
            <button class="strip-card ${escapeHtml(profile.state)} ${profile.id === appState.currentProfileId ? "active" : ""}" data-instance="${escapeHtml(profile.id)}">
              <span class="strip-icon" style="background:${escapeHtml(profile.color)}">${escapeHtml(profile.short)}</span>
              <span>
                <strong>${escapeHtml(profile.name)}</strong>
                <small>${escapeHtml(profile.gameServerLabel)} · ${escapeHtml(profile.fullName)}</small>
              </span>
              <em>${escapeHtml(profile.stateText)}</em>
            </button>
          `,
        )
        .join("")}
    </div>

    <div class="overview-dashboard">
      <article class="panel dense-resource-panel">
        <div class="panel-title compact-title">
          <h2>资源数据</h2>
          <span>${profiles.length} 个实例</span>
        </div>
        <div class="resource-matrix">
          ${profiles
            .map(
              (profile) => `
                <section class="resource-group">
                  <div class="resource-group-head">
                    <strong>${escapeHtml(profile.fullName)}</strong>
                    <span>${escapeHtml(profile.name)}</span>
                  </div>
                  <div class="resource-list dense">${renderResourceLines(profile)}</div>
                </section>
              `,
            )
            .join("")}
        </div>
      </article>

      <article class="panel screenshot-panel">
        <div class="panel-title compact-title">
          <h2>最近获得</h2>
          <span>截图索引</span>
        </div>
        <div class="acquisition-list">${renderAcquisitions()}</div>
      </article>

      <article class="panel chart-panel">
        <div class="panel-title compact-title">
          <h2>资源变化</h2>
          <span>${escapeHtml(selected?.gameServerLabel || "未选择")}</span>
        </div>
        ${renderChart(selected)}
      </article>

      <article class="panel recent-panel">
        <div class="panel-title compact-title">
          <h2>调度摘要</h2>
          <span>${escapeHtml(statusText())}</span>
        </div>
        <div class="summary-metrics">
          <div><span>当前任务</span><strong>${escapeHtml(scheduler.currentTaskLabel || "无任务")}</strong></div>
          <div><span>下一任务</span><strong>${escapeHtml(scheduler.nextTaskLabel || "未知")}</strong></div>
          <div><span>下次运行</span><strong>${escapeHtml(scheduler.nextRunTime || "未知")}</strong></div>
          <div><span>队列</span><strong>${escapeHtml(scheduler.pendingCount ?? 0)} / ${escapeHtml(scheduler.waitingCount ?? 0)}</strong></div>
        </div>
        <pre class="log-output compact-log">${renderLogRows(10)}</pre>
      </article>
    </div>
  `;
}

function renderProfileDetail() {
  const profile = currentProfile();
  if (!profile) return;
  $("#instanceTitle").textContent = profile.name;
  $("#instanceSubtitle").textContent = `${profile.gameServerLabel} · ${profile.fullName}`;
  renderRuntimeBanner("profileRuntimeBanner");
  const scheduler = profile.scheduler || {};
  $("#profileSurface").innerHTML = `
    <div class="profile-grid">
      <article class="panel">
        <div class="panel-title compact-title">
          <h2>调度器</h2>
          <span>${escapeHtml(profile.stateText)}</span>
        </div>
        <div class="summary-metrics profile-metrics">
          <div><span>运行状态</span><strong>${scheduler.alive ? "运行中" : "待机"}</strong></div>
          <div><span>当前任务</span><strong>${escapeHtml(scheduler.currentTaskLabel || "无任务")}</strong></div>
          <div><span>下一任务</span><strong>${escapeHtml(scheduler.nextTaskLabel || "未知")}</strong></div>
          <div><span>下次运行</span><strong>${escapeHtml(scheduler.nextRunTime || "未知")}</strong></div>
          <div><span>待执行</span><strong>${escapeHtml(scheduler.pendingCount ?? 0)}</strong></div>
          <div><span>等待中</span><strong>${escapeHtml(scheduler.waitingCount ?? 0)}</strong></div>
        </div>
      </article>

      <article class="panel">
        <div class="panel-title compact-title">
          <h2>资源变化</h2>
          <span>${escapeHtml(profile.gameServerLabel)}</span>
        </div>
        ${renderChart(profile)}
      </article>

      <article class="panel">
        <div class="panel-title compact-title">
          <h2>最近截图</h2>
          <span>${escapeHtml(profile.name)}</span>
        </div>
        <div class="acquisition-list">${renderAcquisitions(profile.id)}</div>
      </article>

      <article class="panel log-panel-large">
        <div class="panel-title compact-title">
          <h2>日志</h2>
          <span>最近事件</span>
        </div>
        <pre class="log-output">${renderLogRows(80)}</pre>
      </article>
    </div>
  `;
}

function renderSettings() {
  renderRuntimeBanner("settingsRuntimeBanner");
  $("#settingsInstances").innerHTML = currentProfiles()
    .map(
      (profile) => `
        <div class="settings-row">
          <span>${escapeHtml(profile.name)}</span>
          <strong>${escapeHtml(profile.gameServerLabel)}</strong>
        </div>
      `,
    )
    .join("");

  const health = appState.snapshot.health;
  $("#runtimeSettings").innerHTML = `
    <div class="settings-row"><span>HTTP API</span><strong>${escapeHtml(window.GachaPilotRuntimeClient.httpBase)}</strong></div>
    <div class="settings-row"><span>事件流</span><strong>${escapeHtml(window.GachaPilotRuntimeClient.wsBase)}</strong></div>
    <div class="settings-row"><span>PID</span><strong>${escapeHtml(health?.pid || "未连接")}</strong></div>
    <div class="settings-row"><span>状态目录</span><strong>${escapeHtml(health?.stateDir || "%LOCALAPPDATA%")}</strong></div>
    <div class="tool-grid runtime-actions">
      <button data-command="start">启动自动化</button>
      <button data-command="stop">停止自动化</button>
      <button data-command="restart">重启自动化</button>
      <button data-command="refresh">刷新状态</button>
    </div>
  `;
}

function renderAll() {
  renderInstances();
  renderOverview();
  renderProfileDetail();
  renderSettings();
  $("#topbarTitle").textContent =
    appState.currentView === "overview" ? "总览" : appState.currentView === "settings" ? "设置" : currentProfile()?.name || "实例";
}

function setView(view) {
  appState.currentView = view;
  $("#overviewPage").classList.toggle("hidden", view !== "overview");
  $("#instancePage").classList.toggle("hidden", view !== "instance");
  $("#settingsPage").classList.toggle("hidden", view !== "settings");
  document.querySelectorAll("[data-nav]").forEach((button) => {
    button.classList.toggle("active", button.dataset.nav === view);
  });
  renderAll();
}

function selectInstance(id, updateHash = true) {
  const next = currentProfiles().find((profile) => profile.id === id);
  if (!next) return;
  appState.currentProfileId = next.id;
  setView("instance");
  if (updateHash && location.hash !== `#${id}`) {
    history.replaceState(null, "", `#${id}`);
  }
}

function routeFromHash() {
  const nextHash = location.hash.replace("#", "");
  if (nextHash === "settings") {
    setView("settings");
  } else if (currentProfiles().some((profile) => profile.id === nextHash)) {
    selectInstance(nextHash, false);
  } else {
    setView("overview");
  }
}

async function loadRuntimeSnapshot() {
  appState.snapshot = await window.GachaPilotRuntimeClient.loadSnapshot();
  if (!currentProfiles().some((profile) => profile.id === appState.currentProfileId)) {
    appState.currentProfileId = currentProfiles()[0]?.id || "alasr";
  }
  renderAll();
}

async function runCommand(commandName) {
  try {
    await window.GachaPilotRuntimeClient.command(commandName);
    await loadRuntimeSnapshot();
  } catch (error) {
    appState.snapshot = window.GachaPilotMockData.snapshot(error.message);
    renderAll();
  }
}

document.addEventListener("click", (event) => {
  const nav = event.target.closest("[data-nav]");
  if (nav) {
    if (nav.dataset.nav === "overview") {
      history.replaceState(null, "", "#overview");
      setView("overview");
    }
    if (nav.dataset.nav === "settings") {
      history.replaceState(null, "", "#settings");
      setView("settings");
    }
    return;
  }

  const instance = event.target.closest("[data-instance]");
  if (instance) {
    selectInstance(instance.dataset.instance);
    return;
  }

  const command = event.target.closest("[data-command]");
  if (command) {
    runCommand(command.dataset.command);
  }
});

window.addEventListener("hashchange", routeFromHash);

renderAll();
loadRuntimeSnapshot().then(() => routeFromHash());
window.GachaPilotRuntimeClient.connectEvents(
  (event) => {
    if (event.type === "runtime.snapshot") {
      appState.snapshot = {
        degraded: false,
        degradedReason: "",
        ...event.payload,
      };
      appState.eventStreamError = "";
      renderAll();
      return;
    }
    if (event.type) {
      loadRuntimeSnapshot();
    }
  },
  (error) => {
    appState.eventStreamError = error.message;
    renderAll();
  },
);
