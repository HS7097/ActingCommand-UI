<p align="right">🌐 <a href="./README.md">English</a> · <b>简体中文</b></p>

# ActingCommand 监控台

**⚠️ 预发布，调试阶段：接口、配置与文件格式还会变，只支持最新版。见「开发状态」。**

这是 ActingCommand 的**人类监控台**（即控制台 `acui`），一个只读的原生程序。它在一个 Runtime 状态根上打开
账本的**正式读面**，把账本自己给出的视图页渲染成三栏界面：实例卡、时间线、详情。

它不是 Runtime 的一部分，是 Runtime 的**外部可拆客户端**。

本仓还放着**安装向导** `acsetup`（见「安装向导 acsetup」一节）与固定入口 `acforward`——A/B 安装经它拉起
各个程序。与 Runtime 一样，这些程序不认识任何游戏：不含游戏逻辑，也不含游戏数据。游戏经 Runtime 加载的
资源包接入（每个游戏一个标准包）；向导只读包里声明的内容，什么也不猜。

## 当前发布

- **UI v0.11.3**，与此前每一版一样是预发布：本仓 [Releases](https://github.com/HS7097/ActingCommand-UI/releases)
  上的 `acui-windows-<sha>.zip` 与 `SHA256SUMS`。zip 里有 `acui.exe`（监控台）、`acsetup.exe`（安装向导）、
  `acforward.exe`（固定入口）、`LICENSE`、`README.md`，以及把它们绑到所在提交的 `BUILD-MANIFEST.json`。它基于
  Runtime v0.11.3 的 Runtime crate 构建（钉点见「数据来源」一节）。
- **v0.11.3 新增**：能否搭配按各组件声明的接口判断，不再看版本号（不相容时退出码 6，见「组件接口」）；认得 Tools
  布局第 3 版（带看门狗启动器 `actingwatch.exe`）；账本升到修订 2 之后，拒绝回退到 Runtime v0.11.2 的槽（退出码 1、
  不做改动）；只更新资源 `acsetup --resources <zip>`，第 0 步也可勾选；actingd 经 WMI 以隐藏窗口拉起，启动即退出时
  报错带上日志里的 `FATAL` 行；升级时刷新字节有变化的固定入口。
- **可与 Runtime v0.11.6 搭配**：acsetup v0.11.3 已把一份 A/B 安装升级到它。
- **从哪里取**：伞仓 [Releases](https://github.com/HS7097/ActingCommand/releases) 上有两个安装器，在线版
  `acsetup.exe` 与离线版 `acsetup-full-<tag>.exe`，各带 `.sha256`。伞仓最新发布 v0.11.4 带 Runtime v0.11.4、本仓
  UI v0.11.3 与可用的标准包。Runtime v0.11.5 与 v0.11.6 在
  [Runtime 仓的 Releases](https://github.com/HS7097/ActingCommand-Runtime/releases) 上，还没有进伞仓发布，所以
  目前在线向导装上的是 Runtime v0.11.4。

## 开发状态

- **调试阶段**：主循环是「发现问题 → 修复 → 对照预期检查 → 再改」；部署与可用性是次要的。
- **全部是预发布**：接口、配置与文件格式还会变。**只支持最新版**，旧系列不出修复（不维护 0.11 系列）。
- **会有破坏性变化**，随 0.12 系列到来：0.12.0 起用新账本（0.11 的状态根不带过去）、命令行输出与退出码变化、
  MCP 档位取消、Lab 默认不装。安装与查看 Runtime 0.12.0 预计要等下一版 UI。见「计划中（未发布）」。
- **实机现状**：自 10 月上旬起，在真实 MuMu 实例上按目录、用可用的标准包每天跑例行批次。标准包的内容覆盖还不
  完整，多日无人值守长跑仍在验证。
- **版本号规则**：X 重大或不兼容、Y 新特性或新覆盖面、Z 修复（含为修复服务的特性）。Runtime、UI 与各标准包
  各自发版、只在自身有变化时发；能否搭配看各自声明的接口，不看版本号是否一致。

## 数据来源：读面，不是文件

程序只认一个参数：状态根。**读账本数据的文件 IO 都归读面**，监控台自己从不拼接状态根里的路径，
不打开 `ledger/`、`artifacts/` 或 `runtime-state.sqlite`，也不为读数据拉起任何 CLI。它拉起的
进程只有三种，都在「启动器」与「实例配置」两节里写明：actingd 本身（「启动」），经两次确认的
`actingd unlock-owner`（解锁，见「启动器」一节），以及保存实例配置时用来校验的
`actingd check-config`（见「实例配置」一节）。

读面有两张，同一套查询、页、游标语义，只是答案从哪来不同：

**离线**（原路径）：

- `GlobalLedger::open_metadata` 打开状态根，由账本自己判定介质（segment / sqlite）、
  认证快照、给出 `latest_sequence` 与完整性观察。整个会话固定在这一个快照位置上读。
- `actingcommand_ledger_forensics::query_view_page` 出正式页 `RuntimeEventQueryPage`：
  事件、每条事件自带的视图归属、读取范围、页游标、运行恢复分组、产物淘汰事实。
- `actingcommand_ledger_forensics::read_material_complete` 整份读素材：由读面解析引用、拿到
  共享读保护、二次核对引用与保留状态，并在整份 sha256 校验通过后才交出字节；从不交出前缀。

**在线**（经 `actingcommand-runtime-client`，客户端唯一的类型化 IPC 路径）：

- `RuntimeClient::connect(RuntimeClientConfig::new(state_root, Ui, Ui))`：`runtime-info.json`
  由客户端自己读、回环地址由它取、owner epoch 由它在连接时核对。读面只是客户端：
  不杀、不等 Runtime，不碰 `owner.lock`，不往状态根里写任何东西；拉起与请求关闭归
  顶栏的启动器（见「启动器」一节），关闭也只经这同一个类型化客户端。
- 开台第一页不带快照位置去问，Runtime 在页上说出的 `snapshot_ledger_position` 就是这一
  会话读的位置，即钉点。人按「跳到最新」或开启跟随时，同样用一张新的第一页来移动它；跟随中每次
  轮询把它移到 Runtime 事实快照说出的位置（见下）。离线从不移动。之后每页都是
  `RuntimeClient::query_event_page(query, ProjectionProfile::Ui, page.at_snapshot(pos))`，
  同一个限定到一窗的 `EventQuery`、同一个页上限、同一个 `next_cursor`。
- 素材走同一条连接上的 `RuntimeClient::read_material_complete`：客户端自己分段读（每段
  192 KiB，遇到拒收大段的 Runtime 退到 64 KiB），每段由 Runtime 对整份校验，拼完后客户端再核
  一次总长与 sha256——结果形状与离线整份读相同。监控台不再自己拼段。
- 介质、损坏尾部、写入进程记录与修复条数是离线读面对文件的观察，Runtime 不在页上说这些；
  在线时实例卡的「存储格式」写「由 Runtime 判定」，修复条数写 Runtime 不提供，事件条数就是钉住
  的位置（契约规定序号从 1 起无缺口，所以某位置上的条数就是该位置），「写入进程」写的是所连的
  Runtime 本身（PID、owner epoch、启动时间，均出自它自己的 `runtime-info.json`）。
- 钉住之后，立刻在同一条连接上读一次 `status()` 和一次 `runtime_fact_snapshot()`，给实例卡
  「运行时实例」几行。状态只在人按「跳到最新」时重读；跟随时每一次轮询都只重读任务事实，与钉点是否移动无关。
  头两行写两次读各自所在的序号；两者都可能晚于钉住的快照，所以这是最近一次读到的状态，不是钉点上
  的状态。按契约，Runtime 会把这次状态读取本身记
  成一条观察事件（`command.validated`），落在钉点之后，不在本会话的快照内。随后对状态里登记的每个
  实例写：别名（灰字是实例编号）、端口、租约（占用中 / 接管冷却中 / 空闲，有排队请求时加上排队
  数），以及实例事实 `task.game`、`task.server`、`task.page`（最近一次识别匹配到的页面标签）原样，
  没有就写「未记录」。快照里有事实、状态却没登记的编号，单独一行写明未登记。任一次读失败，就分行
  写 Runtime 拒绝码、客户端错误与宿主失败，会话照常打开。离线时由读面的 `runtime_facts_at` 在钉住的
  位置本身重放事实库，就是每一页读的那个位置：几行写出这个位置、离线没有租约，以及每个实例的编号
  （缩写，灰字是完整编号）与三项事实。账本里还没有事件时直说；读面拒绝重放时写它给的原因，重放
  失败就分行写 code、operation、IO 类别（io 错误时）与 detail。重放在会话打开时、窗口出现之前跑
  一次，期限 30 秒。

`--source <auto|offline|online>`，默认 `auto`：客户端能连上状态根所指的 Runtime 就在线，
否则离线；实例卡第一行「读面」写明选了哪张、为什么（`runtime-info.json` 不存在，或连接
失败的客户端错误码）。`online` 连不上就**带着客户端的错误码直接退出**，不会悄悄改走离线。
两张读面里，连上了却答不出第一页的 Runtime 在任何模式下都是错误，不回退。

离线读面打不开时——`offline`，或 `auto` 退到离线——监控台不退出，窗口以**未打开**状态开台；
刚装好的机器就是这样，账本要等 Runtime 第一次启动才建。这时实例卡只有第一行「读面」：写明账本
未能打开，并原样写出读面自己的 `code`、`operation` 与 `detail`；io 错误另写 IO 类别
（`LedgerIoKind`：`not_found`、`permission_denied` 等）。分辨「还没有账本」和「账本在但读不了」靠的
是这个类别，从不解析本地化的 `detail`：只有 `ledger_io` 且 IO 类别为 `not_found` 时，才补一句状态根里
还没有账本、多半是 Runtime 从未在这里启动过，并指向启动器的「启动」。不显示任何账本事实，连 0 也
不写：列表写账本未打开，不留一片空白；页签、过滤框、编号框和时间滑块都停用；模块框、端口框与帧区写「账本未打开」。不向账本发
任何查询，不读任何素材。启动器照常可用。

依赖钉在一个准确的 Runtime 源提交上（`Cargo.toml`），即 Runtime v0.11.3 的提交：

```
rev = "484bdc14fcacbb2787707a5f03e65acdb8f9ff1a"
```

五个 crate（contract / ledger / ledger-forensics / runtime-client / execution-kernel）共用这一个 rev。
内嵌 typed client 读取在线 lifecycle-failure 页中的可选 `resource_dispositions` 分组；离线 ledger
reader 支持完整 failure 与 `ResourceQuiescence` 记录中的该分组。未含该字段的历史记录按原语义读取，
严格校验和显式读取错误保持。
UI 仍使用 `ProjectionProfile::Ui`：已提供的 failure 分组按原隐私规则显示，observed 阶段的分组
由完整事件/Forensic 读取，UI 投影不含该分组。
内嵌 ledger reader 同时使用共享恢复生命周期契约：准备触发关联其准备事件，任务触发保留真实 task/run
关联；`environment_ready` 与资源包 `recovered` 分别表示各自结果。这些细节属于完整生命周期记录，
公开 UI 投影及页内 `run_recovery` 分组保持既有形状。发布时 producer、Tools 与 UI 须配套使用这一契约。
`Cargo.lock` 入库，CI 在 windows-latest 与 ubuntu-latest 上跑
`cargo build --locked --release --workspace`；闭包里含 `rusqlite`（bundled），两边都要 C 编译器。
CI 编译 PR 与推送到 `main` 以外分支的提交；推送 `main` 不编译。发版只按需：Actions → release → Run workflow
（或 `gh workflow run release.yml -f bump=patch|minor|major [-f version=X.Y.Z] [-f source_sha=<sha>] [-f prerelease=true] [-f dry_run=true]`）
编译选定的 `main` 提交，把 `acui-windows-<sha>.zip` 连同 `SHA256SUMS` 发布为本仓 Release `vX.Y.Z`，tag 建在该提交上；
勾选 `prerelease`（`-rc.N` 版本总是如此）时发为预发布，不标为 Latest；版本号只存在于 tag。

## 变了什么

- **归类由契约说了算**：六个页签就是契约的六个 `LedgerView`。一行属于哪个页签，看页里
  那条事件自带的 `views`。页签上的数字**只出现在当前页签上**，是这一视图**已载入的行数**；
  读面没有给出每个视图的总数，所以别的视图的页签上不摆数字。
- **过滤是账本查询**：视图、严重度上下界、来源模块、`correlation_`/`request_`/`run_`/`task_`/
  `instance_` id、时间上界组装成一个 `EventQuery`，在**同一个快照位置**上重新查一次账本；不在
  本地已有的行上筛选然后自称是账本查询（唯一写明的例外是下面的性能监视）。id 必须是完整的规范 id，否则会说明要填完整标识。
  「来源模块」的选项**每次载入都按已载入的行重建**，选中项按**模块名**解析——列表会变，下标
  不是一个能存住的说法。
- **实例按端口筛选（只在离线读面）**：ADB 端口就是一个模拟器实例的身份。开台时按同一快照位置
  经 `actingcommand_ledger_forensics::instance_bindings` 把 `runtime.instance_bound` 事实读**一次**，
  得到每个端口下**曾经绑定过的全部** `instance_id`（按首次绑定顺序）。「端口」下拉框选一个端口，
  就把这一整组编号作为 `EventQuery.instance_ids` 在同一快照上重新查一次账本——整组一起查，
  从不拆开、从不只查一部分；选「全部实例」清掉；下拉框的项按端口升序，写端口号与最新一次绑定
  的实例编号缩写，同一端口不止一个编号时追加 ` +n`。端口与 `instance_` 编号只能选一：两个都给了，
  过滤错误行直接说明，不做默认优先。账本里没有绑定事实时框里只有「无绑定记录」一项且不可用；
  有绑定事实但没有一条给出端口（串口配置等）时框里只有「全部实例」且不可用——两种情形编号框都
  仍可直接填 `instance_` 编号；读绑定失败时框里只有读面的错误码，实例卡也写这个码；
  在线读面下框不可用，写「在线态不支持」。选中端口时实例卡多写实例别名、端口、来源
  （`physical_device` / `fixture_simulation` 原样在下层）、绑定编号数与最新绑定序号；级别计数与
  条数仍来自重查后的行。行多一列「端口」：事件没有实例关联的写「宿主」；有关联且该编号最新
  一次绑定给了 HOST:PORT 的写端口号；有关联但端口表里没有的（夹具、串口配置、未见绑定）写缩写
  的实例编号——不编造端口。
- **从最新一端读起**：账本查询没有降序，所以时间线从钉住的位置往回一窗一窗读，一窗先从 256 个位置
  开始。这么宽的一窗里事件不多于一页的条数上限，一次查询就能读完，各项过滤都由账本做。一窗读回的
  事件少于 64 条时，下一窗宽度加倍，最宽 4096 个位置；读回 128 条以上就回到 256。更宽的窗，以及
  回复被字节上限拆页的窗，都跟着游标整窗读完——所以一次填充可能多于目标行数。在线时每次页查询
  都在 Runtime 的账本写线程上做（Runtime `71db072d` 起只复核头部、边界与新增尾部，一万条时一页约
  15 ms；此前要把整本账读一遍、校验一遍），所以一次填充至少读一窗，多出 256 行、读到位置 1、读满 16 窗或已过 1 秒之后，就不再开始新的一窗。行按**最新在上**排列；底部
  的「读更早」接着往下读。顶栏常驻已读窗口覆盖
  的位置（「已读序号 A–B」），有窗口的页说读取不完整时追加「源不完整」。
- **跳到最新、跟随最新（只在在线读面）**：「跳到最新」用一张新的第一页把钉点移到 Runtime 的最新位置，
  重读状态与事实（状态读取会在账本留一条观察事件），去掉时间上界，从新钉点重新读起。「跟随最新」先
  同样对齐但不读状态，之后每 5 秒读一次 Runtime 的事实快照——它在内存里的事实库，位置就是账本的最新
  位置，不读账本也不写账本。只有这个位置动了，钉点才移过去，并用页查询把新旧钉点之间的窗口读到视图
  顶部；已载入的行、选中项和正在读的帧都保留，任务事实直接取自同一张快照。跟随中设了时间上界会停止
  跟随。时间跨度的终点跟着钉点走：新窗里有钉点上那条事件就直接取它的时间，否则再读一条。某次跟随
  失败时，在视图自己的错误旁写「跟随最新：…」。轮询失败的，等后面某次跟随越过它才消失；页读取失败的，
  还会停止跟随——不对一个拒绝查询的 Runtime 每 5 秒再问一次——直到重新开启跟随。离线没有
  在跑的 Runtime 写新事件，两个控件都不可用。
- **性能监视的例行事件默认隐藏**：它们会淹没别的事件。除非勾选「显示性能监视」、在模块框里选了
  性能监视、或打开的是由这些事件组成的「健康」页签，查询请账本直接不返回在所有写入方那里都恒为
  `Info` 的两种类型——`perf.pressure_ended`、`perf.monitor_recovered`（`exclude_event_types`）——监控台再在
  读每一窗时丢掉性能监视其余**低于警告级别**的事件，`perf.summary` 也在其中。`perf.summary` 留给监控台
  处理，因为容量监视在磁盘有压力时会把它写成警告或错误。顶栏写明丢了多少条；账本不返回的那两种不计数。
  有丢弃时性能监视留在模块列表里，仍可选中。它的警告和错误（磁盘压力、高压力、卡顿、监视降级、有压力
  时的小结）始终显示。实例卡上的条数与级别计数只算已载入的行。`51ba5565` 之前的
  Runtime 不认识 `exclude_event_types`，隐藏时会拒绝查询；勾选「显示性能监视」即可读。
- **恢复分组来自账本**：页里带的 `run_recovery` 在各自运行最新的一行上方插一条分组行，显示
  账本给的状态（已恢复 / 未解决 / 未知）、依据（第 N 条失败，第 M 条已恢复）与缺口；被账本
  判为已恢复的失败行折叠在分组行下，带「已恢复」标记。这是读时分组，不是改写失败事件。
  跨页时按**运行编号合并**：后一页只往里加证据，先前页给出的位置一律保留，已经折叠起来的
  「已恢复」行不会被后一页重新展开。
- **时间上界**：滑块的起始位置就是它代表的状态——最右端即「全部」，第一次拖动是收窄。设了上界时，
  读取只往下走，所以起点必须高于所有匹配的事件：先按已提交的时间跨度、假设事件在时间上均匀来估
  上界的位置，再多留两窗余量，然后用视图自己的查询做一次探测，问起点之上第一条匹配的事件。没有：
  就从这里读起。有：起点移到它之上，步长每次加倍，在一秒预算内最多探测三次，仍不行就退回钉点。实际显示什么仍
  由查询自己的时间上界决定，顶栏写明实际读了哪些位置。上界与标签随手柄即时变化；手柄停住 0.4 秒后
  视图才重新读取（其间切页签或点「读更早」会先应用新上界）。
- **行类型不再镜像**：`acui-rows` 直接再导出契约类型，只额外提供本地时间、id 缩写、
  wire 码与显示名字典这些显示用函数。

## 帧素材：读了，但只读已校验的

监控台**会**载入帧字节，但只走素材读面，且只在下面这条规则内：

- 只读**选中事件所基于的那一帧**的 `capture.frame` 产物（见下文「每一行都落到帧上」），按需读，
  一次一份。
- 离线一次 `read_material_complete` 调用读整份，整份长度与 sha256 校验通过后才交出任何字节，
  期限 30 秒。在线由 `RuntimeClient::read_material_complete` 在 Runtime 校验过的分段上做同样的
  事，期限相同（在段与段之间检查）；任何一段不是 `verified` 就停下，没拼完的部分丢弃。
- 整份上限两张读面都是 8 MiB，即这次读的 `max_material_bytes`（监控台还自己拼 64 KiB 分段时
  定下的上限，128 段）；超出的产物直接拒读并说明。
- 产物已被淘汰时，只显示淘汰事实（处置、意图/结果位置、观察至哪个位置），不去碰文件。
- 读取失败时显示读面给的状态与安全错误码。
- 读取在后台线程里做，每次请求带代号；慢读回来时若选中项已变就丢弃。**任何时候都不会
  显示过期的或未校验的图。**
- 读取由一条后台工作线程做，**同时只有一条**。离线的整份读中途停不下来：被顶掉的那次读完为止
  （受 8 MiB 上限与 30 秒期限约束），结果直接丢弃。在线时客户端每读完一段都问一次还要不要，被顶掉
  的那次在下一段就停。还在排队等工作线程时就被顶掉的，根本不开始。
- 解码只用 `image`（只开 `png` feature，版本钉在工作区的 `[workspace.dependencies]`）。程序
  解码的图只有两类：这样读回来的帧，和自带的应用图标。

几何叠加与帧共用同一个坐标系，尺寸按这个顺序取：账本正式给出的画面范围（`task.effect_intent`
的 `frame_extent`、`task.geometry_observed` 的画面范围），其次 payload 里的
`frame_width`/`frame_height`，再次这一帧的识别或操作意图给出的尺寸（见下文），再次校验过的帧
解码出的像素尺寸，最后才是叠加本身的范围。解码尺寸
归属于读出它的那次帧请求：重新选中同一事件或读更早都沿用；换事件、清空或读取失败都把它作废。

### 每一行都落到帧上

一个步骤的事件都在 `links.frame_id` 里写明所在的帧：截图、这一帧的 `artifact.created` /
`artifact.verified`、识别、操作意图。选中其中任何一条，帧区都显示这一帧；步骤里的其他事件经
共享 `action_id` 的操作意图找到它。物理输入（`input.*`）在 `Ui` 投影下不带帧——它的
`before_frame_id` 被裁掉了——所以取同一运行中它之前最后一条操作意图的帧，帧下的说明写明帧取自
哪条事件。先在已载入的行里找；只有它们里没有这一帧的截图时，才按帧编号查一次这一帧自己的事件，
帧区停在这一帧上时一直沿用。读取失败或读取不完整写在帧下的状态行里，下次重读时再试。

帧上，除了事件自己的几何叠加：

- **页标签**，在帧上方标题旁：这一帧上识别匹配到的页面，或写明没有匹配。画在帧上会盖住页面自己的
  页头目标。
- **点击标记**：以操作意图的每个点（`action.x, y`；滑动、拖动每个点各一个）为圆心、带白圈的
  圆点。它取代事件自己的 `action` 几何，免得同一次输入画两遍。
- **一句说明**，在帧下面，每个在这一帧上的输入一句：「步骤 0 notice_close：识别到
  `<game>/news`，点击 (1142, 102)」。
- **识别目标框**：这一帧上最近一次识别评估过的目标（`task.recognition_completed.targets`，取匹配页的；
  没有匹配时取第一个候选页的），各画在自己的 `region` 上：通过的画绿色实线，没通过的画琥珀色虚线，
  标上 `target_id` 和角色。纯关键字目标没有区域，不画；帧下那句说明把它们都算上（「……（目标 3/4
  通过）」）。

## 运行

```
acui [--state-root <state_root>] [--source <auto|offline|online>] [--tab <events|observation|changes|errors|health|lab>] [--lang <zh|en>]
acui --help
```

`--state-root` 只对这一次运行有效，覆盖设置文件里的 `state_root`；两处都没有就打印用法退出，
不猜默认值。经 A/B 安装的固定入口 `<安装根>\ui\acui.exe` 启动时，状态根、配置与 Runtime 程序改取自安装的
选择，`--state-root` 与之不同就拒绝。`--source` 选读面（见上）。`--tab` 指定启动页签（截图与复核用），取值就是视图
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
契约里全部 121 个 `event_type` 与 20 个 `origin.module`；**表里没有的一律照原样显示，不猜**。

### 设置文件

语言与字号存在按用户区分的配置目录下：

```
Windows:  %APPDATA%\ActingCommand\acui.toml
Linux:    $XDG_CONFIG_HOME/ActingCommand/acui.toml（没有就用 $HOME/.config/…）
```

```toml
lang = "zh"          # zh | en
text_size = "standard"   # standard | large | extra-large
state_root = 'C:\AC\state'                                  # 可选，绝对路径
actingd_config = 'C:\AC\actingd.config.json'                # 可选，绝对路径
actingd_exe = 'C:\AC\runtime\actingcommand-actingd.exe'     # 可选，绝对路径
```

开台时读一次，下拉框一改就写一次；写回时三个路径键原样保留。**这是监控台唯一自己读写的
文件**（实例配置窗口保存进 `actingd_config` 的 `instances`，以及启动器创建、早退后读回的 actingd
启动日志除外，见各自那一节）：它不在任何
状态根里，状态根依旧全归读面。文件不存在、读不出来或取值不认识，都按默认值（中文、标准）
来。解析器是手写的：一行一个 `key = value`，去掉一对成对的引号，不处理转义——Windows 路径
写在单引号里（TOML 字面量字符串），不要写 `"D:\\…"`。

## 启动器

顶栏第三行。左边是上一次探测的 Runtime 状态，两个按钮（最后还有「实例配置」按钮，见下一节），
下面一行是上一次「启动」或「请求关闭」的结果，或实例配置窗口没能打开的原因。

- **Runtime 状态**：开台时探测一次，之后每按一次「启动」或「请求关闭」再探测。探测就是一次
  `RuntimeClient::connect`：连上了写「运行中 · PID · owner epoch」（取自它自己的
  `runtime-info.json`，经客户端的 owner epoch 核对）；连不上写「未运行」加客户端的错误码与
  操作名，不猜原因。
- **启动**：先探测，已在运行就什么也不拉，写「已在运行，未拉起」并记下这次按钮（见下）。
  否则按 `actingd_exe` 分离拉起
  `actingcommand-actingd --config <actingd_config>`——命令行就这一条；两个键缺一个或不是
  绝对路径，都写明是哪个键，什么也不拉。stdout / stderr 都进监控台**自己**目录下的日志：
  `%LOCALAPPDATA%\ActingCommand\logs\actingd-<unix_ms>.log`（Linux：`$XDG_STATE_HOME` 或
  `$HOME/.local/state` 下同名路径），目录由监控台建，**永不在状态根里**。Windows 下用
  `DETACHED_PROCESS` 拉起：守护进程不继承监控台的控制台，收不到它的 Ctrl+C。
- **就绪判定**：最多 60 次、每次 500 ms。每次先 `try_wait()`：子进程已退出就停下，写退出码、
  该日志里最后一行以 `FATAL actingd:` 开头的**原文**（日志读不了就写读取错误，没有这样的行就照直
  说没有）与日志路径；没退出就再 `connect` 一次，连上即就绪，写 PID 与 owner epoch，随后记下这次按钮
  （见下）；本台若按离线读（含离线读面未打开），追加一句「要在线读请带 `--source online` 重启」——
  **不会在会话中途悄悄换读面**。60 次都没连上，写「仍未就绪」和最后一次客户端错误码。就绪**从不看
  守护进程的输出**：只在早退之后读回日志、只取那一行，其中只认它是否带 `owner_resource_unconfirmed`（见下）。
- **记下启动按钮**：Runtime 存在之后才记得下，所以顺序是探测 →（需要时）拉起 → 就绪判定 → 记账。
  按按钮是人的动作，所以记账新开一条身份为 actor `user`、source `ui` 的连接（控制台自己发起的读用
  `ui` / `ui`），`begin_interaction()` 开一个交互，用 `record_client_action_receipt` 记一条
  `client_action`（surface `acui.launcher`，类别 `button`、不带值；这次按钮拉起了进程记 control
  `launcher.start`，发现 Runtime 已在运行记 `launcher.start.skipped_running`），回执必须带 terminal；
  在工作线程上做，不占窗口的事件循环。有没有拉起进程是账本自己记下的结果（`runtime.started`），
  不是按钮的值。结果行追加动作落账的
  序号；记账失败就把 Runtime 的拒绝码（若有）**原样**写出，外加客户端错误码与操作名，拒绝带出
  宿主失败时再写宿主码与操作——Runtime
  仍照写已就绪 / 运行中。就绪判定
  失败时没有连接，**什么也不记**，失败只写在结果行上——这是启动器里可能生效却不落账的两种动作之一，
  另一种是在 `ledger` 阶段失败或被终止（超时或读取子进程状态失败）的解锁（见下）。
- **解锁 owner**：只在上一次启动的 FATAL 行带 `owner_resource_unconfirmed` 时出现——actingd 拒绝了
  这个状态根，因为它上一个 Runtime owner 退出时设备资源仍在用或未确认。此时结果行下面多一行「解锁
  owner…」按钮。第一下什么也不运行，只亮出声明「上一个 Runtime 的设备资源已经释放」和「确认并解锁」
  按钮；第二下才按 `actingd_exe`（同启动一样须是绝对路径）运行，命令行就这一条：
  `unlock-owner --config <actingd_config> --actor acui --confirm-resources-released`；在工作线程上跑，
  不开控制台窗口（Windows 用 `CREATE_NO_WINDOW`：输出要截获，所以不分离），截获 stdout 与 stderr，
  最多 180 秒——远高于 Runtime 自己给账本阶段的 120 秒预算，不会截断本来能完成的解锁；
  超时就终止并回收，结果行写结果未知。actor 是 `acui`，指这个监控台而不是某个人：不会把
  操作系统用户名写进账本。stdout 去掉首尾空白后必须整体是一个 JSON 对象，`schema_version` 为
  `actingcommand.actingd.unlock-owner.v1`。`ok` 且退出码 0：写被解锁的 owner epoch、解锁前处置
  （`in_use` / `unconfirmed`）与 `owner.lock` 修订号，收起解锁入口，再按同一条路径自动启动一次。
  `failed`：**原样**写 `error.code`、`error.stage` 与 `journal_appended`（只有阶段 `ledger` 才是
  `true`：解锁已落盘，下次启动会接管那个 epoch，但账本里缺它那条事实）。没有 JSON 时，**原样**写
  stderr 里最后一行 `FATAL actingd:`——参数错误，或装的 actingd 太旧、不认识这条命令。拉起失败、
  超时、输出无法解析或不合约定、`ok` 却退出码非 0，各有各的结果行，都不重试启动。除解锁成功外，
  任何结果之后入口回到第一步；新的一次启动收起它。解锁进行中「启动」被拒；上一次「启动」的就绪轮询还没结束时再按，也直接拒回，写「上一次启动仍在等待就绪」，什么也不拉。监控台不为它记
  `client_action`：没有运行中的 Runtime 可经手，`unlock-owner` 自己追加 `cli.command` 事实（action
  `owner.unlock`）。它从不删除 `owner.lock`（Runtime `contracts/actingd-unlock-owner.md`）。
- **请求关闭**：只走类型化客户端，从不杀进程。新开一条身份为 actor `user`、source `ui` 的连接
  （Runtime 只接受控制台前的人或操作员 CLI 发起关闭请求：`75ed4b3f` 起如此，之前只接受 CLI，这个按钮
  因此从未成功过），`begin_interaction()` 开一个
  交互，先用 `record_client_action_receipt` 把这次按钮记成 `client_action`（surface
  `acui.launcher`、control `request_shutdown`），拿到带 terminal 的回执后再发
  `request_shutdown()`——动作先落账，再请求。被拒为 `runtime_busy`（别的请求——比如一次状态
  查询——之后 Runtime 会短暂占着生命周期准入；有租约在用或有排队请求时也会被拒为忙碌，这几次重试等不过去）就隔一秒在同一个交互上再发，总共最多 5 次，其间
  结果行写「忙碌重试 n/5」；按钮仍只记一次。受理了写回执状态、请求编号、动作落账的序号；以别的
  理由被拒（owner / governance 等）或第 5 次仍忙，就把 Runtime 的拒绝码**原样**写出，外加客户端
  错误码与操作名（拒绝带出宿主失败时再写宿主码与操作）；其他拒绝或错误立即停下。最终结果行也写发了几次。之后再探测一次状态——Runtime
  按自己的节奏停，这一眼可能还写着运行中。
- **永不杀**：`Child` 句柄只用来 `try_wait()` 看有没有早退，不 `kill`、不阻塞 `wait`、不挂
  job object；就绪判定结束就丢掉句柄，守护进程活得比监控台久。

监控台没有暂停与恢复的控件：用 `actingctl pause --state-root <状态根>` 暂停调度（只停一个实例时加
`--instance <别名>`），用带同样参数的 `actingctl resume` 恢复，或用 MCP 工具 `ac_pause` / `ac_resume`（见「智能体用的 MCP」
一节；Runtime v0.11.6 里别名含大写字母时这两个工具会以 `client_action_invalid` 失败，是已知问题，
这时改用 `actingctl pause` / `resume`）。开机自启是安装向导的一个选项，见「安装向导 acsetup」一节。

## 实例配置

「实例配置」按钮打开第二个窗口，对象是 `actingd_config`——「启动」交给 actingd 的那份文件——
里的 `instances`。每一项列出别名、`instance_id`、文件里写的绑定（`fixture_backend`，Runtime
先于任何绑定键采用它 / MuMu 序号 / MuMu 名称 / ADB 序列号，与 `host` + `port` 同在时只显示
序列号 / ADB host:port / 文件未写绑定键——不替 Runtime 复述默认地址）、`application_id`、截
图与触控后端，以及本会话的端口映射对这个编号怎么说——绑在某个端口、绑了但最新一次不在端口映
射里（序列号配置或无端口，映射分不出是哪种）、没有绑定；在线读面、账本未打开或读绑定失败时，行里直说；
没有字符串 `instance_id` 的项在这个位置写明不可编辑。值为 JSON `null` 的键，列表和表单里都
当作没写。每次打开窗口、每次保存之后都重新读文件；列不出来的原因写在条数的位置上，绝不显示
成一个空列表。

- **发现**：「发现实例」在工作线程上经类型化客户端（`discover_instances()`）请正在运行的 Runtime 重跑
  一次其提供者的 MuMu 实例发现；Runtime 会在宿主上运行 `MuMuManager`，客户端最多等 25 秒。契约只接受
  控制台前的人或操作员 CLI 发起这个查询，所以它单独开一条连接，身份是 actor `user`、source `ui`（枚举
  值，不是系统用户名），与启动器的按钮相同。它不绑定任何东西，也不碰设备；按契约，Runtime 把答复了
  的查询记成一条观察事件（`command.validated`），把拒绝记成 `command.rejected` 加 `runtime.failed`。旁边的框随后列出报告的
  每个实例：MuMu 序号、是否在运行、运行中的 Runtime 给它绑定的别名（若有）、ADB 地址、Android 版本，
  名称放最后；那一行写出个数、提供者版本和这次查询的序号。在框里选只是选中，点「采用所选」才生效：
  配置文件里没有指向它的项（`instance_index` 等于其序号、`instance_name` 等于其名称，或 `port` 等于其
  ADB 端口）、运行中的 Runtime 也没绑定它时，就以这个序号为绑定起一个新项（别名等照填；地址留给启动
  时的发现）；已被绑定的只指明是哪一项，什么也不改。没有 Runtime 在跑，或
  被拒（`instance_discovery_unavailable`、`mumu_manager_version_unsupported` 等）时，那一行写 Runtime
  的码、客户端错误与宿主失败，之前的结果也不再可选。
- **表单**：「新增实例」另起一项，点一行把那一项载入表单。没有字符串 `instance_id` 的项，以及表单
  的哪种绑定都表示不了的项——配置了 `fixture_backend`、设了 `serial`、一个绑定键都没写——照样列
  出，但点它会写明原因、不能保存。没有删除。`alias` 必填；新实例的 `instance_id` 是 `instance_`
  加系统随机源的 32 位小写十六进制，所有 `instance_id` 都只读显示；绑定恰好一种——`instance_index`
  （MuMu 序号）、`instance_name`（MuMu 名称），或 `host` + `port`（显式 ADB 地址）。`adb_path` 三种
  绑定下都选填：留空则 Runtime 用 AC 自带的 adb（`<安装根>/tools/platform-tools/adb.exe`，Runtime 核它的
  sha256，缺失或不符就拒绝启动）；MuMu 绑定下若填写，只能是 MuMu 自带的 adb 或 AC 自带的 adb。留空的
  框不写这个键，编辑已有项时清空它会删掉这个键。这需要从 A/B 安装根取工具的 Runtime；AC 自带 adb 属于安装根、两个槽共用，其它工具使用各自配置的 adb。`nemu_app_index` 是选填的整数。`application_id`、`capture_backend`、`touch_backend` 表单不检查，要不要填、取值是否有效都由
  check-config 判定，`nemu_app_index` 的配对也由它查。只有要靠 MuMu 发现结果的几项到 Runtime 启动
  时才查：`MuMuManager` 版本与能力、发现结果恰好匹配一个、声明的 `adb_path`、`host`、`port` 与发现值
  不冲突、ADB 端点（Runtime `contracts/actingd-check-config.md`，`3d5398d6` 起）。文本去掉首尾空白，
  留空的框不写这个键；必填项为空、数字解析不了，都在写任何东西之前直说。
- **保存**：已安装监控台读取本进程固定的私有配置代际，只修改表单管理的键，
  经 stdin 把 JSON 提案与原完整选择交给已核的固定 acsetup。只有 acsetup 做资源资格、私有后继代际准备、
  配置检查及准确 generation/字节基线比较后的提交。冲突和材料不足明确失败，原代际保留。
  客户端最多等 240 秒；答复丢失或超时表示结果未知，监控台不结束管理进程、不重复提交。
- **生效**：返回选择核验通过才报告保存。重新打开监控台以取得后继代际，再重启 Runtime；
  原监控台及其子调用保持原代际，编辑器本身不重启 Runtime。

## 安装向导 acsetup

`crates/acui-setup` 是一个独立的二进制 `acsetup.exe`（Slint 窗口，与监控台同一套样式与图标），把
伞仓 [Releases](https://github.com/HS7097/ActingCommand/releases) 里的发布件装成一份**按用户**的安装。
它随 UI 仓的 Windows 构建一起发布（`acui-windows-<sha>.zip`——分支或 PR 构建的 Actions 产物，或 UI Release `vX.Y.Z` 的资产——包含 `acui.exe`、`acsetup.exe` 及固定入口 `acforward.exe`）。
同一个程序有两种形态，人只下载其中一个：在线版 `acsetup.exe` 自己去取发布件，也可以用人手工下好的文件夹；
离线版 `acsetup-full-<tag>.exe` 自带一整个发布件（见下文「离线版」）。一个窗口，只有下一步（实例步可跳过），五步；每页像安装器那样只显示
进行到哪了，详情全写进安装日志（见下文「进度与安装日志」）：

0. **位置**：只有安装根（可改，默认 `%LOCALAPPDATA%\Programs\ActingCommand`，不需要管理员）、该卷的
   可用空间、此处是否已有安装（读取选中 MEMBERS，首次迁移前读取原根清单；已有就是
   **升级**，见下文）。下一步时建好安装根与安装日志。新装时这一步还要求状态根可用、安装位置不含单引号（监控台设置把路径写成 TOML 字面量字符串，单引号写不进去），否则停在这一步。引导从业务槽或原程序目录运行时停止；已核的固定管理入口
   `<安装根>/ui/acsetup.exe` 可直接升级业务槽。离线版在可用空间一行后面加上取出自带发布件所需的量。
1. **安装**（已有安装时为**升级**，见下文），一页从下载做到铺开，成功后自动进入下一页。默认联网。一进这一步就经 HTTPS 向伞仓 [Releases](https://github.com/HS7097/ActingCommand/releases)
   要一个发布件：有正式版取最新正式版（GitHub 的 `releases/latest`），否则按发布时间取最新的预发布（现在伞仓发的是
   `vX.Y.Z` 预发布；旧的每日 `build-*` 预发布已经停用，留在页面上仅作参考），从不取草稿；页面与日志写明它的标签、
   名称、日期、种类与大小。点「安装」依次下载 `SHA256SUMS`、`MEMBERS.json`，再下载 `SHA256SUMS` 列出的
   其余文件——别的一个不下——存到 `<安装根>\downloads\<标签>\`；每个文件先写 `.part`，长度等于发布件
   声明的长度才改名，1 MiB 以上的文件每满十分之一写一行进度；目录里已有的同名文件重新下载，从不直接采信，
   下载失败的 `.part` 文件会删掉。标签与每个文件名都只在只含字母、数字、`.`、`-`、`_`，不以点开头，且不是
   Windows 设备名时才使用。只走 HTTPS（重定向也一样）；连接一分钟没有数据即失败。改勾**离线**则用一个已放好同一发布件全部文件的文件夹（默认
   `%USERPROFILE%\Downloads`，路径直接填）。查询失败写在页面与日志里，可点「重新查询」，离线仍可选；下载失败则停下。这是
   程序仅有的联网代码（`ureq`，阻塞式，rustls 加编译进去的 Mozilla 根证书）；Runtime 没有任何联网代码。
   离线版没有查询，也没有「离线」勾选：页面写出它自带的发布件——标签与启动时已核对的 `MEMBERS.json` 里的
   两个提交号——点「安装」把它取出到 `<安装根>\downloads\<标签>\`，每个文件先写 `.part`，长度与 sha256
   都对上才改名（杀毒软件短暂占用导致的改名失败有限次重试）。
   下好的、离线的或取出的文件夹随后在同一页校验并铺开：要求文件夹里有 `SHA256SUMS`、`MEMBERS.json`、`actingcommand-runtime-<sha>.zip`、
   `actingcommand-tools-<sha>.zip`、`acui-windows-<sha>.zip`（`<sha>` 取 `MEMBERS.json` 的
   `runtime_sha` / `ui_sha`，三个 zip 必须在 `SHA256SUMS` 里）。逐条核对 `SHA256SUMS`；解压到安装根
   下的临时目录 `.staging-<unix_ms>`，其下分 `runtime\`、`ui\`、`tools\`，不从中运行任何程序；再按每个 zip
   自带的 `BUILD-MANIFEST.json` 核对来源仓、提交号
   （等于 MEMBERS 的 sha）、Runtime 的 `runtime_payload_layout`（`distribution-v1`），以及 `files[]`
   每一项的大小与 sha256；zip 里多出清单没列的文件也算不一致。任何不一致都停下，措辞是
   「内容与创建时不一致」——这是完整性陈述，不是授权口吻。校验期间不运行 zip 里的任何东西。随后将程序核心（Runtime 与 UI 载荷）准备到新安装 A 槽或备用槽；
   Tools 文件放进安装根的 `tools\`，只替换内容变了的文件。被替换的文件保留，那里什么也不删；发布件不认识的文件
   原样留着——例如 Runtime v0.11.6 起不再带的 `actingcommand-device-test.exe`，请手动移走。各槽原始清单及 MEMBERS 保留；候选及下载材料保留。
   完成页从安装根的 `tools\platform-tools\source.properties` 读取工具版本。
2. **选项**，配置已自动写好。铺开之后，在安装页、同一个不许关窗的区段里，新装不问任何问题就配置好：状态根为
   `<安装根>\state`（必须不存在或为空目录——在第 0 步、下载之前就检查；**已有内容的状态根一律不接管**）；
   生成 `secret_fingerprint_salt` = 系统随机源 32 字节的十六进制（`getrandom`；**不显示、不写日志**）；
   私有 `install/generations/<generation>/actingd.config.json` 通过检查后，原子提交 `install/active.json`。
   私有配置的字段只有 `schema_version`、`state_root`、`bind_host`（127.0.0.1）、`bind_port`（0）、
   `secret_fingerprint_salt`、`instances`（空）——Runtime 的解析器 `deny_unknown_fields`，多一个字段都不写。
   监控台设置 `%APPDATA%\ActingCommand\acui.toml` 写 `state_root`、`actingd_config`、`actingd_exe`（同
   「设置文件」一节的格式，单引号字面量；已有的 `lang` / `text_size` 原样保留）。配置和 Runtime 路径指向固定根参数别名与程序入口。写法与
   `crates/acui-app/src/settings.rs` 一致，但 `acui-setup` 不依赖 `acui-app`，是一份小的重复写入器。这里
   `instances` 留空，由实例步填。随后的选项页有四个勾选，在工作线程里一起写（失败写在页面上，页面可继续用）：
   - **开机自启**（默认不勾）：勾了才在按用户的启动文件夹（系统的 `FOLDERID_Startup`）写 `ActingCommand.cmd`，
     内容是 `@echo off` 加一行 `start "" "<安装根>\runtime\actingcommand-actingd.exe" --config "<安装根>\actingd.config.json"`；
     再勾「同时拉起监控台」才多一行 `start "" "<安装根>\ui\acui.exe"`。批处理里不出现 acsetup。不勾就什么也不写；
     已有的同名文件不动，写进日志。安装位置含 `%` 时批处理无法原样引用，勾了开机自启就在这一步报错，页面可继续用。
   - **开始菜单快捷方式**（默认勾）与**桌面快捷方式**（默认不勾）：指向 `<安装根>\ui\acui.exe`、工作目录 `ui\`
     的 `ActingCommand.lnk`，放在按用户的开始菜单「程序」（`FOLDERID_Programs`）或桌面（`FOLDERID_Desktop`，
     桌面被重定向或在 OneDrive 里也照资源管理器的位置找），经系统自己的 `IShellLink` 写出。不勾就什么也不写；
     完成页列出写出的快捷方式。
3. **实例**，可选，一进页面就开始查找：「跳过」让 `instances` 留空，之后点监控台顶栏的「实例配置」按钮添加；
   完成页也这样写。MuMu 在哪由 Runtime 自己的 `check-config` 给出（`mumu_root`：路径与来源——
   `ACTINGCOMMAND_NEMU_FOLDER`、运行中的 MuMu、
   按用户或按机器的卸载注册表、厂商目录；见 Runtime 的 `contracts/actingd-check-config.md`），找不到时才由人填
   目录；两者都经与下文同样的候选文件与 `check-config` 钉进配置的 `mumu_root`，免得以后另一套 MuMu 把实例接走。
   Runtime 版本太旧、不回报 MuMu 位置时不钉，写成注意事项。随后按下文升级的方式从安装根拉起 Runtime（已应答的先请
   它关闭再重新拉起：它还没有实例），用 `actingctl emulator discover` 从 MuMu 自己的实例清单列出实例——不启动、
   不关闭任何模拟器；只有一个实例时替人勾上。「重新查找」再来一遍；MuMu 栏清空后「重新查找」会重新探测。
   资源是发布件自带的资源仓**标准包**：`MEMBERS.json` 的 `bundles[]` 写明每个标准包与它的 sha256，文件就在下载文件夹里，
   安装时已按 `SHA256SUMS` 核过；第一次查找时，向导要求每个标准包以同一个 sha256 列在 `SHA256SUMS` 里，再核一次文件的
   sha256 后读取。哪一项不过，就在注意事项里写明；发布件必需标准包读不出时不能提交配置计划。**不问网址，也不要人填哈希。**标准包里有
   `applications.json`（游戏、给了就有的显示名 `label`、各服务器的标签与安卓包名）和 `bundle.json`（每个包的路径、
   包 id、服务器、sha256 与字节数，以及 `default_packs` 里各服务器的默认任务包）。`bundle.json` 为
   `actingcommand.bundle.v2`（契约的 `BundleIndexV2`）的标准包，把每个包 id 映射到以
   `content-directory.v1` 摘要命名的内容目录 `packs/<digest>/`，各服务器的默认任务包取 `applications.json` 的
   `servers.<server>.default_package_id`；v1 照旧读取。`actingcommand.bundle.v3` 增加必需的 `maintenance` 数组，
   每项为 `package_id`、`server`、`uses`（`startup`、`prerequisite`、`return_home`）。共享契约严格解码及校验 v2/v3；
   v3 索引全体实际包经 hash/Containment、`PreparedContainedTask::describe_path` 后，再交共享
   `validate_bundle_maintenance` 核身份、用途资格及包内完整链。离线准入有 120 秒期限，不执行任务或 provider。
   未知版本、字段、用途、坏引用及不合格材料明确失败。页面写出它们支持的程序与包名——形如
   「`<label>`：`<服务器标签>` `<安卓包名>`」，取自标准包的 `applications.json`（标准包没给 `label` 时写 game id）——写明是否发布件自带，
   两类服务器写明不可选：声明了默认任务包却没有包名的，以及有包名却没有默认任务包的。本机标准包文件可以随时加入，主要用在发布件没带标准包时：填绝对路径，点「加入」，
   不要哈希；已有同一游戏（不分大小写）的标准包时拒收。每个勾选的实例
   填别名（默认 `mumu-<序号>`），并**各自选**「程序 · 服务器 · 包名」，每个同时有包名与默认任务包的服务器一项；只有一项时替每个
   实例选上。向导不懂游戏，也不猜。「写入实例」先暂存及验证标准包，形成配置计划并解决冲突，确认 Runtime 关闭后才铺到
   `<安装根>\packages\<game>\`。ZIP 标准包放在其下的 `bundles\<标准包-sha256>\`，同名旧包保留。
   v2/v3 任务包逐个文件流式解到 `<digest>.part\`，边写边算每个文件的哈希，文件数、字节数与
   `content-directory.v1` 摘要都与索引一致才改名为 `<digest>\`；已有的 `<digest>\` 整个读一遍，摘要一致就复用，不一致就改名
   `<digest>.broken-<unix>` 保留（写进注意事项）后重新解包；解包失败的也按这个名字保留。旧目录与旧 zip 原样留着，两份声明不落到
   安装根。日志按实例写明程序、任务包的包 id、路径与 sha256；再为每个勾选的实例写一项——别名、新的
   `instance_id`（`instance_` + 系统随机源 32 位十六进制）、`instance_index`、所选服务器的包名、
   `touch_backend` 为 `adb_shell_input`、`capture_backend` 为 `adb`（实机确认 `nemu_ipc` 首帧前）、该服务器默认任务包的
   绝对路径（v2/v3 为它的 `<digest>\` 目录，支持内容目录的 Runtime 会完整准入）作 `resource_package`。
   所选游戏/服务器的维护用途生成现有 `startup_package`、`prerequisite_packages`、`return_home_packages`；回主页用途同时注册前置引用。
   不替业务 control 增加字段或触发器。完整键值相同就复用；冲突显示旧值、新值及来源，由人明确保留或采用，取消即停止本次计划，
   选择页面等待期限 30 分钟。清单缺项不会删除旧绑定。实际合并后保留的旧引用与新引用全部经实物准入、descriptor 资格及
   `PrerequisiteChain` 核完整链；材料无法核定则拒绝。acsetup 准备私有后继代际并保持相对路径语义，目标 `check-config` 检查后，
   当前 generation 与精确字节基线仍一致才原子提交安装选择，原代际保留。启动前失败恢复原配置并保持 Runtime 停止；恢复不完整则停止引导。提交后重启 Runtime，
   `actingctl status` 必须应答。替换配置之前的失败写在页面与日志里，页面可继续用；之后的失败停下，任何一次日志
   写入失败也停下。离开这一步时再问一次 `mumu_root` 的现值与 Runtime 是否应答，写进摘要。
4. **完成**：摘要写出装上的是什么（runtime 与 ui 的提交号，以及发布件标签、离线文件夹或自带发布件）、各路径，以及各步
   留下的注意事项。「启动监控台 / Open console」分离拉起 `<安装根>\ui\acui.exe` 并关闭引导；「完成」只关闭。
   实例步拉起的 Runtime 继续运行；没有在运行的，由监控台的启动器拉起。

**A/B 安装与升级**：`<安装根>/A`、`B` 各保留程序核心：Runtime 与 UI 载荷、原始构建清单及准确 MEMBERS
（保留下来的 v0.11.1 槽另有自己的 Tools）。Tools、状态根、内容哈希资源、模型、下载、日志和私有配置代际位于槽外。acsetup 唯一写 `install/active.json`，
一次原子提交槽、generation、MEMBERS 身份及带哈希的配置/provider 输入。配置编辑也经其单写者锁及准确代际/字节基线比较。

下载、完整核验、备用槽物化、资源准入和配置规划均在关闭前完成。原实例身份及业务设置保留；
关联和维护冲突沿实例页的明确选择处理。私有代际保持相对路径语义，模型留在共享位置；只有 v0.11.1 槽的代际保留绑定该槽 tools 的 provider 清单；
目标程序的 `check-config` 检查准确未选中候选。

永久空文件 `install/slot-A.lock`、`slot-B.lock` 仅定位原生占用锁。消费者持共享锁，物化须取得排他锁，
并排除旧 MCP/Tools/ADB 等进程的原生占用。备用槽被占用时阻断物化，下载和当前选择保留；
不为清槽结束共享 ADB 服务。被替换的备用材料保存在 `install/retained-<槽>-<时间>`。

Host 在同一生命周期准入下自然排空并提交正式原子关闭。acsetup 必须确认准确接受 owner、已关闭的 owner journal
及进程退出，才能切选择。status 失败本身不证明停机；已停机安装仍须通过正式冷态 owner/writer 门。
每次切换也对当前静止数据执行目标程序的 `ledger-maintenance verify`；它会更新 owner.lock epoch/revision，
材料或预算不足时兼容门未证明。

原 Runtime 在运行时，acsetup 启动新槽至 Provider 前的 held，核准确票据及进程后经同一 Host 放行。
排空、held、放行各用 60 秒 Host 期限；控制客户端最多等 75 秒，冷态验证客户端最多等 150 秒且 Runtime 自身限额仍有效。
提交结果未知时查询原 transition，不重新提交；关闭不明确就停止切换，不结束结果未知的控制/维护进程。
Host 的 released 结果才表示准备及本次原用户暂停恢复完成。原先停机的安装保持停机。
acsetup 拉起 Runtime 一律用同一种方式：由 WMI `Win32_Process.Create`（`Win32_ProcessStartup.ShowWindow = 0`）启动 `cmd.exe`，
再由它运行 actingd，并把输出追加到 `<安装根>\actingd-<时间>.log`。因此 Runtime 窗口隐藏，不属于任何应用作业，也不属于运行 acsetup 者的作业，
也没有能被关掉的窗口。拉起的 actingd 读取刚提交的选择；它一启动就退出时，acsetup 的报错带上日志里的 FATAL 行。

**组件接口**：Runtime、UI、acsetup 自己和本次用到的标准包能否搭配，按各自的声明判断，不再按写死的版本对。
构建清单的 `interfaces` 对象（标准包则是 zip 根目录下可选的 `interfaces.json`）按接口写明该组件能读的修订区间 `[min, max]`，写方写 `max`。
词表、升版规则和两种检查见 Runtime 的 `contracts/component-interfaces.md`：一方写、他方读的数据（`ledger`、`install-selection`、`package`）
要求写方的 `max` 落在每个读方的区间里；活的交互（`install-control`、`runtime-client`）要求两个区间相交，取最高公共修订。
Runtime 的配置（`actingd-config`）涉及三方：acsetup 写、控制台改、Runtime 读，所以三方须有一个都能说的共同修订，两两核对不够。
acsetup 自己的声明是 `crates/acui-setup/component-interfaces.json`，编进 acsetup，也由构建写进 UI 清单；它随本 UI 所钉的 Runtime crate 一起变。
声明出现之前的发布件（Runtime 与 UI 的 v0.11.0 至 v0.11.2）按内置表识别，其它未声明的程序一律拒绝；没有 `interfaces.json` 的标准包按 `package` [1, 1]。
全新安装、首次迁移和 A/B 升级在提任何问题、改任何东西之前，核对发布件的 Runtime 与 UI 彼此之间、与要接手的 Runtime 之间、与 acsetup 之间是否相容；
放进新槽的 Runtime 还须 Tools 布局为 2 或 3（槽只含程序核心）。发布件自带的标准包、向导加入的本机标准包和实例步的标准包，
都与将运行它们的 Runtime 及 acsetup 核对。回退也按同样方式核对保留槽。每一条不满足的边都列出（接口、双方组件及其区间），
安装不改动即停止（命令行退出码 6，`--rollback` 为 1）。协商出的 `install-control` 修订决定 acsetup 怎样关闭与拉起 Runtime：
0 是冷态协议，1 是 Host 安装过渡。`<安装根>\runtime\` 与 `<安装根>\ui\` 下的固定入口也读 `install/active.json`，
所以 A/B 升级会把字节与发布件 `acforward.exe` 不同的固定入口换掉；原入口保留在 `install\entries-<代际>\`，正在运行的入口从那里继续运行。

首次迁移接受接口能被驱动的旧布局程序（有声明的，或内置表里的 v0.11.0、v0.11.1），先备好 A 槽，
再按双方都支持的 `install-control` 修订关闭旧 Runtime（v0.11.0 用冷态协议的原子空闲关闭）；Busy 则不提交切换。
原根程序、配置和监控台设置完整保留在 `install/initial-backup-<generation>`。
固定根入口由 `acforward.exe` 固定一份完整选择、继承 stdio 并返回实际退出码。
根级 `actingd.config.json` 是选中私有输入的参数别名，此处没有可写配置文件。开机项、快捷方式及 MCP 保留固定根路径。
MCP 获得明确 root/state-root 参数，位置冲突即拒绝。

运行中的 UI/MCP/Tools 固定自己的代际及材料，重新打开进程才取得后继代际。
v0.11.0 UI 是明确绑定共享状态根的观察入口；设置只保存根级 Runtime 入口与配置别名，
其旧编辑器读取不存在的别名时明确失败。有效编辑交给固定 acsetup；只支持 `install-control` 0 的 Runtime（v0.11.0）由 acsetup 按真实冷态启动能力控制。

**显式回退**：运行 `<安装根>/ui/acsetup.exe --rollback`，重新核备用槽全部清单，
从当前业务设置重规划候选。原 owner 关闭后，目标程序的 `check-config` 及对最新数据的
`ledger-maintenance verify` 均须通过。这些门不证明 Provider/设备就绪。
正常安装页也可提供更早发布件并确认提示；发布时间只用于顺序提示，冷态门仍必需。只支持 `install-control` 0 的 Runtime（v0.11.0）按冷态路线启动。

回退有限制，因为较新的 Runtime 可能单向改动状态根。Runtime v0.11.3 或更新的版本把账本写成修订 2 之后，
带 Runtime v0.11.2 或更早版本的槽被拒绝（退出码 1、不做改动）。Runtime v0.11.6 的截图清理器跑过一次之后，
Runtime v0.11.5 及更早版本拒绝打开该状态根。状态根很大时，从 v0.11.1–v0.11.3 升到 v0.11.4 实际上是单向的。

acsetup 是窗口程序：在控制台直接输入 `--rollback` 或 `--replace-manager` 时提示符会立即返回。
工作不挂在该控制台上（关闭窗口不会中断它），结束时结果行（或 `失败 / FAILED: …`）和日志路径会显示在那个控制台里；
每次运行都把结果写成 `<安装根>/acsetup-<时间>.log` 的最后一行。在交互式 PowerShell 窗口里等待结果并查看退出码（0 成功、1 失败）：

```powershell
$p = Start-Process -FilePath '<安装根>\ui\acsetup.exe' -ArgumentList '--rollback' -PassThru -NoNewWindow
$null = $p.Handle; $p.WaitForExit(); "exit code: $($p.ExitCode)"
```

脚本里则把退出码传下去（不要贴进交互式窗口：`exit` 会关掉窗口，结果行也随之消失）：

```powershell
$p = Start-Process -FilePath '<安装根>\ui\acsetup.exe' -ArgumentList '--rollback' -PassThru -NoNewWindow
$null = $p.Handle; $p.WaitForExit(); exit $p.ExitCode
```

首次新 Runtime 启动尝试前，只有完整原事务可恢复已提交选择；首次迁移还须恢复原根程序、配置和设置。
恢复不完整则保留材料并保持 Runtime 停止。首次启动尝试之后保留当前选择及完整账本，
后续显式回退须重新验证最新数据。关闭未确认时不启动第二个 Runtime。

**命令行**：除 `--commit-config`、`--rollback`、`--replace-manager` 外，带任何参数都不开窗口、走命令行。
`acsetup --root <安装根> (--plan | --yes) [--conflicts new|old] [--associate <别名>=<标准包>/<服务器>]… [--allow-downgrade] [--online | --from <文件夹>]`
安装或升级；向导要问的每个问题都由参数回答，缺了参数的问题在安装改动之前停下。`--plan` 列出全部改动与差异，安装根下不写任何东西：
日志和临时副本放在 `%TEMP%`。退出码：0 完成，1 失败，2 用法，3 有维护绑定差异而未给 `--conflicts`，4 降级而未给 `--allow-downgrade`，
5 资源关联需要选择而未给 `--associate`，6 接口不兼容（见"组件接口"）；2 至 6 都停在安装改动之前。`acsetup --help` 列出全部。

**只更新资源**：`acsetup --root <安装根> --resources <标准包.zip> [--sums <SHA256SUMS>] (--plan | --yes) [--conflicts new|old] [--associate …]`
把一个资源仓标准包（v2 或 v3；v1 标准包须走完整升级）放进已有的 A/B 安装，不换程序、不切槽。它不与 `--online`、`--from`、`--allow-downgrade` 同用；
两种版本都接受它，离线版自带的发布件此时不用。它在写者锁下：按 zip 旁边（或 `--sums` 指定）的 `SHA256SUMS` 里那一行核对 zip，读标准包，
核对选中槽的程序、acsetup 与标准包的接口，暂存并准入每个包（v3 还校验维护声明），再把每个包与 `packages\<游戏>\<摘要>\` 比较：
已有且相同的复用，没有的是新包，已有但不同的让本次运行停下、安装不改动——这样的目录一律不挪不删；请手动改名移开，或做一次完整升级（升级会把它改名保留）。
随后按标准包声明的维护绑定（启动包、前置包、回主页包）做计划，关联与冲突规则同升级（缺参数时退出码 5、3），并列出每一项要改的绑定。
既无新包又无绑定变化就停下：无需改动。否则逐个经 `<摘要>.part` 放入新包；绑定没有变化时到此为止——不生成新代际、不重启。
绑定有变化时，准备新的配置代际、校验整条链、跑选中槽的 `check-config`，关闭 Runtime（在运行就排空并原子关闭，不在运行就过冷态门），
提交，原先在运行的再拉起。只是路径写法不同不算变化。摘要写出 zip、新放入与复用的包数、每一项改变的绑定、选中的代际（或"配置不变"）、
Runtime 的情况以及未变的程序与槽。`--plan` 做同样的检查，安装根下不写任何东西。典型用法是
`acsetup.exe --root <安装根> --resources <目录>\<标准包>.zip --plan`，再 `--yes --conflicts new`。
在向导里，A/B 安装的第 0 步多一个勾选「只更新资源（程序不变）/ Update resources only (programs unchanged)」（旧布局则说明须先完整升级一次）。
勾选后点下一步，做与升级相同的检查（安装根、不得从这份安装的程序目录运行、日志），进入第 7 步「只更新资源 / Update resources」：
填标准包 zip 的绝对路径，可另填一个 `SHA256SUMS`，点「更新资源 / Update resources」。运行与命令行相同：先取得写者锁，再清上次留下的临时目录；关联页（第 5 步）与冲突页（第 6 步）回答它的问题；
在这两页取消则什么都不改。运行结束前窗口不能关闭。成功后进入完成页，显示上面的摘要，「启动监控台 / Open console」经安装的固定入口打开监控台；失败页写明此时的状态。

**临时目录**：每次运行在结束时（无论成败）删除自己的 `.staging-<时间>`（实例步的 `.staging-resources-<时间>`、`--replace-manager` 的
`install\manager-source-<时间>`）并记入日志；删不掉的写进摘要的注意事项，与被中断的运行留下的一样，在下次运行开始时删除。

**固定管理入口**：`<安装根>/ui/acsetup.exe` 的独立原始构建清单及 MEMBERS 在 `install/manager`；
业务槽回退保留该管理程序。替换时先关闭固定管理程序（所有 `<安装根>/ui/acsetup.exe` 窗口），
再运行该发布件任一份已核、未改动的 acsetup（固定管理程序本身除外），例如新槽的 `<安装根>/<槽>/ui/acsetup.exe` 或发布件自带的 acsetup：
`& "<安装根>\<槽>\ui\acsetup.exe" --replace-manager "<安装根绝对路径>" "<发布件目录绝对路径>"`
（PowerShell 里带引号的程序路径前须加 `&`）。A/B 升级会自行替换较旧的固定管理程序；从固定管理程序本身启动的升级会在摘要里给出这条确切命令。
目录须有正常发布文件及 SHA256SUMS；原生占用须证明旧管理进程退出，运行的程序须与已核 UI 载荷一致。
旧管理材料及身份保留，各槽的 acsetup 原件完整保留。

**进度与安装日志**：有限安装操作记录于 `<安装根>/acsetup-<时间>.log`；Runtime 结果来自正式 Host/CLI 与
GlobalLedger，拉起进程的日志保留启动及致命错误末言。日志失败即停止。页面展示阶段、进度和明确错误；
失败材料、历史代际、程序备份及哈希资源保留。安装/配置提交期间不可关闭向导。
预准备用于减少停机，冷态验证和启动实际耗时需实测。

**离线版**：`acsetup-full-<tag>.exe` = `acsetup.exe` 原样字节，其后是 `SHA256SUMS` 与它按行序列出的每个文件
（含 `MEMBERS.json` 与资源仓的标准包），再是索引，最后是恰好 125 字节的尾部：
`ACSETUP-PAYLOAD-1 <载荷起点，%020d> <索引长度，%020d> <索引的 sha256>\n`。索引第一行是
`acsetup-payload v1 <tag>`；其后每个文件一行 `<sha256> <长度> <文件名>`，名单、顺序、sha256 与 `SHA256SUMS`
逐项相同，只在最前面多一行 `SHA256SUMS`。运行哪种形态，在开窗之前从自身 exe 读出：PE 映像末端——各节原始数据的
最远处，由手写的最小解析读出（DOS 头、PE 签名、COFF 头、可选头、数据目录、节表，全用 checked 算术）——对比文件的
有效末尾：文件长度；签名以后则是证书表偏移再去掉至多 7 个 NUL 填充——证书表没有恰好止于文件末尾的，按损坏处理。相等是在线版；更长就必须是完整的载荷：尾部格式、
载荷起点等于映像末端、索引不超过 1 MiB 且 sha256 相符、各长度之和恰好到有效末尾、文件名合乎在线取件的规则且另外不以
`-` 开头、不以 `.` 或 `.part` 结尾、不区分大小写不重复、从载荷里读出的 `SHA256SUMS` 与 `MEMBERS.json` 与索引相符、
标签若是旧的每日形状 `build-r<7>-u<7>`，须与 `MEMBERS.json` 的提交号相符（`vX.Y.Z` 标签不含提交号，没有可核对的）。其他任何情况——文件被截断、尾部、索引或头部损坏——都在
写任何东西之前进失败页：写明应为多少、实为多少，写明「尚未写任何文件，也没有日志」，并提示重新下载（可用旁边的
`.sha256` 核对）或改用在线版 `acsetup.exe`；绝不改走联网。从这次检查到取出，自身文件一直开着同一个句柄。索引与
`SHA256SUMS` 只防损坏，不防篡改：可信度来自从 Releases 页经 HTTPS 下载，以及每个安装器旁边的 `.sha256`，与在线版
一样。Windows SmartScreen 与部分杀毒软件可能对新出现、未签名、映像后带数据的安装器报警，这是预期，不是向导缺陷。
每次伞仓发布都拼装这两个安装器，离线版在发布前另行独立读回核对。

**永远不做的事**：不装服务、不建计划任务、不改 PATH、不写注册表；不改配置模板；不碰已有内容的状态根；
联网只为列出与下载伞仓发布件——离线版完全不联网；只在升级时、实例步和绑定有变化的只更新资源中按上文关闭与拉起 Runtime；
不解开密封任务包——由 Runtime 加载（v1 标准包里的任务包是整个取出；v2 标准包里的任务包本就是内容目录，按上文逐个文件放置）。Linux 上 crate
照常编译（CI 两条腿都跑 `--workspace`），运行即以 `acsetup v1 is Windows-only` 退出。

Runtime 的看门狗是一个计划任务，所以 acsetup 不替人设置。要让没经正式关闭就不在了的 Runtime（崩溃、关窗、重启）
被重新拉起，请以运行 Runtime 的那个用户、在普通（非提权）PowerShell 里运行一次
`<安装根>\runtime\actingctl.exe watchdog install --root <安装根>`（Runtime v0.11.3 起；`watchdog status` 查看状态）。
删除安装之前先运行 `watchdog uninstall`。它的规则见 Runtime 的 `distribution/windows/INSTALL.md`。

向导多出七个依赖（离线版读取自身 exe 是手写的，不加依赖），都在 `[workspace.dependencies]` 或该 crate 的
`Cargo.toml` 里注明用途：`actingcommand-contract`（标准包索引 v2/v3 与 `content-directory.v1` 摘要，只此一份实现；
与监控台同一 rev，只带来 `serde`、`serde_json` 与 `sha2`）、`actingcommand-execution-kernel`（离线、按哈希准入的
任务包资格核对，同一 rev）、`sha2`（校验）、`zip`
（`default-features = false`，只开 `deflate`，与 Runtime 锁定的同一版本线）、`getrandom`（salt 与
`instance_id`）、
`ureq`（取件；`default-features = false`，只开 `tls`：rustls、它的 `ring` 实现与编译进去的
`webpki-roots`，不用系统 TLS 库），以及仅限 Windows 的 `windows` 0.62（`Win32_Foundation`、`Win32_System_Com`、
`Win32_System_RestartManager`、`Win32_UI_Shell`：经 `SHGetKnownFolderPath` 找启动、开始菜单与桌面文件夹，经
`IShellLinkW` + `IPersistFile` 写快捷方式，经重启管理器（Restart Manager）找出仍占着某个槽里文件的进程；与 Slint
已锁定的同一版本，锁文件不新增 crate）。

## 智能体用的 MCP

监控台自己没有 MCP 服务。MCP 服务由 Runtime 的命令行提供：`actingctl mcp-serve`（Runtime v0.11.0 起）是 stdio 上的
本地 MCP 服务，只有工具，供 Claude Code、Codex 等智能体使用。`actingctl mcp-config` 打印给某个客户端的注册内容，
不写任何客户端文件：

```powershell
& '<安装根>\runtime\actingctl.exe' mcp-config --client claude --tier observer
& '<安装根>\runtime\actingctl.exe' mcp-config --client codex --tier observer,operator
```

- **Claude Code**：打印一条命令
  `claude mcp add --scope user actingcommand -- "<安装根>\runtime\actingctl.exe" mcp-serve --tier …`，运行一次即可。
- **Codex**：打印一段 `[mcp_servers.actingcommand]`（命令、参数、两个超时，以及注释掉的 `enabled_tools` 示例），
  放进 Codex 的 `config.toml`。
- 在 A/B 安装里，它打印的总是固定入口 `<安装根>\runtime\actingctl.exe`，从不打印槽内路径，所以升级或回退之后注册
  仍然有效（Runtime v0.11.2 起）。经这个入口启动时，服务从安装的选择取状态根；与之不同的 `--root` 或
  `--state-root` 一律拒绝。
- **档位**（0.11 系列）：`observer`（只读；总是开着，也是默认）、`operator`（设备与调度）、`author`（Lab 录制）。
  未开档位的工具答 `tier_not_enabled`。`actingctl mcp-serve --list-tools [--format json|markdown]` 列出全部 22 个工具。
- 批准、actingd 配置的改动与 Runtime 的重启归人。
- 运行中的服务保持它启动时的安装代际（见「A/B 安装与升级」）；升级后请在客户端里重启它。
- **计划中（0.12 系列）**：取消档位；一般工具总在，Lab 工具只在装了 Lab 选项时列出。

每个工具接收什么、回答什么：`actingctl mcp-serve --list-tools --format markdown`。给智能体的操作手册是
[伞仓](https://github.com/HS7097/ActingCommand/tree/main/skills/actingcommand)里的 `skills/actingcommand/`。

## 四层四 crate

同一个 Cargo workspace，依赖方向 app → model → rows ← source：

- `acui-rows`：唯一为视图模型命名契约类型的地方；再导出契约类型，外加显示用函数、显示名
  字典，与两个由 `acui-source` 填、`acui-model` 读的平铺结构。
- `acui-source`：读面，唯一碰状态根的地方。离线读面是 `EvidenceSource::open` / `query` /
  `open_report` / `read_material`；`Session::open(root, mode)` 按 `--source`
  在它和在线的 `OnlineSource` 之间选一张，账本拒开离线读面时交回带账本原错误的
  `Session::Unopened`（`LedgerOpenFailure`：code、operation、detail、IO 类别），`material_reader()` 交给后台线程读素材。
  它还每个会话读一次实例的任务事实（`instance_facts()`）：离线在钉住的位置重放，在线在钉住之后连同
  实例状态一起读；事实快照的作用域与取值类型直接从契约 crate 取。
- `acui-model`：纯 Rust 视图模型（页签、过滤、翻页、恢复折叠、选中项），不依赖 slint，**也不
  出人话**——它只给结构化事实，措辞一律由 `acui-app` 按语言表挑。
- `acui-app`：唯一依赖 slint 的 crate，`.slint` 文件在 `crates/acui-app/ui/`；两张语言表在
  `strings.rs`，设置文件的读写在 `settings.rs`。

`slint` 1.17.x，`default-features = false`；账本对监控台只读（监控台发出的请求——启动按钮、关闭
请求、在线开台时的那次状态读取、实例发现查询——由 Runtime 自己记账），控制入口只有启动器的两个按钮（启动 /
请求关闭，见上）、启动器的解锁入口（经确认的 `actingd unlock-owner`）与实例配置窗口经 check-config
把关的保存，没有审批入口；监控台的 crate 不带测试，CI 只构建工作区、不跑测试。启动器在
`crates/acui-app/src/launcher.rs`，实例配置窗口在 `instances.rs`，探测、请求关闭、记下启动按钮、
在线开台时的状态与事实读取、实例发现这几个客户端操作在 `acui-source`（`probe_runtime` /
`request_shutdown` / `record_start` / `instance_facts` / `discover_instances`）。

每个后台工作线程——启动的就绪等待与记账、请求关闭、unlock-owner、保存时的 check-config、实例发现、
读帧，以及 acsetup 的每个工作线程（查询发布件、安装或升级、写入选项、实例发现、读取资源、写入实例、收尾确认）——都经 `std::thread::Builder` 启动。系统拒绝建线程时，在这个动作
回报的位置写明，附系统错误，并把工作线程本该复位的状态（进行中的启动或解锁、保存中、帧请求）复位：
绝不在事件循环里 panic。已拉起的 actingd 若等不到就绪线程，照样在跑，结果行直说，并提示再按一次「启动」即可探测。子进程终止了但
回收失败时照实写，不说成终止失败。

另有两个 crate 在这四层之外。`acui-setup`（二进制 `acsetup`）是安装向导，不依赖上面任何一层；除
`acui-installation` 外，它用 Slint、`serde`、`serde_json`、`anyhow` 以及「安装向导 acsetup」一节列出的依赖，其中
包括共享契约与 execution-kernel 的离线任务包资格核对。`acui-installation` 为监控台和向导提供共享的安装选择消费者，
并生成二进制 `acforward`，即固定入口。

## 图标

应用图标是黑色单人「指挥官」标记，素材在 `crates/acui-app/assets/`：
`acui-256.png`（256×256 透明 PNG）与 `acui.ico`（16..256 多尺寸）。

- **窗口与任务栏图标**：`app.slint` 的 `Window.icon: @image-url("../assets/acui-256.png")`。
- **可执行文件图标**：`build.rs` 里 `#[cfg(windows)]` 调 `winresource` 把 `acui.ico` 编进
  exe 资源段；这条依赖挂在 `[target.'cfg(windows)'.build-dependencies]` 下，Linux 上不编译。
- **acsetup**：同一套素材，不复制：`crates/acui-setup/build.rs` 与 `ui/setup.slint` 用相对路径指向
  `crates/acui-app/assets/` 里的这两个文件。

## 读面挡住的事

这些不是绕过去了，是照实显示、在此记录。文件都指钉住的 rev 上的 Runtime 源码：

- **事件条数与修复条数：离线都已解决，在线只有事件条数**。`GlobalLedgerMetadata`
  （`crates/ledger/src/global/evidence.rs`）现在给出 `event_count()` 与 `repair_count()`，取自已认证的
  元数据，不校验任何素材，监控台不再需要为此去调 `GlobalLedger::open_evidence`（同一文件）。实例卡两项都显示，另外标出**本视图已载入**的
  条数，两者不混用。读取不完整时事件条数只计已校验的前缀，实例卡写明这一点。SQLite 介质没有
  修复日志（`None`），实例卡照写，不显示成 0；修复日志与事件快照不共用同一个序号边界。在线时
  事件条数就是钉住的位置，依据 `contracts/runtime-state-observation.md`（序号从 1 起无缺口）；页
  （`LedgerReadScope`）与 `runtime-info.json` 都不给修复条数，这一行写 Runtime 不提供。
- **整份读素材：两张读面都已解决**。`read_material_complete`
  （`crates/ledger-forensics/src/material.rs`）整份读一个对象：重开两次账本元数据、一个
  reader、整份哈希一次，受 `max_material_bytes` 与期限约束。离线读面以 8 MiB 帧上限和 30 秒
  期限调用它（Runtime 里还没有它的调用方定下期限；契约的 4 秒 `RUNTIME_MATERIAL_READ_BUDGET_MS`
  约束的是单段读，不是整份对象）。在线由类型化客户端的 `RuntimeClient::read_material_complete`
  （`crates/runtime-client/src/client.rs`）在校验过的分段上给出同样形状的结果，监控台以同样
  的上限与期限调用它；Runtime 仍对每一段校验整份素材（一张 3.6 MB 的帧是 19 段 192 KiB）。
- **实例事实：两张读面都已解决，位置不同**。在线经 `RuntimeClient::runtime_fact_snapshot()`
  （`crates/runtime-client/src/client.rs`）读，它答的是 Runtime 最新位置上的状态，晚于钉点。离线由
  `runtime_facts_at`（`crates/ledger-forensics/src/runtime_facts.rs`）按 Runtime 自己的重放规则，在
  钉住的位置本身重放事实库；监控台从不自己折叠 `runtime.fact_*` 事件。租约只来自在线的状态读取，离线
  没有。
- **几何与帧在两个早期状态根上凑不到一起**。监控台最初核对用的两个早期实机状态根里，带 `capture.frame`
  产物的事件只有 `artifact.created` / `artifact.verified`，payload 里没有几何；带几何的事件只有寥寥几条
  `task.effect_intent`，payload 里是一个 tap 坐标，`links` 里**没有** `frame_id`。账本没有给出把这两者连起来的
  关系，监控台就不连——真实帧照画，叠加为空。钉住的 rev 上，`task.effect_intent` 可以给出坐标所在的画面范围
  （`frame_extent`，`crates/actingcommand-contract/src/event/payload.rs`），`task.geometry_observed` 可以给出其
  画面的范围（`TaskGeometryFrame::extent`，同一文件）；事件给了，叠加画布就用它。那两个根上的 effect intent
  都没给，尺寸仍是「未记录」。
- **那两个根里都没有产物淘汰事实**，所以淘汰占位在它们上面不会出现；代码路径按契约写好。

## 计划中（未发布）

本节全部是计划，尚未发布。各项归入哪个系列仍可能调整，不写日期。

**0.12 系列**

- **下一版 UI**（计划随 0.12 系列，作为 UI 的新大版本）：监控台读新账本；报错按结果码解释（中英）；监控台显示
  任务为什么没跑；安装器新增一页，给 Lab 模块两个相互独立的选项（创作；调试），默认都不装；安装器提供机读输出
  （`--json`）。
- **新账本**：Runtime 0.12.0 起用新的账本格式（修订 3），从空账本开始；0.11 的状态根不带过去（旧安装整体留作
  归档，不删）。预计要等下一版 UI 才能安装它。
- **结果码统一**：所有程序共用一张登记过的码目录，每个码有类别；每条命令输出一行结果，退出码收敛为 0/1/2。
- **统一接口**：一道门、三个前端——命令行、MCP 与本 UI——一一对应；每次请求只记来自哪个前端，权限不再按「谁」
  来定。MCP 档位取消。
- **Lab 改为可拆的模块**：默认不装。安装器两个选项勾任一就装：创作（录制与制包工具，给用智能体制作资源的人）、
  调试（直接操控 Runtime 内部动作，给高级用户）。不装时 Lab 通道在，但不可调用。
- **Runtime 一侧**：恢复阶梯配置化（改配置文件，或经命令行、MCP、UI 的一条指令立即生效）、性能节奏、为每个没跑的
  任务写明原因的调度规则、数据刷新、共享数据表、每实例目标、MCP 增补，以及系列末段支持今天 1280×720 基准以外的
  分辨率。

**0.13 系列**

- **MaaFramework 流水线格式导入导出**（「包视图」）：我们的任务包与 MaaFramework 形状的 JSON 互转；存回时过不了
  检包就报错、不写入。编辑界面用我们自己的 UI（节点图编辑器），不随发行带第三方编辑器。
- **Runtime 一侧**：由通用流程件组成的战斗层，以及净室重写的通用空间组件；一切游戏内容仍只在资源包里。

## 许可

`GPL-3.0-only`。仓库附 LICENSE 全文；工作区 `license` 字段与每个 `.rs` / `.slint` 文件的 SPDX 头与之一致。界面由
[Slint](https://slint.dev) 渲染，按其 GPLv3 许可选项使用。依赖的 Runtime crate（contract / ledger /
ledger-forensics / runtime-client / execution-kernel）为 `AGPL-3.0-only`，两者按 GPLv3 第 13 条合并。
