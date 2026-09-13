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
  - `acui-source`：`EventSource` trait（只有 `source_label`）与 `FileSource`（合并多页、按 sequence 排序去重）。
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
`--help` / `-h` 打印这一行用法后退出。

顶栏左侧是账本事实：`latest_sequence` 与 `event_count` 都来自 `open.json`，不给 `--open` 时显示
「—」；本地实际载入的条数另有标签「事件条数（已载入）」，两者不混用。实例卡在给出 `--open` 时
多一行「账本完整性」：`read_complete` / `corrupt_tail` / `repair_count` 三项齐全且干净时显示
「正常」，否则原样列出三个值。实例卡其余数值都是**已载入范围**的统计，不随时间游标变化。

窗口可缩放：默认 1400×900，最小 1100×700，中栏随窗口伸缩，两侧栏保持定宽。

## 图标

应用图标是 Alice 裁定的黑色单人「指挥官」标记，素材在 `crates/acui-app/assets/`：
`acui-256.png`（256×256 透明 PNG）与 `acui.ico`（16..256 多尺寸）。

- **窗口与任务栏图标**：`app.slint` 的 `Window.icon: @image-url("../assets/acui-256.png")`，
  Slint 在编译期把 PNG 嵌进程序。
- **可执行文件图标**：`build.rs` 里 `#[cfg(windows)]` 调 `winresource` 把 `acui.ico` 编进
  exe 资源段；这条依赖挂在 `[target.'cfg(windows)'.build-dependencies]` 下，Linux 上不编译。

图标是编译期嵌入的自带素材，不是账本素材：**「不载入图像字节」的裁定不变**，帧视图依旧只显示
哈希占位。

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
- 自带皮肤与主题系统：不做调色板设置，只跟随系统（见下）。

## 已知取舍

- 界面不再固定浅色：不钉 `fluent-light`，颜色全部取自 std-widgets 的 `Palette`，跟随平台默认
  样式与系统深/浅色设置。只有 severity 保留语义色（info 绿 / warning 橙 / debug 灰 / error 红），
  这几个中间色在两套调色板上都可读。
- 几何叠加只从 payload 的白名单键（`source_regions`、`action`、`boxes`、`points`、`region`、
  `rect`）下提取 x/y/width/height 与 x1..y3，其他位置的数字不当作几何；payload 未给画面尺寸时，
  按几何范围铺排并标注「画面尺寸未知」。
- 时间游标滑杆用 f32 传递 sequence：超过 2^24 的 sequence 会量化到最近的可表示值，v0 接受这一限制。
- `slint` 按 `default-features = false` 只留 winit 后端、femtovg 渲染器等必需项，裁掉的是用不到的
  渲染器与后端（软件渲染器、测试后端、Linux 托盘）。**图像解码没有被裁掉**：`image` 仍是
  `i-slint-core` 的普通依赖（`cargo tree -p acui-app -e normal -i image` 可见），只是本程序从不
  调用它——没有任何 Image 元素被喂过字节。
- 窄窗口下实例卡与详情的长值（各类 id）改为省略号截断而不是折行：Slint 的按宽定高只在布局求解
  拿得到容器宽度时生效，ScrollView 与嵌套布局里拿不到，折行的第二行不会被预留高度而压到下一行。

## 待 Alice 裁定

- **许可**：工作区声明 `AGPL-3.0-only`，每个 `.rs` 文件带 SPDX 头，但**最终许可待裁定**，
  仓库暂未附 LICENSE 全文；裁定后再补。
- **Slint 许可选项**：界面由 [Slint](https://slint.dev) 渲染，选哪一种 Slint 许可待裁定。
- **帧素材读取**：v0 永不载入图像字节；将来是否、以及如何读取帧素材待裁定。
