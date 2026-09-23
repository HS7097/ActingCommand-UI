<p align="right">🌐 <a href="./README.md">English</a> · <b>简体中文</b></p>

# ActingCommand 监控台

**⚠️ 本程序仍在快速迭代，预计 2–5 星期内完成。**

这是 ActingCommand 的**人类监控台**，一个只读的原生程序。它在一个 Runtime 状态根上打开
账本的**正式读面**，把账本自己给出的视图页渲染成三栏界面：实例卡、时间线、详情。

它不是 Runtime 的一部分，是 Runtime 的**外部可拆客户端**。

## 数据来源：读面，不是文件

程序只认一个参数：状态根。**读账本数据的文件 IO 都归读面**，监控台自己从不拼接状态根里的路径，
不打开 `ledger/`、`artifacts/` 或 `runtime-state.sqlite`，也不为读数据拉起任何 CLI。它拉起的
进程只有两种：actingd 本身（「启动」，见「启动器」一节），和保存实例配置时用来校验的
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
  「运行时实例」几行。状态只在人按「跳到最新」时重读；跟随时钉点每移动一次，只重读任务事实。
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

依赖钉在 Runtime **main** 上（`Cargo.toml`）：

```
rev = "5e462b0a6d1ba730d510ca3d23e658eddbc69114"
```

四个 crate（contract / ledger / ledger-forensics / runtime-client）共用这一个 rev。
`Cargo.lock` 入库，CI 在 windows-latest 与 ubuntu-latest 上跑
`cargo build --locked --release --workspace`；闭包里含 `rusqlite`（bundled），两边都要 C 编译器。

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
- **从最新一端读起**：账本查询没有降序，所以时间线从钉住的位置往回按 256 个位置一窗读。一窗里的
  事件不多于一页的条数上限，一次查询就能读完（回复被字节上限拆页时才跟着游标读），各项过滤都由账本
  做。每次页查询都要 Runtime 把整本账读一遍、校验一遍——在线时占用它的账本写线程——所以一次填充至少
  读一窗，多出 256 行、读到位置 1、读满 16 窗或已过 1 秒之后，就不再开始新的一窗。行按**最新在上**排列；底部
  的「读更早」接着往下读。顶栏常驻已读窗口覆盖
  的位置（「已读序号 A–B」），有窗口的页说读取不完整时追加「源不完整」。
- **跳到最新、跟随最新（只在在线读面）**：「跳到最新」用一张新的第一页把钉点移到 Runtime 的最新位置，
  重读状态与事实（状态读取会在账本留一条观察事件），去掉时间上界，从新钉点重新读起。「跟随最新」先
  同样对齐但不读状态，之后每 5 秒读一次 Runtime 的事实快照——它在内存里的事实库，位置就是账本的最新
  位置，不读账本也不写账本。只有这个位置动了，钉点才移过去，并用页查询把新旧钉点之间的窗口读到视图
  顶部；已载入的行、选中项和正在读的帧都保留，任务事实直接取自同一张快照。跟随中设了时间上界会停止
  跟随。时间跨度的终点跟着钉点走：新窗里有钉点上那条事件就直接取它的时间，否则再读一条。某次跟随
  失败时，在视图自己的错误旁写「跟随最新：…」，直到后面某次跟随越过它或关掉跟随才消失。离线没有
  在跑的 Runtime 写新事件，两个控件都不可用。
- **性能监视的例行事件默认隐藏**：`perf.summary` 每 2 秒一条，会淹没别的事件，而账本查询不能排除
  某个模块。除非勾选「显示性能监视」、在模块框里选了性能监视、或打开的是由这些事件组成的「健康」
  页签，监控台在读每一窗时丢掉性能监视**低于警告级别**的事件，在已载入条数旁写明丢了多少，并把性能
  监视留在模块列表里，仍可选中。它的警告和错误（磁盘压力、高压力、采样失败）始终显示。实例卡上的
  条数与级别计数只算已载入的行。这是唯一一处在监控台已拿到的行上做的过滤；等账本查询能排除事件
  类型，就改由查询来做。
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

裁定已改：监控台**会**载入帧字节，但只走素材读面，且只在下面这条规则内：

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

- **页标签**，在左上角：这一帧上识别匹配到的页面，或写明没有匹配。
- **点击标记**：以操作意图的每个点（`action.x, y`；滑动、拖动每个点各一个）为圆心、带白圈的
  圆点。它取代事件自己的 `action` 几何，免得同一次输入画两遍。
- **一句说明**，在帧下面，每个在这一帧上的输入一句：「步骤 0 notice_close：识别到
  bluearchive/news，点击 (1142, 102)」。
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
契约里全部 120 个 `event_type` 与 20 个 `origin.module`；**表里没有的一律照原样显示，不猜**。

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
actingd_config = 'D:\ActingCommand\actingd.config.json'  # 可选，绝对路径
actingd_exe = 'D:\ActingCommand\actingcommand-actingd.exe'   # 可选，绝对路径
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
  任何结果之后入口回到第一步；新的一次启动收起它，解锁进行中「启动」被拒。监控台不为它记
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

暂停/恢复、开机自启、安装器、联网下载都不在这一片里。

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
  （MuMu 序号）、`instance_name`（MuMu 名称），或 `host` + `port`（显式 ADB 地址）。`adb_path` 在
  `host` + `port` 下必填，在 MuMu 绑定下选填（由 MuMu 发现报告 adb）；`nemu_app_index` 是选填的整
  数。`application_id`、`capture_backend`、`touch_backend` 表单不检查，要不要填、取值是否有效都由
  check-config 判定，`nemu_app_index` 的配对也由它查。只有要靠 MuMu 发现结果的几项到 Runtime 启动
  时才查：`MuMuManager` 版本与能力、发现结果恰好匹配一个、声明的 `adb_path`、`host`、`port` 与发现值
  不冲突、ADB 端点（Runtime `contracts/actingd-check-config.md`，`3d5398d6` 起）。文本去掉首尾空白，
  留空的框不写这个键；必填项为空、数字解析不了，都在写任何东西之前直说。
- **保存**：重新把文件当普通 JSON 读——`actingd_config` 没配或不是绝对路径、文件不存在或读不出、
  JSON 解析失败、没有 `instances` 数组、某一项不是对象，各自直说。新实例追加进去；已有的按
  `instance_id` 重新找到（文件里已经没有了，或已被改成表单存不了的样子，就停下并写明原因），只改
  表单管的键，换了绑定种类就删掉别的种类的键。这一项和整个文件里其余的键一概原样、次序不变。结果
  写到同目录的 `<配置文件名>.candidate-<pid>`（里面的相对路径按这个目录解析），在事件循环之外跑
  `<actingd_exe> check-config --config <临时文件>`（30 秒上限，不弹控制台窗口，stdout 整段按一份
  `actingcommand.actingd.check-config.v1` 报告解析）。只有 `status: ok` 且退出码成功才改名覆盖原
  文件；否则删掉临时文件、原文件不动，窗口写明原因：原样写出 `error.code` 与 `stage`，或是
  `actingd_exe` 没配或非绝对、写临时文件失败、拉起失败、读输出的线程起不来、读子进程状态失败、
  超时（这三种还写明 check-config 能否终止、终止后能否回收）、输出读不出或无法识别、报 ok 但退出码非零、改名失败
  中的哪一种。
- **生效**：没有热加载，保存的实例在 Runtime 重启后生效。保存之后在事件循环之外探测一次，写明现在
  有没有 Runtime 在跑，并指向启动器自己的按钮：先「请求关闭」，停下后再「启动」——没在跑就直接
  「启动」。窗口本身不重启任何东西。

## 安装引导程序 acsetup

`crates/acui-setup` 是一个独立的二进制 `acsetup.exe`（Slint 窗口，与监控台同一套样式与图标），把
伞仓 [Releases](https://github.com/HS7097/ActingCommand/releases) 里的发布件装成一份**按用户**的安装。
它随 UI 仓的 Windows 构建产物一起发布（`acui-windows-<sha>.zip` 里多一个 `acsetup.exe`）。
它自己去取发布件，也可以用人手工下好的文件夹。一个窗口，只有下一步，五步：

0. **位置**：只有安装根（可改，默认 `%LOCALAPPDATA%\Programs\ActingCommand`，不需要管理员）、该卷的
   可用空间、此处是否已有安装（看 `runtime\BUILD-MANIFEST.json`；已有就停在这一步——升级流程是下一片，
   现在先换一个根）。下一步时建好安装根与安装日志。
1. **取件**：默认联网。一进这一步就经 HTTPS 向伞仓 [Releases](https://github.com/HS7097/ActingCommand/releases)
   要一个发布件：有正式版取最新正式版（GitHub 的 `releases/latest`），否则取最新预发布（每日构建），从不取草稿；页面与日志写明它的标签、
   名称、日期、种类与大小。下一步时依次下载 `SHA256SUMS`、`MEMBERS.json`，再下载 `SHA256SUMS` 列出的
   其余文件——别的一个不下——存到 `<安装根>\downloads\<标签>\`；每个文件先写 `.part`，长度等于发布件
   声明的长度才改名，1 MiB 以上的文件每满十分之一写一行进度；目录里已有的同名文件重新下载，从不直接采信，
   下载失败的 `.part` 文件会删掉。标签与每个文件名都只在只含字母、数字、`.`、`-`、`_`，不以点开头，且不是
   Windows 设备名时才使用。只走 HTTPS（重定向也一样）；连接一分钟没有数据即失败。改勾**离线**则用一个已放好同一发布件全部文件的文件夹（默认
   `%USERPROFILE%\Downloads`，路径直接填）。查询失败写在页面与日志里，离线仍可选（勾上再取消即重新查询）；下载失败则停下。这是
   程序唯一的联网代码（`ureq`，阻塞式，rustls 加编译进去的 Mozilla 根证书）；Runtime 没有任何联网代码。
2. **校验与铺开**，一步做完：要求文件夹里有 `SHA256SUMS`、`MEMBERS.json`、`actingcommand-runtime-<sha>.zip`、
   `actingcommand-tools-<sha>.zip`、`acui-windows-<sha>.zip`（`<sha>` 取 `MEMBERS.json` 的
   `runtime_sha` / `ui_sha`，三个 zip 必须在 `SHA256SUMS` 里）。逐条核对 `SHA256SUMS`；解压到安装根
   下的临时目录 `.staging-<unix_ms>`；再按每个 zip 自带的 `BUILD-MANIFEST.json` 核对来源仓、提交号
   （等于 MEMBERS 的 sha）、Runtime 的 `runtime_payload_layout`（`distribution-v1`），以及 `files[]`
   每一项的大小与 sha256；zip 里多出清单没列的文件也算不一致。任何不一致都停下，措辞是
   「内容与创建时不一致」——这是完整性陈述，不是授权口吻。校验期间不运行 zip 里的任何东西。随后铺开：
   `runtime\`（Runtime 全部载荷 + 清单，`actingd.config.example.json` 逐字节原样）、
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
   留空，之后点监控台顶栏的「实例配置」按钮添加；完成页也这样写。
   **开机自启**在同一页（可选，默认不勾）：勾了才写按用户的启动文件夹里的
   `%APPDATA%\Microsoft\Windows\Start Menu\Programs\Startup\ActingCommand.cmd`，内容是
   `start "" "<安装根>\runtime\actingcommand-actingd.exe" --config "<安装根>\actingd.config.json"`；
   再勾「同时拉起监控台」才多一行 `start "" "<安装根>\ui\acui.exe"`。批处理里不出现 acsetup。
   不勾就什么也不写；启动文件夹里已有的同名文件不动，只在完成页说一句。
4. **完成**：「启动监控台 / Open console」分离拉起 `<安装根>\ui\acui.exe`（从不直接拉 actingd，
   Runtime 由监控台的启动器拉）并关闭引导；「完成」只关闭。

**安装日志**：离开第 0 步起每一步都往 `<安装根>\acsetup-<unix_ms>.log` 追加人话行；失败时最后一行写
原因，窗口上显示日志路径。除安装载荷、配置、设置、`downloads\` 下取回的发布件与（勾选时的）自启批处理
之外，引导写的文件只有这一个。

**永远不做的事**：不装服务、不建计划任务、不改 PATH、不写注册表；不改配置模板；不碰已有内容的状态根；
不配置实例；联网只为列出与下载伞仓发布件；不做升级、不装资源包。Linux 上 crate 照常编译（CI 两条腿都跑 `--workspace`），
运行即以 `acsetup v1 is Windows-only` 退出。

依赖多四个，都在 `[workspace.dependencies]` 里注明用途：`sha2`（校验）、`zip`
（`default-features = false`，只开 `deflate`，与 Runtime 锁定的同一版本线）、`getrandom`（salt）、
`ureq`（取件；`default-features = false`，只开 `tls`：rustls、它的 `ring` 实现与编译进去的
`webpki-roots`，不用系统 TLS 库）。

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
把关的保存，没有审批入口；不写测试。启动器在
`crates/acui-app/src/launcher.rs`，实例配置窗口在 `instances.rs`，探测、请求关闭、记下启动按钮、
在线开台时的状态与事实读取、实例发现这几个客户端操作在 `acui-source`（`probe_runtime` /
`request_shutdown` / `record_start` / `instance_facts` / `discover_instances`）。

每个后台工作线程——启动的就绪等待与记账、请求关闭、unlock-owner、保存时的 check-config、实例发现、
读帧，以及 acsetup 的校验与铺开两步——都经 `std::thread::Builder` 启动。系统拒绝建线程时，在这个动作
回报的位置写明，附系统错误，并把工作线程本该复位的状态（进行中的启动或解锁、保存中、帧请求）复位：
绝不在事件循环里 panic。已拉起的 actingd 若等不到就绪线程，照样在跑，结果行直说，并提示再按一次「启动」即可探测。子进程终止了但
回收失败时照实写，不说成终止失败。

第五个 crate `acui-setup`（二进制 `acsetup`）在这四层之外：安装引导程序，只依赖 slint、serde、sha2、
zip、getrandom、ureq，不依赖上面任何一层，见上一节「安装引导程序 acsetup」。

## 图标

应用图标是 Alice 裁定的黑色单人「指挥官」标记，素材在 `crates/acui-app/assets/`：
`acui-256.png`（256×256 透明 PNG）与 `acui.ico`（16..256 多尺寸）。

- **窗口与任务栏图标**：`app.slint` 的 `Window.icon: @image-url("../assets/acui-256.png")`。
- **可执行文件图标**：`build.rs` 里 `#[cfg(windows)]` 调 `winresource` 把 `acui.ico` 编进
  exe 资源段；这条依赖挂在 `[target.'cfg(windows)'.build-dependencies]` 下，Linux 上不编译。
- **acsetup**：同一套素材，不复制：`crates/acui-setup/build.rs` 与 `ui/setup.slint` 用相对路径指向
  `crates/acui-app/assets/` 里的这两个文件。

## 读面挡住的事

这些不是绕过去了，是照实显示、在此记账。行号都指钉住的 rev：

- **事件条数与修复条数：离线都已解决，在线只有事件条数**。`GlobalLedgerMetadata`
  （`crates/ledger/src/global/evidence.rs:257`）现在给出 `event_count()`（`:319`）与
  `repair_count()`（`:326`），取自已认证的元数据，不校验任何素材，监控台不再需要为此去调
  `GlobalLedger::open_evidence`（同文件 `:434`）。实例卡两项都显示，另外标出**本视图已载入**的
  条数，两者不混用。读取不完整时事件条数只计已校验的前缀，实例卡写明这一点。SQLite 介质没有
  修复日志（`None`），实例卡照写，不显示成 0；修复日志与事件快照不共用同一个序号边界。在线时
  事件条数就是钉住的位置，依据 `contracts/runtime-state-observation.md`（序号从 1 起无缺口）；页
  （`LedgerReadScope`）与 `runtime-info.json` 都不给修复条数，这一行写 Runtime 不提供。
- **整份读素材：两张读面都已解决**。`read_material_complete`
  （`crates/ledger-forensics/src/material.rs:74`）整份读一个对象：重开两次账本元数据、一个
  reader、整份哈希一次，受 `max_material_bytes` 与期限约束。离线读面以 8 MiB 帧上限和 30 秒
  期限调用它（Runtime 里还没有它的调用方定下期限；契约的 4 秒 `RUNTIME_MATERIAL_READ_BUDGET_MS`
  约束的是单段读，不是整份对象）。在线由类型化客户端的 `RuntimeClient::read_material_complete`
  （`crates/runtime-client/src/client.rs:2089`）在校验过的分段上给出同样形状的结果，监控台以同样
  的上限与期限调用它；Runtime 仍对每一段校验整份素材（一张 3.6 MB 的帧是 19 段 192 KiB）。
- **实例事实：两张读面都已解决，位置不同**。在线经 `RuntimeClient::runtime_fact_snapshot()`
  （`crates/runtime-client/src/client.rs:848`）读，它答的是 Runtime 最新位置上的状态，晚于钉点。离线由
  `runtime_facts_at`（`crates/ledger-forensics/src/runtime_facts.rs:61`）按 Runtime 自己的重放规则，在
  钉住的位置本身重放事实库；监控台从不自己折叠 `runtime.fact_*` 事件。租约只来自在线的状态读取，离线
  没有。
- **几何与帧在这两个根上凑不到一起**。0828 与 v5 两个根里，带 `capture.frame` 产物的事件
  只有 `artifact.created` / `artifact.verified`，payload 里没有几何；带几何的事件只有
  `task.effect_intent`（0828 六条、v5 五条），payload 里是一个 tap 坐标，`links` 里**没有**
  `frame_id`。账本没有给出把这两者连起来的关系，监控台就不连——真实帧照画，叠加为空。
  钉住的 rev 上，`task.effect_intent` 可以给出坐标所在的画面范围（`frame_extent`，
  `crates/actingcommand-contract/src/event/payload.rs:3315`），`task.geometry_observed` 可以给出其
  画面的范围（`:3041`）；事件给了，叠加画布就用它。这两个根上的 effect intent 都没给，尺寸仍是
  「未记录」。
- **两个根里都没有产物淘汰事实**，所以淘汰占位在这两个根上不会出现；代码路径按契约写好。

## 许可

`GPL-3.0-only`（Alice 2026-09-17 裁定）。仓库附 LICENSE 全文；工作区 `license` 字段与每个 `.rs` / `.slint`
文件的 SPDX 头与之一致。界面由 [Slint](https://slint.dev) 渲染，按其 GPLv3 许可选项使用。依赖的 Runtime
crate（contract / ledger / ledger-forensics / runtime-client）为 `AGPL-3.0-only`，两者按 GPLv3 第 13 条合并。
