<p align="center">
  English | 中文
</p>

<p align="center">
  <img src="docs/media/banner.webp" alt="Horizon 网站：你所熟悉的那个游戏，用 Rust 重制" width="100%">
</p>

<p align="center">
  <a href="https://github.com/horizon-rust/horizon/actions/workflows/ci.yml"><img src="https://github.com/horizon-rust/horizon/actions/workflows/ci.yml/badge.svg?branch=main" alt="CI"></a>
  <img src="https://img.shields.io/badge/Minecraft-Java%2026.3-5D9C3F" alt="Minecraft Java 26.3">
  <img src="https://img.shields.io/badge/Rust-stable-B7410E?logo=rust&logoColor=white" alt="Rust">
  <img src="https://img.shields.io/badge/vanilla%20parity-100%25-F0795A" alt="原版对照 100%">
  <a href="#project-status"><img src="https://img.shields.io/badge/player--visible%20gaps-833%20open-F0795A" alt="开放的玩家可见差距"></a>
  <img src="https://img.shields.io/badge/platform-Linux%20%7C%20macOS%20%7C%20Windows-2A2470" alt="Linux、macOS 与 Windows">
  <a href="LICENSE"><img src="https://img.shields.io/badge/license-MIT-3B2F7E" alt="MIT 许可证"></a>
</p>

<p align="center">
  <a href="https://playhorizon.dev"><b>网站</b></a> ·
  <a href="#download"><b>下载</b></a> ·
  <a href="https://playhorizon.dev/docs/"><b>文档</b></a> ·
  <a href="https://github.com/horizon-rust/horizon/releases"><b>发布说明</b></a> ·
  <a href="ROADMAP.md"><b>路线图</b></a> ·
  <a href="#project-status"><b>项目状态</b></a> ·
  <a href="#contributing"><b>贡献</b></a>
</p>

# Horizon

Minecraft: Java Edition 26.3，用 Rust 重写。相同的世界、相同规则、相同地形，经由 300 多个差异测试套件与 Java 游戏比对验证。免费、开源、非官方。

> 并非官方 Minecraft 产品，亦未经 Mojang 或 Microsoft 批准或关联。Horizon 不包含任何 Minecraft 代码、资产或数据：你需要拥有自己的 Java 版 26.3 副本。

## Download

从 [playhorizon.dev](https://playhorizon.dev) 或 [启动器发布页](https://github.com/horizon-rust/horizon/releases?q=launcher-v&expanded=true) 获取 **Horizon 启动器**（Linux x86-64、Apple Silicon Mac、下个 Nightly 起支持 Intel Mac、Windows x64）。它安装并更新游戏、根据你的 Minecraft 副本准备游戏文件，并自行更新。

**你需要** 拥有 Java 版的 Microsoft 账户（[原因](docs/microsoft-account.md)）以及支持 Vulkan 或 OpenGL 3.3（Linux）、Vulkan 或 DirectX 12（Windows）或 Mac（Apple Silicon，或下个 Nightly 起的 Intel）的 GPU。已有 Minecraft 安装（官方启动器、Prism、MultiMC...）会被复用；没有安装时，Play 会为你的账户从 Mojang 下载 26.3。逐步说明：[安装与首次启动](https://playhorizon.dev/docs/install)。

<table>
  <tr>
    <td width="50%"><img src="docs/media/launcher-play.webp" alt="启动器主页：选中的实例准备就绪可开始游戏、版本选择器、邀请好友与实例标签页"></td>
    <td width="50%"><img src="docs/media/launcher-settings.webp" alt="启动器设置页：Microsoft 账户、离线名称、更新、外观与强调色"></td>
  </tr>
</table>

## What works today

- **游戏：** 原版世界生成、生物、村民、袭击、红石、全部命令、HUD 和配方书，运行于原版存档上。世界支持从 1.8 升级到 26.2。[状态](#项目状态)。
- **额外功能：** 新菜单、图形预设、最远 1024 区块的远景地形、控制器、缩放、小地图等，在改变原版外观处默认关闭。[设置](https://playhorizon.dev/docs/settings)。
- **资源包：** 支持资源包，以及包含函数、谓词、物品修改器与世界生成的数据包。
- **多人：** 用代码邀请好友、开放到局域网、加入服务器（含在线模式）。[多人](https://playhorizon.dev/docs/multiplayer)。
- **模组：** 内容模组（方块、物品、数据）与原生 Rust 模组，由启动器构建、无需安装。Java 模组 jar 无法运行。[模组](https://playhorizon.dev/docs/mods)。

在 Rust 客户端中，使用原版资源且无模组或着色器时：

<table>
  <tr>
    <td colspan="2"><img src="docs/media/sunset.webp" alt="落日映照在热带草原河谷与平原村庄上"></td>
  </tr>
  <tr>
    <td width="50%"><img src="docs/media/survival.webp" alt="黄昏求生：一只僵尸靠近，显示快捷栏、生命、饥饿与经验"></td>
    <td width="50%"><img src="docs/media/settings.webp" alt="原版视频设置界面，顶部为 Horizon 自己的按钮：Horizon Graphics 与 Advanced Graphics"></td>
  </tr>
  <tr>
    <td width="50%"><img src="docs/media/village.webp" alt="热带草原山丘间的平原村庄，包含农田、村民与铁傀儡"></td>
    <td width="50%"><img src="docs/media/nether.webp" alt="下界赤 crimson 森林中的僵尸猪灵"></td>
  </tr>
</table>

## Host a server

启动器的 **Servers** 页面可创建、启动、备份与更新服务器，并通过 Tailscale 邀请好友。在 VPS 上，`horizon-server` 一条命令安装且无需 Java。[托管服务器](https://playhorizon.dev/docs/hosting)（[参考](docs/hosting.md)）。

## Modpacks

启动器导入 Modrinth 与 CurseForge 模组包，并运行 Horizon 能迁移的模组：目前是仅由数据与资产构成的内容模组，而非 Java 代码。[模组包](https://playhorizon.dev/docs/modpacks)。

## Agent control

AI Agent 与脚本可游玩 Horizon。游戏附带的 `horizon-ctl` 将状态读取为 JSON 并像玩家一样行动；`horizon-ctl mcp` 通过 MCP 提供相同命令。

```sh
horizon-ctl launch --world Survival --seed 5 --wait   # finds the version the launcher installed
horizon-ctl state                                       # position, health, food, target, screen...
horizon-ctl find-block birch_log                        # the nearest logs
horizon-ctl goto -0.5 65 -2.5 --near 2.5                # walks there along a path
horizon-ctl break -1 65 -3                              # mines it; replies with the drops
horizon-ctl open-inventory && horizon-ctl click-slot 0  # menus: slots, buttons by label, text fields
```

命令会阻塞直到完成并返回结果；`horizon-ctl help` 列出全部命令。[Agent 控制](docs/native-client.md#agent-control-horizon-ctl-and-mcp)。

## Build from source

需要 Rust 1.90+、Python 3.10+、JDK 25，Linux 上需要 `libasound2-dev libudev-dev`。`horizon-prepare` 根据 **你自己的** Java 版 26.3 副本（按 `research-lock.json` 校验）构建游戏资源，且从不下载它：

1. **客户端：** 在官方启动器中玩一次 26.3。Horizon 读取 `~/.minecraft`（或 `--minecraft-dir` / `HORIZON_MINECRAFT_DIR`）。
2. **服务器 jar：** 26.3 服务器 jar（`versions/26.3/26.3.json` 中的 `downloads.server.url`），通过 `--server-jar` 或 `HORIZON_SERVER_JAR` 一次性传入。

```sh
git clone https://github.com/horizon-rust/horizon.git
cd horizon
cargo run --release -p horizon-prepare -- --prepare --server-jar ~/Downloads/server.jar   # needs JDK 25
cargo run --release -p horizon-prepare -- sounds   # optional: sounds from your launcher install
cargo run --release -p horizon-client
```

准备好的文件位于 gitignored 的 `.local/`：切勿提交或分享它们。选项、控制与架构：[原生客户端指南](docs/native-client.md)；全部开发者文档：[docs/README.md](docs/README.md)。

## Roadmap

进行中的工作、接下来的计划与如何参与：[ROADMAP.md](ROADMAP.md)，或含工作看板与已发布内容的 [playhorizon.dev/hub](https://playhorizon.dev/hub)。

## Contributing

欢迎 Issue 和 Pull Request：阅读 [CONTRIBUTING.md](CONTRIBUTING.md)（原版优先、小型 Pull Request）。问题讨论：[Discord](https://discord.gg/playhorizon)。提交 Pull Request 前，运行自检并粘贴其输出：

```sh
git config core.hooksPath .githooks
python3 tools/pr_ready.py
```

它会运行 rustfmt、clippy、受影响 crate 的测试、Python 测试、公开树检查以及你改动影响的原版测试套件。切勿提交游戏文件、反编译源码或准备好的资源。

## Project status

**玩家可见差距：开放 833 个**，共 833 个由 [port ledger](tools/port_ledger.py) 找到，读取 Java 26.3 的每个方法并对照 Horizon 的代码：Horizon 尚未具备的、玩家会注意到的原版行为。按领域分类：界面、HUD 与客户端数据包 [197](https://github.com/horizon-rust/horizon/issues/3518)；实体与方块渲染、模型 [129](https://github.com/horizon-rust/horizon/issues/3519)；动物 [110](https://github.com/horizon-rust/horizon/issues/3512)；服务器与服务器上的玩家 [106](https://github.com/horizon-rust/horizon/issues/3520)；方块、红石与流体 [86](https://github.com/horizon-rust/horizon/issues/3516)；实体、弹射物、载具与装饰 [64](https://github.com/horizon-rust/horizon/issues/3514)；怪物与 Boss [53](https://github.com/horizon-rust/horizon/issues/3513)；生物 AI、村民与袭击 [35](https://github.com/horizon-rust/horizon/issues/3521)；世界、世界生成与存档 [32](https://github.com/horizon-rust/horizon/issues/3517)；物品、附魔与物品栏 [21](https://github.com/horizon-rust/horizon/issues/3515)。随着差距关闭，这个数字会下降（`tools/parity_gaps.py`）；下方的测试结果说明已测试部分与原版一致。

表格是项目自身的审计：每个子系统在所有与之比较的测试套件中都与原版匹配。100% 表示测试所比较部分的 100%，而非再无其他差异。

### Java parity: 100%

得分：0（缺失）、0.25（已开始）、0.5（可玩，主要特性缺失）、0.75（基本完整）或 1（完整并经差异测试验证）；百分比为它们的平均值。最后审计于 2026-10-13：全部 281 个测试套件在 Linux 与 macOS 上通过。[port ledger](tools/port_ledger.py) 提供按方法查看的移植情况。

| 子系统 | 得分 | 主要差距 |
| --- | ---: | --- |
| 窗口、输入与游戏循环 | 1 | 无：游戏循环、选项、摄像机、自动跳跃、切换疾跑/潜行与帧率限制器均匹配 |
| 资产与资源 | 1 | 无：物品模型在超过 196,290 组堆叠下匹配；资源包界面超过 3,000 个会话匹配 |
| 网络协议与多人 | 1 | 无：可加入原版服务器；专用服务器匹配原版的文件、登录、配置、游戏与命令树；局域网发现超过 462 个用例（仅离线模式） |
| 区块存储与调色板 | 1 | 无：客户端与存档共享带调色板的容器，在 266 个场景下与原版一致 |
| 分区网格化 | 1 | 无：112 个场景与超过一百万个四边形与原版一致 |
| 渲染 | 1 | 无：实体模型、方块实体渲染器（407 个用例）与动画（296 个）、HUD 覆层（2,200 个状态）、配方书（600 个）、玩家列表、字幕与世界边界均匹配 |
| 光照 | 1 | 无：保存与重新计算的光照与原版匹配，包含半砖与楼梯 |
| 世界生成 | 1 | 无：在所有三个维度、225 个结构点位上，2,352 个完整区块与原版一致 |
| 区域文件与存档 | 1 | 无：存档可与原版往返；1.8 到 26.2 的世界按原版 DataFixer 升级 |
| 实体与物理 | 1 | 无：属性、生成规则、自然生成（15,500 个用例）、袭击与巡逻（1,900 个）、`finalizeSpawn`（24,000 个）与生物 AI（92 个场景）均匹配 |
| 交互与物品栏 | 1 | 无：菜单点击超过 8,000 个序列；配方书放置（6,000）、按钮（1,200）、已保存书本（1,500）、织布机与合成器界面（1,600）均匹配 |
| 菜单、HUD 与聊天 | 1 | 无：全部命令（758 个中有 39,685 个输入）、记分板（81,000 次操作）、玩家列表（872,000 次绘制调用）、聊天及其建议（3,000 个会话）均匹配 |
| 音频 | 1 | 无：声音引擎、声音事件、音乐、情境音乐、环境音与字幕均与原版匹配 |
| 方块行为与刻更新 | 1 | 无：方块更新、随机刻、红石、发射器（6,000 个用例）、合成器（4,000 个）、容器溢出（4,000 个）、铜傀儡与幽匿家族（435 个场景）均匹配 |

### Known issues

通过 https 提供的皮肤在未缓存时显示默认皮肤；游戏内资源包图标显示 unknown-pack 纹理；物品提示缺少属性行。

## Licence

[MIT](LICENSE)，适用于项目原创作品；依赖保留其各自许可证（[声明](THIRD_PARTY_NOTICES.md)）。Minecraft 是 Mojang Synergies AB 的商标；其代码与资产属于 Mojang 和 Microsoft，受 [Minecraft EULA](https://www.minecraft.net/en-us/eula) 约束。

## Author

由 [Ludovic Marie](https://lu4ovic.com)（[@ludovic111](https://github.com/ludovic111)）制作。
