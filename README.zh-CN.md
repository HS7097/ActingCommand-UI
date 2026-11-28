<p align="right">🌐 <a href="./README.md">English</a> · <b>简体中文</b></p>

# ActingCommand 监控台

**⚠️ 预发布，调试阶段：接口、配置与文件格式还会变，只支持最新版。**

本仓是 ActingCommand 的**人类监控台** `acui` 和**安装向导** `acsetup`，都是原生程序。监控台在一个 Runtime 状态根上打开账本的正式读面，把账本自己给出的视图页渲染成三栏界面（实例卡、时间线、详情）；账本它只读不写。它不是 Runtime 的一部分，是 Runtime 的外部可拆客户端。与 Runtime 一样，这些程序不认识任何游戏：不含游戏逻辑，也不含游戏数据；游戏经 Runtime 加载的资源包接入。

## 现状

写于 2026-11-29，对应最新的 **UI v0.12.37**。

- **发布。** 最新的是 UI v0.12.37，在本仓 [Releases](https://github.com/HS7097/ActingCommand-UI/releases) 页：`acui-windows-<sha>.zip` 与 `SHA256SUMS`。zip 里有 `acui.exe`（监控台）、`acsetup.exe`（安装向导）、`LICENSE`、`README.md`，以及把它们绑到所在提交的 `BUILD-MANIFEST.json`。每一版都是预发布，所以 GitHub 上没有标「Latest」的版本，取标签最新的那个。
- **搭配。** v0.12.37 基于 Runtime v0.12.104 构建，声明 `actingd-config` [5, 32]、`ledger` [3, 11]、`runtime-client` [12, 15] 与 `runtime-outcomes` [1, 1]，可与 Runtime v0.12.73 到 v0.12.104 搭配，配不了 v0.12.72 及更早的。Runtime 从 v0.12.86 起写账本修订 10，UI v0.12.33 及更早的版本离线读不了。能否搭配按双方声明的接口判断，不看版本号。0.11 系列不再维护。
- **监控台现在能做的。** 看账本：实例卡、时间线、详情三栏；主页右栏有「实时画面」「日常任务」「目标」三个页签。经 Runtime 的 `ui` 通道，它还能暂停与恢复调度（顶栏的「全部暂停」「全部开始执勤」，以及「暂停这个实例的调度」）、在「目标」页设定目标，并启动 Runtime 或请求它关闭。界面有中文和英文，字号三档，主题两种。
- **安装向导现在能做的。** `acsetup` 从一份安装集，把 Runtime、工具和监控台装进一个新文件夹，并把 MuMu 模拟器里、或用 USB 连着的安卓手机上登录好的游戏接上；它不联网，不下载任何东西。手机能连接、能看画面，在手机上跑任务要等后续版本。安装集要另外拼（`tools/make_install_set.py`），目前没有公开发布的安装集。
- **还没有的。** 监控台里没有实例配置窗口，「添加实例」按钮是灰的；添加实例用 Runtime Tools 里的 `actinglab instance add`。
- **实机现状。** 在项目自己的机器上，监控台配着 0.12 系列的 Runtime 使用；那里的 Runtime 调度六台 MuMu 实例，四个标准包每天按目录跑例行批次。标准包未公开。

## 文档

- [REFERENCE.zh-CN.md](./REFERENCE.zh-CN.md)：从本页移出的详细参考（各版本怎样搭配、数据来源、运行、设置文件、启动器、安装向导、四层四 crate、读面挡住的事），不随每次发布更新。
- 每一版改了什么：[Releases](https://github.com/HS7097/ActingCommand-UI/releases) 页上各版的说明。
- [model/README.md](./model/README.md)：本仓里的离线建模工具 `acmodel`。

## 许可

`GPL-3.0-only`。仓库附 LICENSE 全文；工作区 `license` 字段与每个 `.rs` / `.slint` 文件的 SPDX 头与之一致。界面由 [Slint](https://slint.dev) 渲染，按其 GPLv3 许可选项使用。依赖的 Runtime crate（contract / display-adapter / ledger / ledger-forensics / runtime-client / runtime-config）为 `AGPL-3.0-only`，两者按 GPLv3 第 13 条合并。

本仓没有其他自动化项目的源码。整个项目族的上游许可矩阵见伞仓的 [LICENSE_POLICY.md](https://github.com/HS7097/ActingCommand/blob/main/LICENSE_POLICY.md)。
