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

- **归类由契约说了算**：六个页签就是契约的六个 `LedgerView`。原先按 `event_type` 前缀与
  `origin.module` 猜的四条临时规则、以及「（临）」标记，全部删除。一行属于哪个页签，看
  页里那条事件自带的 `views`。页签上的数字是**已载入的页**里各视图的归属条数。
- **过滤是账本查询**：视图、严重度上下界、来源模块、`correlation_`/`request_`/`run_`/`task_`
  id、时间上界组装成一个 `EventQuery`，在**同一个快照位置**上重新查一次账本；不在本地
  已有的行上筛选然后自称是账本查询。id 必须是完整的规范 id，否则报「过滤条件无效」。
- **翻页是页游标**：「加载更多」拿页给的 `next_cursor` 取下一页并追加。中栏上方常驻
  「已读到 #N」，源不完整时追加「源不完整」。
- **恢复分组来自账本**：页里带的 `run_recovery` 在各自运行的第一行前插一条分组行，显示
  账本给的状态（已恢复 / 未解决 / 未知）、依据（失败 #N → 成功 #M）与缺口；被账本判为已
  恢复的失败行折叠在分组行下，带「已恢复」标记。这是读时分组，不是改写失败事件。
- **行类型不再镜像**：`acui-rows` 直接再导出契约类型，只额外提供本地时间、id 缩写与
  wire 码这些显示用函数。

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
- 解码只用 `image`（只开 `png` feature）。程序解码的图只有两类：这样读回来的帧，和自带的
  应用图标。

几何叠加与帧共用同一个坐标系：payload 给了画面尺寸就用它，没给就用解码出的像素尺寸。

## 运行

```
acui --state-root <state_root> [--tab <events|observation|changes|errors|health|lab>]
acui --help
```

`--tab` 指定启动页签（截图与复核用），取值就是视图自己的 wire 名。

窗口可缩放：默认 1400×900，最小 1100×700，中栏随窗口伸缩，两侧栏保持定宽。

## 四层四 crate

同一个 Cargo workspace，依赖方向 app → model → rows ← source：

- `acui-rows`：唯一为视图模型命名契约类型的地方；再导出契约类型，外加显示用函数与两个
  由 `acui-source` 填、`acui-model` 读的平铺结构。
- `acui-source`：读面，唯一碰状态根的地方。`EvidenceSource::open` / `query` /
  `open_report` / `read_material`。
- `acui-model`：纯 Rust 视图模型（页签、过滤、翻页、恢复折叠、选中项），不依赖 slint。
- `acui-app`：唯一依赖 slint 的 crate，`.slint` 文件在 `crates/acui-app/ui/`。

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
  「—（读面未给）」，另外标出**本视图已载入**的条数，两者不混用。
- **整份素材没有入口，读一帧很贵**。`crates/ledger-forensics/src/material.rs:51` 的
  `read_material_to` 只做一段，且每段都要重开两次账本元数据并把整份素材重新哈希一遍；
  读一张 3.6 MB 的帧要 57 段，实测约 5 秒（release）。没有整份读入口，也没有跨段复用的
  reader，所以监控台把读取放进后台线程，而不是自己去拼一套简化的读取流程。
- **几何与帧在这两个根上凑不到一起**。0828 与 v5 两个根里，带 `capture.frame` 产物的事件
  只有 `artifact.created` / `artifact.verified`，payload 里没有几何；带几何的事件只有
  `task.effect_intent`（0828 六条、v5 五条），payload 里是一个 tap 坐标，`links` 里**没有**
  `frame_id`。账本没有给出把这两者连起来的关系，监控台就不连——真实帧照画，叠加为空。
- **两个根里都没有产物淘汰事实**，所以淘汰占位在这两个根上不会出现；代码路径按契约写好。

## 待 Alice 裁定

- **许可**：工作区声明 `AGPL-3.0-only`，每个 `.rs` 文件带 SPDX 头，但**最终许可待裁定**，
  仓库暂未附 LICENSE 全文；裁定后再补。
- **Slint 许可选项**：界面由 [Slint](https://slint.dev) 渲染，选哪一种 Slint 许可待裁定。
