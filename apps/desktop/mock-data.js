// SPDX-License-Identifier: AGPL-3.0-only
window.GachaPilotMockData = (() => {
  const profiles = [
    {
      id: "alasr",
      name: "AlasR",
      short: "AZ",
      gameServerLabel: "Azur.jp",
      fullName: "Azur.jp 港区OA",
      state: "running",
      stateText: "运行中",
      color: "#786ee6",
      resourceSnapshot: [
        { key: "oil", label: "石油", value: "464 / 17350", updatedAgo: "1小时前", color: "#050505", delta: "+120" },
        { key: "coin", label: "物资", value: "10556 / 101400", updatedAgo: "1小时前", color: "#ffb23e", delta: "+2460" },
        { key: "gem", label: "钻石", value: "295", updatedAgo: "2小时前", color: "#ff4b4b", delta: "0" },
        { key: "pt", label: "活动PT", value: "62750 / 1500000", updatedAgo: "1小时前", color: "#16c4f6", delta: "+840" },
        { key: "cube", label: "魔方", value: "844", updatedAgo: "2小时前", color: "#34e3e8", delta: "+2" },
        { key: "action", label: "行动力", value: "54 (404)", updatedAgo: "3天前", color: "#1017ff", delta: "-60" },
      ],
      resourceHistorySummary: {
        primaryKey: "oil",
        labels: ["18:00", "19:00", "20:00", "21:00", "22:00", "23:00"],
        values: [310, 360, 330, 420, 410, 464],
      },
      scheduler: {
        alive: true,
        currentTaskLabel: "每日任务",
        nextTaskLabel: "科研",
        nextRunTime: "2026-05-31 21:59",
        pendingCount: 1,
        waitingCount: 4,
        lastSeverity: "info",
      },
    },
    {
      id: "maab",
      name: "MaaB",
      short: "AK",
      gameServerLabel: "Ark.cn",
      fullName: "Ark.cn 罗德岛B",
      state: "idle",
      stateText: "待机",
      color: "#35d8a4",
      resourceSnapshot: [
        { key: "sanity", label: "理智", value: "128 / 135", updatedAgo: "刚刚", color: "#48d17d", delta: "+28" },
        { key: "lmd", label: "龙门币", value: "1245800", updatedAgo: "刚刚", color: "#ffb23e", delta: "+43200" },
        { key: "orundum", label: "合成玉", value: "42600", updatedAgo: "12分钟前", color: "#ff6464", delta: "+100" },
      ],
      resourceHistorySummary: {
        primaryKey: "sanity",
        labels: ["18:00", "19:00", "20:00", "21:00", "22:00", "23:00"],
        values: [72, 86, 104, 118, 123, 128],
      },
      scheduler: {
        alive: false,
        currentTaskLabel: "无任务",
        nextTaskLabel: "信用商店",
        nextRunTime: "2026-05-31 22:30",
        pendingCount: 0,
        waitingCount: 3,
        lastSeverity: "info",
      },
    },
    {
      id: "baasjp",
      name: "BaasJP",
      short: "BA",
      gameServerLabel: "BA.jp",
      fullName: "BA.jp 夏莱档案",
      state: "warning",
      stateText: "等待确认",
      color: "#ff8c6b",
      resourceSnapshot: [
        { key: "ap", label: "AP", value: "172 / 230", updatedAgo: "3分钟前", color: "#48d17d", delta: "+34" },
        { key: "credit", label: "信用点", value: "8154200", updatedAgo: "3分钟前", color: "#ffb23e", delta: "+120000" },
        { key: "pyroxene", label: "青辉石", value: "23640", updatedAgo: "1小时前", color: "#4da3ff", delta: "+30" },
      ],
      resourceHistorySummary: {
        primaryKey: "ap",
        labels: ["18:00", "19:00", "20:00", "21:00", "22:00", "23:00"],
        values: [96, 118, 137, 151, 166, 172],
      },
      scheduler: {
        alive: false,
        currentTaskLabel: "无任务",
        nextTaskLabel: "总力战",
        nextRunTime: "2026-05-31 23:00",
        pendingCount: 0,
        waitingCount: 3,
        lastSeverity: "warning",
      },
    },
  ];

  const logs = [
    { timestamp: "2026-05-31T21:23:49Z", level: "INFO", source: "runtime", message: "预览数据已加载" },
    { timestamp: "2026-05-31T21:23:51Z", level: "INFO", source: "scheduler", message: "等待 AliceRuntimeOrchestrator 连接" },
    { timestamp: "2026-05-31T21:24:06Z", level: "WARNING", source: "ui", message: "当前为降级预览模式" },
  ];

  const acquisitions = [
    { timestamp: "2026-05-31T21:23:49Z", profileId: "alasr", imageReference: "runtime://mock/acquisition-001", labels: ["物资", "活动PT"], sourceTask: "每日任务" },
    { timestamp: "2026-05-31T21:28:12Z", profileId: "maab", imageReference: "runtime://mock/acquisition-002", labels: ["龙门币"], sourceTask: "日常关卡" },
    { timestamp: "2026-05-31T21:31:46Z", profileId: "baasjp", imageReference: "runtime://mock/acquisition-003", labels: ["信用点", "青辉石"], sourceTask: "悬赏通缉" },
  ];

  return {
    snapshot(reason = "AliceRuntimeOrchestrator 不可用") {
      return {
        degraded: true,
        degradedReason: reason,
        health: null,
        status: {
          runtime: {
            orchestratorAlive: false,
            automationAlive: false,
            state: "unavailable",
            lastSeverity: "warning",
          },
          scheduler: {
            alive: false,
            currentTaskLabel: "无任务",
            nextTaskLabel: "未知",
            nextRunTime: "未知",
            pendingCount: 0,
            waitingCount: 0,
            lastSeverity: "warning",
          },
        },
        profiles,
        logs,
        resourceHistory: [],
        acquisitions,
      };
    },
  };
})();
