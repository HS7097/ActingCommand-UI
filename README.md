# ActingCommand 监控台 v0-standalone

这是 ActingCommand 的**人类监控台**，一个只读的原生程序。它把离线导出的账本事件页
（`actingledger events` 的 JSON 输出）渲染成三栏界面：实例卡、时间线、详情。

它不是 Runtime 的一部分，是 Runtime 的**外部可拆客户端**。

## 遵循的裁定

- **Slint 原生渲染**：crates.io 上的 `slint` 1.x，Rust 原生窗口。没有 egui，没有
  Electron / Tauri / 任何 webview。
- **对 Runtime 零依赖**：不依赖任何 Runtime crate（无 git/path 依赖），不拉起、不包装任何
  CLI，不打开 state_root 里的任何文件（不读账本段、不读 runtime-state.sqlite、不读 artifacts 目录）。
  唯一输入是命令行上给出的导出事件页 JSON 文件。
- **四层四 crate**（同一个 Cargo workspace）：
  - `acui-rows`：`EventRow` 等行类型，镜像 `actingcommand.event.v2` 投影；serde 容忍未知字段。
  - `acui-source`：`EventSource` trait 与 `FileSource`（合并多页、按 sequence 排序去重）。
  - `acui-model`：纯 Rust 视图模型（页签、过滤、时间游标、选中项），不依赖 slint。
  - `acui-app`：唯一依赖 slint 的 crate，`.slint` 文件在 `crates/acui-app/ui/`。
  - 依赖方向：app → model → rows ← source。
- **无假数据**：代码里没有样例事件、没有演示生成器、没有占位行。不给文件时就老实显示空状态。
- **只读**：没有任何控制按钮、审批入口或写操作。
- **不载入图像**：产物只显示哈希占位（artifact_id / kind / media_type / byte_count / sha256），
  几何叠加画在空画布上，永不读取图像字节。
- **不写测试**：按裁定，UI 的 CI 只构建。

## 导出与运行

先用离线读取器导出页（在任一 state_root 上）：

```
actingledger --state-root <root> open > open.json
actingledger --state-root <root> events --after 0    --limit 1024 > page1.json
actingledger --state-root <root> events --after 1024 --limit 1024 > page2.json
```

然后运行监控台：

```
acui --events page1.json --events page2.json [--open open.json]
```

页可以任意顺序给出；行按 sequence 排序，重复 sequence 去重。
`--tab <stream|errors|observe|changes|health|lab>` 可指定启动时的页签（截图与复核用）。

## 六个页签

| 页签 | 归类依据 | 状态 |
| --- | --- | --- |
| 事件流 | 全部事件 | 裁定 |
| 错误 | severity ≥ warning（按契约枚举序） | 裁定 |
| 观察与操作（临） | event_type 前缀 capture. / recognition. / input.，或模块 capture、capture-pipeline、recognition、device-proxy | **临时** |
| 变更（临） | event_type 前缀 task. / artifact. / command. / lease. / scheduler.，或模块 artifact-store、scheduler | **临时** |
| 运行状况（临） | event_type 前缀 perf. / runtime.，或模块 performance-monitor | **临时** |
| Lab（临） | 模块 actinglab、actingctl，或 event_type 前缀 lab. / cli. | **临时** |

四个临时页签在标签上带「（临）」后缀，选中时中栏顶部显示「临时归类，待行契约」。
行类型（`acui-rows` 里的 `EventRow`）同样是临时的：**行契约**落地后按契约重写。

## v0 不做的事

- 在线模式（订阅 Runtime、实时跟随）——只有离线文件模式。
- 真实帧图：不载入图像字节，只有哈希占位与几何叠加。
- 任何控制/审批动作。
- 皮肤与主题系统：只用 Slint 自带控件样式，且固定为浅色（见下）。

## 已知取舍

- 界面固定浅色（`fluent-light`）。系统处于深色模式时不跟随：窗口自绘的浅色面板与深色控件
  调色板混在一起会导致白底白字，v0 选择固定浅色而不是引入主题系统。
- 几何叠加从 payload 里按通用键（x/y/width/height、x1..y3）提取；payload 未给画面尺寸时，
  按几何范围铺排并标注「画面尺寸未知」。

## 许可

工作区声明 `AGPL-3.0-only`，每个 `.rs` 文件带 SPDX 头。**最终许可待 Alice 裁定**，
仓库暂未附 LICENSE 全文；裁定后再补。

界面由 [Slint](https://slint.dev) 渲染（Slint 按其许可条款分发）。
