# ActingCommand 监控台

这是 ActingCommand 的**人类监控台**，一个只读的原生程序。它在一个 Runtime 状态根上打开
账本的**正式读面**，把账本自己给出的视图页渲染成三栏界面：实例卡、时间线、详情。

它不是 Runtime 的一部分，是 Runtime 的**外部可拆客户端**。

## 数据来源：读面，不是文件

程序只认一个参数：状态根。**所有文件 IO 都归读面**，监控台自己从不拼接状态根里的路径，
不打开 `ledger/`、`artifacts/` 或 `runtime-state.sqlite`，也不拉起任何 CLI。

读面有两张，同一套查询、页、游标语义，只是答案从哪来不同：

**离线**（原路径，一行未改）：

- `GlobalLedger::open_metadata` 打开状态根，由账本自己判定介质（segment / sqlite）、
  认证快照、给出 `latest_sequence` 与完整性观察。整个会话固定在这一个快照位置上读。
- `actingcommand_ledger_forensics::query_view_page` 出正式页 `RuntimeEventQueryPage`：
  事件、每条事件自带的视图归属、读取范围、页游标、运行恢复分组、产物淘汰事实。
- `actingcommand_ledger_forensics::read_material_to` 读素材：每段由读面解析引用、拿到
  共享读保护、二次核对引用与保留状态，并在整份 sha256 校验通过后才交出这一段字节。

**在线**（经 `actingcommand-runtime-client`，客户端唯一的类型化 IPC 路径）：

- `RuntimeClient::connect(RuntimeClientConfig::new(state_root, Ui, Ui))`：`runtime-info.json`
  由客户端自己读、回环地址由它取、owner epoch 由它在连接时核对。读面只是客户端：
  不杀、不等 Runtime，不碰 `owner.lock`，不往状态根里写任何东西；拉起与请求关闭归
  顶栏的启动器（见「启动器」一节），关闭也只经这同一个类型化客户端。
- 开台第一页不带快照位置去问，Runtime 在页上说出的 `snapshot_ledger_position` 就是这一
  会话固定读的位置——和离线一样，整个会话一个快照。之后每页都是
  `RuntimeClient::query_event_page(query, ProjectionProfile::Ui, page.at_snapshot(pos))`，
  同一个 `EventQuery`、同一个页上限、同一个 `next_cursor`。
- 素材走 `RuntimeClient::read_material`：同样的 `RuntimeMaterialReadRequest`、同样的
  分段与整份校验，只是校验由 Runtime 做，在同一条连接上。
- 介质、损坏尾部、写入进程记录是离线读面对文件的观察，Runtime 不在页上说这些；在线时
  实例卡的「存储格式」写「由 Runtime 判定」，「写入进程」写的是所连的 Runtime 本身
  （PID、owner epoch、启动时间，均出自它自己的 `runtime-info.json`）。

`--source <auto|offline|online>`，默认 `auto`：客户端能连上状态根所指的 Runtime 就在线，
否则离线；实例卡第一行「读面」写明选了哪张、为什么（`runtime-info.json` 不存在，或连接
失败的客户端错误码）。`online` 连不上就**带着客户端的错误码直接退出**，不会悄悄改走离线。
两张读面里，连上了却答不出第一页的 Runtime 在任何模式下都是错误，不回退。

依赖钉在 Runtime **main** 上（`Cargo.toml`）：

```
rev = "c30c3c45aae8b06b96feca15c5ec9fbb744a70a7"
```

四个 crate（contract / ledger / ledger-forensics / runtime-client）共用这一个 rev。
`Cargo.lock` 入库，CI 在 windows-latest 与 ubuntu-latest 上跑
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
acui [--state-root <state_root>] [--source <auto|offline|online>] [--tab <events|observation|changes|errors|health|lab>] [--lang <zh|en>]
acui --help
```

`--state-root` 只对这一次运行有效，覆盖设置文件里的 `state_root`；两处都没有就打印用法退出，
不猜默认值。`--source` 选读面（见上）。`--tab` 指定启动页签（截图与复核用），取值就是视图
自己的 wire 名。`--lang` 只对**这一次运行**有效，覆盖设置文件里的语言，不写回设置文件。

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
state_root = 'D:\ActingCommand\state'                    # 可选，绝对路径
actingd_config = 'D:\ActingCommand\actingd.toml'         # 可选，绝对路径
actingd_exe = 'D:\ActingCommand\actingcommand-actingd.exe'   # 可选，绝对路径
```

开台时读一次，下拉框一改就写一次；写回时三个路径键原样保留。**这是监控台唯一自己读写的
文件**：它不在任何状态根里，状态根依旧全归读面。文件不存在、读不出来或取值不认识，都按
默认值（中文、标准）来。解析器是手写的：一行一个 `key = value`，去掉一对成对的引号，不处理
转义——Windows 路径写在单引号里（TOML 字面量字符串），不要写 `"D:\\…"`。

## 启动器

顶栏第三行。左边是上一次探测的 Runtime 状态，两个按钮，下面一行是上一次按钮的结果。

- **Runtime 状态**：开台时探测一次，之后每按一次按钮再探测。探测就是一次
  `RuntimeClient::connect`：连上了写「运行中 · PID · owner epoch」（取自它自己的
  `runtime-info.json`，经客户端的 owner epoch 核对）；连不上写「未运行」加客户端的错误码与
  操作名，不猜原因。
- **启动**：先探测，已在运行就只写「已在运行，未拉起」。否则按 `actingd_exe` 分离拉起
  `actingcommand-actingd --config <actingd_config>`——命令行就这一条；两个键缺一个或不是
  绝对路径，都写明是哪个键，什么也不拉。stdout / stderr 都进监控台**自己**目录下的日志：
  `%LOCALAPPDATA%\ActingCommand\logs\actingd-<unix_ms>.log`（Linux：`$XDG_STATE_HOME` 或
  `$HOME/.local/state` 下同名路径），目录由监控台建，**永不在状态根里**。Windows 下用
  `DETACHED_PROCESS` 拉起：守护进程不继承监控台的控制台，收不到它的 Ctrl+C。
- **就绪判定**：最多 60 次、每次 500 ms。每次先 `try_wait()`：子进程已退出就停下，写退出码与
  日志路径；没退出就再 `connect` 一次，连上即就绪，写 PID 与 owner epoch；本台若按离线读，
  追加一句「要在线读请带 `--source online` 重启」——**不会在会话中途悄悄换读面**。60 次都
  没连上，写「仍未就绪」和最后一次客户端错误码。**不解析守护进程的输出**。
- **请求关闭**：只走类型化客户端，从不杀进程。新开一条连接，`begin_interaction()` 开一个
  交互，先用 `record_client_action_receipt` 把这次按钮记成 `client_action`（surface
  `acui.launcher`、control `request_shutdown`），拿到带 terminal 的回执后再发
  `request_shutdown()`——动作先落账，再请求。受理了写回执状态、请求编号、动作落账的序号；
  被拒（owner / governance 等）就把 Runtime 的拒绝码**原样**写出，外加客户端错误码与操作名；
  **不重试**。之后再探测一次状态——Runtime 按自己的节奏停，这一眼可能还写着运行中。
- **永不杀**：`Child` 句柄只用来 `try_wait()` 看有没有早退，不 `kill`、不阻塞 `wait`、不挂
  job object；就绪判定结束就丢掉句柄，守护进程活得比监控台久。

暂停/恢复、解锁 owner、开机自启、安装器、联网下载都不在这一片里。

## 安装引导程序 acsetup

`crates/acui-setup` 是一个独立的二进制 `acsetup.exe`（Slint 窗口，与监控台同一套样式与图标），把
伞仓 [Releases](https://github.com/HS7097/ActingCommand/releases) 里的发布件装成一份**按用户**的安装。
它随 UI 仓的 Windows 构建产物一起发布（`acui-windows-<sha>.zip` 里多一个 `acsetup.exe`）。
**v1 离线**：程序里没有任何联网代码，发布件由人先下载到一个文件夹。一个窗口，上一步 / 下一步，六步：

0. **准备**：安装根（可改，默认 `%LOCALAPPDATA%\Programs\ActingCommand`，不需要管理员）、该卷的
   可用空间、此处是否已有安装（看 `runtime\BUILD-MANIFEST.json`；已有就停在这一步——v1 没有升级
   流程，换一个根）、发布件所在文件夹（默认 `%USERPROFILE%\Downloads`，可改；v1 没有原生目录对话框，
   路径直接填）。
1. **校验**：要求文件夹里有 `SHA256SUMS`、`MEMBERS.json`、`actingcommand-runtime-<sha>.zip`、
   `actingcommand-tools-<sha>.zip`、`acui-windows-<sha>.zip`（`<sha>` 取 `MEMBERS.json` 的
   `runtime_sha` / `ui_sha`，三个 zip 必须在 `SHA256SUMS` 里）。逐条核对 `SHA256SUMS`；解压到安装根
   下的临时目录 `.staging-<unix_ms>`；再按每个 zip 自带的 `BUILD-MANIFEST.json` 核对来源仓、提交号
   （等于 MEMBERS 的 sha）、Runtime 的 `runtime_payload_layout`（`distribution-v1`），以及 `files[]`
   每一项的大小与 sha256；zip 里多出清单没列的文件也算不一致。任何不一致都停下，措辞是
   「内容与创建时不一致」——这是完整性陈述，不是授权口吻。校验期间不运行 zip 里的任何东西。
2. **铺开**：`runtime\`（Runtime 全部载荷 + 清单，`actingd.config.example.json` 逐字节原样）、
   `ui\`（监控台载荷 + 清单）、`tools\`（**只有** `actinglab.exe`、`actingledger.exe`、
   `ac_fastdeploy_ppocr.dll`；tools 包里另外两个 exe 不装、不显示）。之后删除临时目录。
3. **配置**：状态根默认 `<安装根>\state`（必须不存在或为空目录，**已有内容的状态根一律不接管**）；
   生成 `secret_fingerprint_salt` = 系统随机源 32 字节的十六进制（`getrandom`；**不显示、不写日志**）；
   写 `<安装根>\actingd.config.json`，字段只有 `schema_version`、`state_root`、`bind_host`
   （127.0.0.1）、`bind_port`（0）、`secret_fingerprint_salt`、`instances`（空）——Runtime 的解析器
   `deny_unknown_fields`，多一个字段都不写。再写监控台设置 `%APPDATA%\ActingCommand\acui.toml` 的
   `state_root`、`actingd_config`、`actingd_exe`（同「设置文件」一节的格式，单引号字面量；已有的
   `lang` / `text_size` 原样保留）。写法与 `crates/acui-app/src/settings.rs` 一致，但 `acui-setup`
   不依赖 `acui-app`，是一份小的重复写入器。**实例（模拟器 / 设备）不在引导里配置**，`instances`
   留空，之后在监控台里添加。
4. **开机自启**（可选，默认不勾）：勾了才写按用户的启动文件夹里的
   `%APPDATA%\Microsoft\Windows\Start Menu\Programs\Startup\ActingCommand.cmd`，内容是
   `start "" "<安装根>\runtime\actingcommand-actingd.exe" --config "<安装根>\actingd.config.json"`；
   再勾「同时拉起监控台」才多一行 `start "" "<安装根>\ui\acui.exe"`。批处理里不出现 acsetup。
   不勾就什么也不写；启动文件夹里已有的同名文件不动，只在完成页说一句。
5. **完成**：「启动监控台 / Open console」分离拉起 `<安装根>\ui\acui.exe`（从不直接拉 actingd，
   Runtime 由监控台的启动器拉）并关闭引导；「完成」只关闭。

**安装日志**：从第 1 步起每一步都往 `<安装根>\acsetup-<unix_ms>.log` 追加人话行；失败时最后一行写
原因，窗口上显示日志路径。除安装载荷、配置、设置与（勾选时的）自启批处理之外，引导写的文件只有这一个。

**永远不做的事**：不装服务、不建计划任务、不改 PATH、不写注册表；不改配置模板；不碰已有内容的状态根；
不配置实例；不联网；不做升级、不装资源包。Linux 上 crate 照常编译（CI 两条腿都跑 `--workspace`），
运行即以 `acsetup v1 is Windows-only` 退出。

依赖只多三个，都在 `[workspace.dependencies]` 里注明用途：`sha2`（校验）、`zip`
（`default-features = false`，只开 `deflate`，与 Runtime 锁定的同一版本线）、`getrandom`（salt）。

## 四层四 crate

同一个 Cargo workspace，依赖方向 app → model → rows ← source：

- `acui-rows`：唯一为视图模型命名契约类型的地方；再导出契约类型，外加显示用函数、显示名
  字典，与两个由 `acui-source` 填、`acui-model` 读的平铺结构。
- `acui-source`：读面，唯一碰状态根的地方。离线 `EvidenceSource::open` / `query` /
  `open_report` / `read_material` 原样保留；`ReadSource::open(root, mode)` 按 `--source`
  在它和在线的 `OnlineSource` 之间选一张，`material_reader()` 交给后台线程读素材。
- `acui-model`：纯 Rust 视图模型（页签、过滤、翻页、恢复折叠、选中项），不依赖 slint，**也不
  出人话**——它只给结构化事实，措辞一律由 `acui-app` 按语言表挑。
- `acui-app`：唯一依赖 slint 的 crate，`.slint` 文件在 `crates/acui-app/ui/`；两张语言表在
  `strings.rs`，设置文件的读写在 `settings.rs`。

`slint` 1.17.x，`default-features = false`；账本只读，控制入口只有启动器的两个按钮（启动 /
请求关闭，见上），没有审批入口；不写测试。启动器在 `crates/acui-app/src/launcher.rs`，探测与
请求关闭这两个客户端操作在 `acui-source`（`probe_runtime` / `request_shutdown`）。

第五个 crate `acui-setup`（二进制 `acsetup`）在这四层之外：安装引导程序，只依赖 slint、serde、sha2、
zip、getrandom，不依赖上面任何一层，见上一节「安装引导程序 acsetup」。

## 图标

应用图标是 Alice 裁定的黑色单人「指挥官」标记，素材在 `crates/acui-app/assets/`：
`acui-256.png`（256×256 透明 PNG）与 `acui.ico`（16..256 多尺寸）。

- **窗口与任务栏图标**：`app.slint` 的 `Window.icon: @image-url("../assets/acui-256.png")`。
- **可执行文件图标**：`build.rs` 里 `#[cfg(windows)]` 调 `winresource` 把 `acui.ico` 编进
  exe 资源段；这条依赖挂在 `[target.'cfg(windows)'.build-dependencies]` 下，Linux 上不编译。
- **acsetup**：同一套素材，不复制：`crates/acui-setup/build.rs` 与 `ui/setup.slint` 用相对路径指向
  `crates/acui-app/assets/` 里的这两个文件。

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
crate（contract / ledger / ledger-forensics / runtime-client）为 `AGPL-3.0-only`，两者按 GPLv3 第 13 条合并。
