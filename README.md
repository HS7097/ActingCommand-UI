# ActingCommand 监控台

这是 ActingCommand 的**人类监控台**，一个只读的原生程序。它在一个 Runtime 状态根上打开
账本的**正式读面**，把账本自己给出的视图页渲染成三栏界面：实例卡、时间线、详情。

它不是 Runtime 的一部分，是 Runtime 的**外部可拆客户端**。

## 数据来源：读面，不是文件

程序只认一个参数：状态根。**所有文件 IO 都归读面**，监控台自己从不拼接状态根里的路径，
不打开 `ledger/`、`artifacts/` 或 `runtime-state.sqlite`，也不拉起任何 CLI。

- `GlobalLedger::open_metadata` 打开状态根，由账本自己判定介质（segment / sqlite）、
  认证快照、给出 `latest_sequence` 与完整性观察。整个会话固定在这一个快照位置上读。
- `actingcommand_ledger_forensics::query_view_page` 出正式页 `RuntimeEventQueryPage`：
  事件、每条事件自带的视图归属、读取范围、页游标、运行恢复分组、产物淘汰事实。
- `actingcommand_ledger_forensics::read_material_to` 读素材：每段由读面解析引用、拿到
  共享读保护、二次核对引用与保留状态，并在整份 sha256 校验通过后才交出这一段字节。

依赖按 **PR398 头**钉在 `Cargo.toml`：

```
rev = "e41467b9f51d68ddf754f1b898ba4747963a9a1f"
```

三个 crate（contract / ledger / ledger-forensics）共用这一个 rev。旁边留了注记：
**PR398 合并后改钉 main**。`Cargo.lock` 入库，CI 在 windows-latest 与 ubuntu-latest 上跑
`cargo build --locked --release --workspace`；闭包里含 `rusqlite`（bundled），两边都要 C 编译器。

## 变了什么

- **归类由契约说了算**：六个页签就是契约的六个 `LedgerView`。一行属于哪个页签，看页里
  那条事件自带的 `views`。页签上的数字**只出现在当前页签上**，是这一视图**已载入的行数**；
  读面没有给出每个视图的总数，所以别的视图的页签上不摆数字。
- **过滤是账本查询**：视图、严重度上下界、来源模块、`correlation_`/`request_`/`run_`/`task_`
  id、时间上界组装成一个 `EventQuery`，在**同一个快照位置**上重新查一次账本；不在本地
  已有的行上筛选然后自称是账本查询。id 必须是完整的规范 id，否则会说明要填完整标识。
  「来源模块」的选项**每次载入都按当前页重建**，选中项按**模块名**解析——列表会变，下标
  不是一个能存住的说法。
- **翻页是页游标**：「继续读」拿页给的 `next_cursor` 取下一页并追加。顶栏常驻
  「读到第 N 条」，源不完整时追加「源不完整」。
- **恢复分组来自账本**：页里带的 `run_recovery` 在各自运行的第一行前插一条分组行，显示
  账本给的状态（已恢复 / 未解决 / 未知）、依据（第 N 条失败，第 M 条已恢复）与缺口；被账本
  判为已恢复的失败行折叠在分组行下，带「已恢复」标记。这是读时分组，不是改写失败事件。
  跨页时按**运行编号合并**：后一页只往里加证据，先前页给出的位置一律保留，已经折叠起来的
  「已恢复」行不会被后一页重新展开。
- **时间上界**：滑块的起始位置就是它代表的状态——最右端即「全部」，第一次拖动是收窄。
- **行类型不再镜像**：`acui-rows` 直接再导出契约类型，只额外提供本地时间、id 缩写、
  wire 码与显示名字典这些显示用函数。

## 帧素材：读了，但只读已校验的

裁定已改：监控台**会**载入帧字节，但只走素材读面，且只在下面这条规则内：

- 只读**选中事件自己**带的 `capture.frame` 产物，按需读，一次一份。
- 按 `MAX_RUNTIME_MATERIAL_CHUNK_BYTES`（64 KiB）分段请求，每段由读面做整份长度与
  sha256 校验；任何一段不是 `verified` 就中止，已拿到的字节全部丢弃。
- 整份上限是契约的段上限 × 128 段（8 MiB）；超出的产物直接拒读并说明。
- 产物已被淘汰时，只显示淘汰事实（处置、意图/结果位置、观察至哪个位置），不去碰文件。
- 读取失败时显示读面给的状态与安全错误码。
- 读取在后台线程里做，每次请求带代号；慢读回来时若选中项已变就丢弃。**任何时候都不会
  显示过期的或未校验的图。**
- 读取由一条后台工作线程做，**同时只有一条**：请求被顶掉的那条在下一段之前就停手，不再
  发下一段——每一段都要把整份素材重新哈希一遍，让一份没人等的读继续跑是最贵的错。
- 解码只用 `image`（只开 `png` feature，版本钉在工作区的 `[workspace.dependencies]`）。程序
  解码的图只有两类：这样读回来的帧，和自带的应用图标。

几何叠加与帧共用同一个坐标系：payload 给了画面尺寸就用它，没给就用解码出的像素尺寸。

## 运行

```
acui --state-root <state_root> [--tab <events|observation|changes|errors|health|lab>] [--lang <zh|en>]
acui --help
```

`--tab` 指定启动页签（截图与复核用），取值就是视图自己的 wire 名。`--lang` 只对**这一次
运行**有效，覆盖设置文件里的语言，不写回设置文件。

窗口可缩放：默认 1400×900，最小 1100×700，中栏随窗口伸缩，两侧栏保持定宽。窗口跟随系统
DPI；程序自己不设缩放。

## 界面语言与字号

顶栏右侧两个下拉框，选完就写进设置文件：

- **字号**：标准 / 大 / 特大 = 1.0 / 1.25 / 1.5。**当场生效**。界面里所有字号与行高都是一个
  给正常 DPI 下的人看的基准尺寸（列表行 14px、详情 13px、小标题 16px、行高 26px）乘这一个
  系数，`app.slint` 里只有 `Scale.factor` 这一个旋钮。
- **语言**：中文 / English。**重启后生效**，框旁边就写着这句话。两张语言表在
  `crates/acui-app/src/strings.rs`（`ZH` 与 `EN`），开台时按选定的那张填一次 `Strings` 全局，
  之后不再重排——没有 gettext，也没有运行时换词。

两层标签：上层是给人看的名字，下层灰色的是程序里的原样写法——原始 `event_type`、模块名、
各种 id、`payload_schema`、sha256 一律不翻译。字典在 `crates/acui-rows/src/display.rs`，覆盖
契约里全部 115 个 `event_type` 与 19 个 `origin.module`；**表里没有的一律照原样显示，不猜**。

### 设置文件

语言与字号存在按用户区分的配置目录下：

```
Windows:  %APPDATA%\ActingCommand\acui.toml
Linux:    $XDG_CONFIG_HOME/ActingCommand/acui.toml（没有就用 $HOME/.config/…）
```

```toml
lang = "zh"          # zh | en
text_size = "standard"   # standard | large | extra-large
```

开台时读一次，下拉框一改就写一次。**这是监控台唯一自己读写的文件**：它不在任何状态根里，
状态根依旧全归读面。文件不存在、读不出来或取值不认识，都按默认值（中文、标准）来。

## 四层四 crate

同一个 Cargo workspace，依赖方向 app → model → rows ← source：

- `acui-rows`：唯一为视图模型命名契约类型的地方；再导出契约类型，外加显示用函数、显示名
  字典，与两个由 `acui-source` 填、`acui-model` 读的平铺结构。
- `acui-source`：读面，唯一碰状态根的地方。`EvidenceSource::open` / `query` /
  `open_report` / `read_material`。
- `acui-model`：纯 Rust 视图模型（页签、过滤、翻页、恢复折叠、选中项），不依赖 slint，**也不
  出人话**——它只给结构化事实，措辞一律由 `acui-app` 按语言表挑。
- `acui-app`：唯一依赖 slint 的 crate，`.slint` 文件在 `crates/acui-app/ui/`；两张语言表在
  `strings.rs`，设置文件的读写在 `settings.rs`。

`slint` 1.17.x，`default-features = false`；只读，没有任何控制按钮或审批入口；不写测试。

## 图标

应用图标是 Alice 裁定的黑色单人「指挥官」标记，素材在 `crates/acui-app/assets/`：
`acui-256.png`（256×256 透明 PNG）与 `acui.ico`（16..256 多尺寸）。

- **窗口与任务栏图标**：`app.slint` 的 `Window.icon: @image-url("../assets/acui-256.png")`。
- **可执行文件图标**：`build.rs` 里 `#[cfg(windows)]` 调 `winresource` 把 `acui.ico` 编进
  exe 资源段；这条依赖挂在 `[target.'cfg(windows)'.build-dependencies]` 下，Linux 上不编译。

## 读面挡住的事

这些不是绕过去了，是照实显示、在此记账：

- **`event_count` 与 `repair_count` 没有**。`GlobalLedgerMetadata`
  （`crates/ledger/src/global/evidence.rs:257`）只给 `latest_sequence` / `read_complete` /
  `backend` / `writer_metadata` / `corrupt_tail`，没有事件条数与修复条数的访问器；唯一给出
  这两项的 `GlobalLedger::open_evidence`（同文件 `:417`）要求调用方为每个产物引用交出
  `VerifiedArtifactReference`，验证不了的事件会被丢掉（在 0828 根上实测 2585 条只剩 10 条），
  等于开台就要把整个 artifacts 目录（457 MB）全哈希一遍。实例卡因此把 `event_count` 显示为
  「—（读面未给，见 README）」，另外标出**本视图已载入**的条数，两者不混用。
- **整份素材没有入口，读一帧很贵**。`crates/ledger-forensics/src/material.rs:51` 的
  `read_material_to` 只做一段，且每段都要重开两次账本元数据并把整份素材重新哈希一遍；
  读一张 3.6 MB 的帧要 57 段，实测约 5 秒（release）。没有整份读入口，也没有跨段复用的
  reader，所以监控台把读取放进后台线程，而不是自己去拼一套简化的读取流程。
- **几何与帧在这两个根上凑不到一起**。0828 与 v5 两个根里，带 `capture.frame` 产物的事件
  只有 `artifact.created` / `artifact.verified`，payload 里没有几何；带几何的事件只有
  `task.effect_intent`（0828 六条、v5 五条），payload 里是一个 tap 坐标，`links` 里**没有**
  `frame_id`。账本没有给出把这两者连起来的关系，监控台就不连——真实帧照画，叠加为空。
- **两个根里都没有产物淘汰事实**，所以淘汰占位在这两个根上不会出现；代码路径按契约写好。

## 许可

`GPL-3.0-only`（Alice 2026-09-17 裁定）。仓库附 LICENSE 全文；工作区 `license` 字段与每个 `.rs` / `.slint`
文件的 SPDX 头与之一致。界面由 [Slint](https://slint.dev) 渲染，按其 GPLv3 许可选项使用。依赖的 Runtime
crate（contract / ledger / ledger-forensics）为 `AGPL-3.0-only`，两者按 GPLv3 第 13 条合并。
