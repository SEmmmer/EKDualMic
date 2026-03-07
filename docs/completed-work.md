# Completed Work

这个文件是 EKDualMic 的持续交接记录。后续 Codex 在本仓库完成了任何非琐碎实现、修复、验证或测试流程调整后，都应在同一个变更里更新这里。

## Read First

继续工作前，先按这个顺序读：

1. `README.md`
2. `Dual-Mic-Crosstalk-Canceller-README.md`
3. 按任务需要阅读 `docs/architecture.md`、`docs/config.md`、`docs/tuning.md`
4. 本文件
5. 如果要做 Windows 测试，再读 `docs/windows-test.md`

## Update Rules

- 只把“已有代码 + 已做验证”的内容写成已完成。
- 每次完成重要改动后，追加一条带绝对日期的日志，不要只改口头描述。
- 日志至少写清楚：改了什么、怎么验证、还剩什么风险或未完成项。
- 如果 Windows 测试方式或验收条件变了，同时更新 `docs/windows-test.md`。
- 如果入口命令、关键配置或项目范围说明变了，同时更新 `README.md`。

## Current Completed Scope

### Workspace And Docs

- Rust workspace 已拆成独立 crate：`common_types`、`audio_capture`、`audio_transport`、`audio_sync`、`audio_vad`、`audio_cancel`、`audio_residual`、`audio_output`、`app`
- 工具入口已提供：`tools/offline_replay`、`tools/wav_dump`
- 配置、架构、调参文档已建立

### Core Runtime Skeleton

- 固定 `48 kHz` / `mono` / `10 ms` 帧结构已经统一到 `common_types`
- `PipelineRuntime` 已串起采集、传输、对齐、VAD、NLMS、残差抑制、输出、调试导出
- 调试导出已支持 `metrics.tsv` 和多路 WAV：`local_raw`、`peer_raw`、`peer_aligned`、`output`

### Implemented Algorithm Stubs And First-Pass Logic

- `audio_transport`
  - `udp` 传输
  - `mock` 传输
  - 基础 jitter buffer / concealment / sequence 追踪
- `audio_sync`
  - 基于历史帧相关性的粗对齐
  - 输出 `coarse_delay_ms` / `coherence`
  - `drift_ppm` 目前仍固定为 `0.0`
- `audio_vad`
  - 基于能量的平滑 VAD
- `audio_cancel`
  - 时域 NLMS
  - 支持冻结更新和状态重置
- `audio_residual`
  - 轻量残差抑制
- `audio_output`
  - `wav_dump`
  - Windows render endpoint 输出桥接 MVP（现有 `virtual_stub` backend 在 Windows 上会写到目标 render device）

### Tooling And GUI

- `audio_device_probe` 能列出当前激活的 Windows capture / render devices，并标记默认设备
- `offline_replay` 能用配置启动离线处理并导出调试文件
- `runtime_smoke` 能按原始配置启动 headless runtime，用于 mock / WASAPI 冒烟验证
- `wav_dump` 能统计 WAV 的采样格式、时长、峰值和 RMS
- Windows GUI 已有 `eframe/egui` 外壳，可启动、加载配置、驱动 runtime、显示实时指标，并展示当前 capture / render 设备列表
- GUI 已支持读取当前配置文件中的 `input_device` / `target_device`，并把点选后的设备字段保存回 TOML
- GUI worker 现已支持运行中 `Reload Runtime`、`Save Device Fields` 后自动 reload，以及 runtime 构建/运行失败后的周期性重试恢复

### Windows Capture

- `audio_capture`
  - Windows `wasapi` 后端已实现第一版 shared-mode 真实采集
  - 当前固定输出 `48 kHz` / `mono` / `float32` / `10 ms`
  - 支持按 friendly name 选择输入设备，也支持 `input_device = "default"` 选择默认输入端点

### Mock Validation Path

- 当 `audio.backend = "mock"` 且 `node.transport_backend = "mock"` 时，runtime 不再使用简单本地回环
- 当前会启用内置双人交替说话场景：
  - `peer_raw` 为对端原始参考
  - `local_raw` 为“本机近端 + 延迟后的对端串音 + 极低底噪”的混合
  - 可真实触发 sync 对齐、VAD 冻结和 NLMS 收敛
- 已提供可直接用于 GUI / Windows 验证的 `configs/node-a-mock.toml`

## Known Gaps

- `audio_sync` 还没有细粒度漂移补偿，也没有更稳的延迟跟踪
- `audio_capture` 当前还是第一版 MVP：底层还没有更细粒度的原地 reopen；当前恢复依赖 GUI worker 级 runtime 重建
- `audio_output` 当前仍是“写到现有 render endpoint”的桥接，不会由仓库自己创建新的系统级 capture endpoint
- `audio_output` 还没有更细粒度的原地热切换和异常恢复；当前恢复依赖 GUI worker 级 runtime 重建
- GUI 当前是控制面和指标面，不是完整产品界面
- 真实双机 Windows 端到端链路还不能宣称完成

## Implementation Log

### 2026-03-08

- 建立了 Rust workspace 骨架、crate 切分、TOML 配置、架构/配置/调参文档、`offline_replay`、`wav_dump` 和 Windows GUI 外壳。
- 实现了第一版 runtime 主链路：采集、传输、粗对齐、VAD、NLMS、残差抑制、输出、调试导出。
- 实现了 mock 双人场景，替代原先的简单回环，使离线链路可以真实验证 `mock_peer_delay_ms`、冻结/更新窗口和能量下降。
- 增加了 `app` 内部端到端测试，要求 mock 场景里出现 `update_frozen = false`，并在对端单讲阶段出现明显衰减。
- 增加了持续交接文档 `docs/completed-work.md`、Windows 测试文档 `docs/windows-test.md` 和推荐测试配置 `configs/node-a-mock.toml`。
- 安装了本地 skill `ek-dual-mic-maintainer`，要求后续 Codex 先读项目说明，再把完成项回写到本文件。
- 将 `ek-dual-mic-maintainer` 的版本化副本纳入仓库 `skills/ek-dual-mic-maintainer/`，避免 skill 只存在于本机 `~/.codex/skills/`。

验证：

- `cargo check --workspace`
- `cargo test --workspace`
- `cargo run -q -p offline_replay -- configs/node-a.toml 180`
- `cargo run -q -p wav_dump -- artifacts/offline/processed-output.wav`
- `python3 /home/emmmer/.codex/skills/.system/skill-creator/scripts/quick_validate.py /home/emmmer/.codex/skills/ek-dual-mic-maintainer`
- `python3 /home/emmmer/.codex/skills/.system/skill-creator/scripts/quick_validate.py /home/emmmer/git/EKDualMic/skills/ek-dual-mic-maintainer`
- `cargo run -q -p offline_replay -- configs/node-a-mock.toml 10`

关键结果：

- `offline_replay` 180 帧运行成功，最终 `coherence=0.999`
- `artifacts/offline/metrics.tsv` 中后段可见 `coarse_delay_ms=20.000`
- `artifacts/offline/metrics.tsv` 中帧 `174-180` 的 `input_rms` 约 `0.031`，`output_rms` 已降至约 `0.0013-0.0039`

仍未完成：

- WASAPI 真实采集
- 系统级虚拟麦输出
- 漂移补偿与更稳的同步策略

### 2026-03-08（Windows 构建复核）

- 按仓库交接规则重新检查了全部 Markdown 文档：`README.md`、`Dual-Mic-Crosstalk-Canceller-README.md`、`docs/architecture.md`、`docs/config.md`、`docs/completed-work.md`、`docs/tuning.md`、`docs/windows-test.md`、`skills/ek-dual-mic-maintainer/SKILL.md`。
- 在当前 Windows 工作区完成了整仓构建复核，没有修改代码实现；本次工作的目标是确认文档描述与现有构建状态一致，并继续产出 Windows 侧可用的 release 构建。

验证：

- `cargo check --workspace`
- `cargo test --workspace`
- `cargo run -q -p offline_replay -- configs/node-a.toml 180`
- `cargo run -q -p wav_dump -- artifacts/offline/processed-output.wav`
- `cargo build --workspace --release`
- 启动 `target/release/app.exe` 并观察 5 秒，进程保持运行后再停止，用于确认 GUI 可拉起

关键结果：

- `cargo check --workspace` 与 `cargo test --workspace` 在 Windows 上均通过；`app` 的 mock 集成测试仍然通过。
- `offline_replay -- configs/node-a.toml 180` 仍可成功运行，产出 `artifacts/offline/` 下的 `metrics.tsv`、`local_raw.wav`、`peer_raw.wav`、`peer_aligned.wav`、`output.wav`、`processed-output.wav`。
- `wav_dump` 读取 `artifacts/offline/processed-output.wav` 成功，结果为 `48 kHz`、单声道、`float32`、`duration_s=1.800`、`rms=0.052491`。
- `artifacts/offline/metrics.tsv` 末段仍显示 `coarse_delay_ms=20.000`；帧 `174-180` 的 `output_rms` 约为 `0.00131-0.00387`，继续明显低于对应 `input_rms`。
- `cargo build --workspace --release` 在 Windows 上通过，`target/release/app.exe` 能成功启动并保持运行，说明 GUI 二进制已可生成且至少通过基础启动冒烟测试。

仍未完成：

- WASAPI 真实采集仍未实现，默认 `configs/node-a.toml` / `configs/node-b.toml` 的实时 GUI 启动预期仍会在采集阶段报错。
- 系统级虚拟麦输出仍是 `virtual_stub`，尚未形成可供 Discord / OBS 直接使用的真实 Windows 输入端点。
- 这次仅完成了构建、离线链路和 GUI 启动级验证，未做人工 GUI 交互或真实双机联调。

### 2026-03-08（WASAPI 采集 MVP）

- 在 `crates/audio_capture` 实现了 Windows WASAPI shared-mode 采集 MVP，当前固定输出 `48 kHz` / `mono` / `float32` / `10 ms` 帧。
- 增加了按 friendly name 选设备和 `input_device = "default"` 选默认输入设备的路径；当配置的设备名不存在时，会明确报错并列出当前激活的 capture 设备。
- 新增 headless 验证工具 `tools/runtime_smoke`，用于按原始配置直接启动 runtime，而不是像 `offline_replay` 一样强制改成 mock。
- 新增 `configs/node-a-wasapi-wav.toml`，用于“真实 WASAPI 采集 + WAV 落盘”验证；同时更新了 `README.md`、`docs/config.md` 和 `docs/windows-test.md`，把 Windows 端验证步骤切换到新入口。

验证：

- `cargo check -p audio_capture`
- `cargo check -p runtime_smoke`
- `cargo check --workspace`
- `cargo test --workspace`
- `cargo build --workspace --release`
- `cargo run -q -p runtime_smoke -- configs/node-a-mock.toml 20`
- `cargo run -q -p runtime_smoke -- configs/node-a-wasapi-wav.toml 10`
- `cargo run -q -p runtime_smoke -- configs/node-a.toml 1`
- `cargo run -q -p wav_dump -- artifacts/windows-wasapi/processed-output.wav`

关键结果：

- `runtime_smoke -- configs/node-a-wasapi-wav.toml 10` 在当前 Windows 机器上成功读取默认输入设备 `Microphone (PRO X 2 LIGHTSPEED)`，并完成 10 帧真实采集链路。
- `runtime_smoke -- configs/node-a.toml 1` 在当前机器上按预期失败，因为占位设备名 `Microphone (Headset A)` 不存在；错误信息同时列出了当前激活的 capture 设备。
- `artifacts/windows-wasapi/` 下已生成 `local_raw.wav`、`peer_raw.wav`、`peer_aligned.wav`、`output.wav`、`processed-output.wav` 和 `metrics.tsv`。
- `wav_dump` 读取 `artifacts/windows-wasapi/processed-output.wav` 成功，结果为 `48 kHz`、单声道、`float32`、`duration_s=0.100`、`rms=0.000011`。
- 在没有真实对端节点的情况下，`artifacts/windows-wasapi/metrics.tsv` 前几帧 `loss_rate=1.0000`，这是当前单节点 capture smoke 场景的预期结果，不是回归。
- `cargo build --workspace --release` 通过，说明新的 WASAPI 采集链路和 `runtime_smoke` 工具已经进入可发布的 release 构建。

仍未完成：

- 系统级虚拟麦输出仍未实现，`virtual_stub` 依旧不会把处理后音频送进真正的 Windows 输入端点。
- 真实双机 UDP + 实时消除 + 系统级输出的端到端 Windows 联调还没有完成。
- WASAPI 采集当前是第一版 MVP，还没有做设备断开恢复、热切换和更完整的错误恢复。

### 2026-03-08（输出桥接 MVP）

- 在 `crates/audio_output` 实现了 Windows render endpoint 输出桥接 MVP；现有 `virtual_stub` backend 在 Windows 上会选择目标 render device，并把处理后 `48 kHz` / `mono` / `float32` / `10 ms` 帧写进去。
- 增加了按 friendly name 选择输出设备和 `target_device = "default"` 选择默认输出设备的路径；当配置的 render device 不存在时，会明确报错并列出当前激活的 render 设备。
- 新增 `configs/node-a-mock-render.toml`，用于“mock 输入 + 真实 render endpoint 输出”的最小风险验证路径，避免真实麦克风回灌。
- 更新了 `README.md`、`docs/config.md` 和 `docs/windows-test.md`，把 Windows 输出桥接的现状、验证方式和剩余缺口写回仓库。

验证：

- `cargo check -p audio_output`
- `cargo check -p runtime_smoke`
- `cargo check --workspace`
- `cargo test --workspace`
- `cargo build --workspace --release`
- `cargo run -q -p runtime_smoke -- configs/node-a-mock-render.toml 5`

关键结果：

- `runtime_smoke -- configs/node-a-mock-render.toml 5` 在当前 Windows 机器上成功完成 mock 输入 + 默认 render endpoint 输出桥接，没有再走空写入 stub。
- `artifacts/windows-render/` 下已生成 `metrics.tsv`、`local_raw.wav`、`peer_raw.wav`、`peer_aligned.wav` 和 `output.wav`，说明 runtime 与调试导出链路仍然稳定。
- workspace 级 `check` / `test` 继续通过，说明输出桥接改动没有破坏既有 mock 测试和其它 crate。
- `cargo build --workspace --release` 通过，说明输出桥接链路也已经进入可发布的 release 构建。

仍未完成：

- 仓库仍不会创建新的系统级 capture endpoint；如果要被 Discord / OBS 当作“虚拟麦”使用，仍需要把输出桥接到外部虚拟声卡的输入端点。
- 真实双机实时输出联调还没有完成。
- 输出侧也还没有做设备断开恢复、热切换和更完整的异常恢复。

### 2026-03-08（设备探测与 GUI 可观测性）

- 为 `audio_capture` / `audio_output` 增加了可复用的 Windows 设备枚举接口，统一返回当前激活的 capture / render endpoint 列表、设备 id 和默认标记。
- 新增 `tools/audio_device_probe`，可直接在 Windows 上打印当前 capture / render 设备清单，避免后续配置 `input_device` / `target_device` 还要人工猜名字。
- 更新了 GUI 左侧控制面，增加 `Refresh Devices` 按钮与 capture / render 设备列表显示，帮助在 GUI 里直接核对默认设备和 friendly name。
- 更新了 `README.md` 与 `docs/windows-test.md`，把 `audio_device_probe` 纳入标准 Windows 测试和配置流程。

验证：

- `cargo check -p audio_capture`
- `cargo check -p audio_output`
- `cargo check -p app`
- `cargo check -p audio_device_probe`
- `cargo check --workspace`
- `cargo test --workspace`
- `cargo run -q -p audio_device_probe`
- `cargo build --workspace --release`
- 启动 `target/release/app.exe` 并观察 5 秒，进程保持运行后再停止

关键结果：

- `audio_device_probe` 在当前 Windows 机器上成功列出 3 个激活的 capture devices 与 5 个激活的 render devices，并正确标记默认设备。
- 当前机器的默认 capture device 为 `Microphone (PRO X 2 LIGHTSPEED)`，默认 render device 为 `扬声器 (PRO X 2 LIGHTSPEED)`；这些结果已经可直接用于配置文件填写。
- workspace 级 `check` / `test` / `release build` 全部继续通过，说明设备探测与 GUI 可观测性改动没有破坏现有运行链路。
- `target/release/app.exe` 在当前机器上仍可成功启动并保持运行，说明 GUI 设备列表改动没有破坏基础窗口启动路径。

仍未完成：

- 设备探测目前只提供“读出当前设备”能力，还没有做 GUI 内直接改写配置文件或热切换设备。
- 设备断开恢复、自动重连和更完整的 operator UX 仍未完成。

### 2026-03-08（GUI 设备字段编辑）

- 在 `crates/app/src/config.rs` 增加了 `save_config`，并补了配置保存/重载 round-trip 单元测试，覆盖设备字段的持久化路径。
- 更新了 GUI 左侧控制面，新增 `Load Config`、`Save Device Fields`、`Audio Input Device`、`Output Target Device` 和 `Use Default Capture/Render` 操作，使当前设备探测结果可以直接写回 TOML。
- 设备列表现在不只是只读展示：点击 capture / render 设备条目会把对应 friendly name 填到可编辑字段里，便于直接保存到当前配置。
- 更新了 `README.md` 和 `docs/windows-test.md`，把 GUI 里的设备字段读写操作纳入标准 Windows operator 流程。

验证：

- `cargo check -p app`
- `cargo check --workspace`
- `cargo test -p app`
- `cargo test --workspace`
- `cargo build --workspace --release`
- 启动 `target/release/app.exe` 并观察 5 秒，进程保持运行后再停止

关键结果：

- `app` crate 现在包含 2 个测试，其中新的 `config::tests::save_and_reload_config_round_trip_preserves_device_fields` 已通过。
- GUI 新增的配置读写与设备字段状态没有破坏窗口启动，`target/release/app.exe` 仍能成功启动并保持运行。
- 当前 GUI 已足以完成“看设备列表 -> 点选设备 -> 保存回配置 -> 再启动 runtime”的单机 operator 流程。

仍未完成：

- GUI 目前仍然只保存设备字段，没有做完整配置编辑、自动热加载或运行中热切换。
- 更完整的 Windows operator UX 仍需要围绕真实双机联调继续补。

### 2026-03-08（运行中 reload 与断开恢复）

- 在 `crates/app/src/gui.rs` 重构了 GUI worker：新增 worker 控制通道和 `Reload Runtime` 命令，不再把 runtime 错误直接当成线程终止条件。
- worker 现在会在配置加载失败、`PipelineRuntime::new` 失败、以及运行中 `runtime.step()` 失败后进入 `Recovering: ...` 状态，并按固定间隔重试重建 runtime。
- `Save Device Fields` 在 runtime 运行中保存成功后会自动请求 reload，因此 capture / render 设备切换不再需要整 GUI 进程退出重开。
- 为新的 worker 生命周期补了 2 个单元测试：一个覆盖运行中 reload，另一个覆盖“启动时配置缺失，随后创建配置并自动恢复”。
- 为 `NodeGuiApp` 增加了 `Drop` 清理，确保关闭 GUI 窗口时会主动停掉 worker 线程，而不是把后台线程遗留在进程生命周期外。
- 更新了 `README.md` 与 `docs/windows-test.md`，把运行中 reload、恢复态和新的 GUI 验证路径写回文档。

验证：

- `cargo check -p app`
- `cargo test -p app`
- `cargo check --workspace`
- `cargo test --workspace`
- `cargo build --workspace --release`
- 启动 `target/release/app.exe` 并观察 5 秒，进程保持运行后再停止

关键结果：

- `app` crate 现在包含 4 个测试，其中新增的 `gui::tests::worker_reloads_runtime_without_full_gui_restart` 与 `gui::tests::worker_recovers_after_missing_config_is_created` 已通过。
- GUI worker 不再在首次配置/设备错误后直接退出，而是会持续进入 `Recovering: ...` 并重试，直到用户修正配置、设备恢复或点击 `Stop`。
- 运行中的 `Save Device Fields` 现在会触发 runtime reload，因此单机 operator 流程已经从“停 runtime -> 改 TOML -> 再启动”缩短为“点选设备 -> 保存 -> 自动重建 runtime”。
- `target/release/app.exe` 在当前 Windows 机器上仍可成功启动并保持运行，说明新的 worker 生命周期与窗口关闭清理没有破坏 release GUI 启动路径。

仍未完成：

- 当前恢复是 GUI worker 级的整 runtime 重建，还不是 capture/output 级的原地 reopen；因此 DSP 状态和 transport 状态在恢复时会重置。
- 还没有接入更细粒度的 Windows 设备通知；当前依赖运行失败后重试，而不是提前订阅 endpoint 变更事件。
- 真实双机 + 外部虚拟声卡场景下的长期人工联调还没有完成。

### 2026-03-08（GUI 中文字体 fallback）

- 在 `crates/app/src/gui.rs` 的 GUI 启动路径里增加了 Windows CJK 字体 fallback 安装逻辑，优先尝试 `NotoSansSC-VF.ttf` / `NotoSerifSC-VF.ttf`，再回退到微软雅黑、黑体、宋体等系统字体。
- 新字体会作为 `egui` 的 proportional / monospace fallback 挂入默认字体链，因此中文设备名、中文状态文案和后续中文 UI 文本都可以直接渲染，不需要额外打包字体文件。
- 更新了 `README.md` 和 `docs/windows-test.md`，把“GUI 可正常显示中文设备名”写入当前能力与验证预期。

验证：

- `cargo check -p app`
- `cargo test -p app`
- `cargo build -p app`
- 启动 `target/debug/app.exe` 并观察 5 秒，进程保持运行后再停止

关键结果：

- `app` crate 编译和现有测试继续通过，说明字体 fallback 没有破坏 GUI worker、runtime 控制或配置读写逻辑。
- 当前 Windows 机器上的字体探测路径可命中 `C:\Windows\Fonts\NotoSansSC-VF.ttf`，因此 GUI 具备稳定的中文 glyph fallback 来源。
- `target/debug/app.exe` 启动冒烟通过，说明新的字体加载逻辑不会阻塞或破坏基础窗口创建。

仍未完成：

- 这次只解决了“能显示中文”，没有做完整中英文本地化；当前大部分 GUI 文案仍是英文。
- 字体 fallback 目前只覆盖 Windows；如果未来 GUI 扩到其它平台，需要单独补对应平台的字体探测策略。

### 2026-03-08（GUI 配置加载反馈与启动前设备同步）

- 调整了 `crates/app/src/gui.rs` 的 GUI 配置交互：`Load Config` 现在会把当前 TOML 中的设备字段重新装入界面，并明确更新状态栏与配置反馈，不再表现为“点了没反应”。
- 如果 runtime 已在运行，`Load Config` 现在也会自动请求 reload，让从磁盘重新装入的设备字段直接作用到当前 worker。
- `Start` 现在会先把界面里当前的 `Audio Input Device` / `Output Target Device` 同步回配置文件，再启动 runtime，因此仅在 GUI 中点选设备也会生效，不再强依赖先点一次 `Save Device Fields`。
- 为新的配置装载/设备同步行为增加了 GUI 单元测试，覆盖“Load Config 会更新 UI 字段与状态”和“界面设备字段可被持久化回配置文件”。
- 更新了 `README.md` 与 `docs/windows-test.md`，把新的 GUI operator 行为写回仓库说明。

验证：

- `cargo check -p app`
- `cargo test -p app`

关键结果：

- `app` crate 测试数已增至 6 个，新增的 `gui::tests::reload_config_fields_updates_ui_values_and_status` 与 `gui::tests::persist_ui_device_fields_to_config_writes_selected_devices` 已通过。
- `Load Config` 点击后现在至少会更新 `Status` 和配置反馈文本，因此用户能立即知道配置是否真正从目标路径装入。
- GUI 内点选设备后直接 `Start`，runtime 会使用当前界面字段同步后的配置，而不是继续吃旧的磁盘设备名。

仍未完成：

- `Load Config` 目前只显式覆盖设备字段，没有把所有 TOML 字段做成完整的 GUI 表单编辑器。
- GUI 仍未提供更丰富的错误提示样式；当前主要依赖状态栏和简短的文本反馈。

### 2026-03-08（配置路径解析与 Load Config 下拉菜单）

- 在 `crates/app/src/config.rs` 为配置读写增加了相对路径解析逻辑；当 GUI 从 `target/debug` 或 `target/release` 启动时，`configs/node-a.toml` 这类相对路径现在会沿祖先目录自动回溯到仓库根目录，而不是错误地只在当前工作目录下查找。
- 新增了配置预设发现逻辑，会扫描仓库 `configs/` 目录下的 `.toml` 文件并返回排序后的 GUI 预设列表。
- 在 `crates/app/src/gui.rs` 中把 `Load Config` 改成了下拉菜单，直接列出已发现的配置预设；同时保留 `Config Path` 文本框和 `Load Current Path`，便于手填任意路径后加载。
- 为新的配置路径解析与预设发现逻辑补了 2 个单元测试，覆盖“从 workspace 根目录解析相对配置路径”和“扫描 configs/ 目录列出 TOML 预设”。
- 更新了 `README.md` 与 `docs/windows-test.md`，把新的 `Load Config` 下拉行为和相对路径解析规则写回仓库说明。

验证：

- `cargo check -p app`
- `cargo test -p app`
- 启动 `target/debug/app.exe` 并观察 5 秒，进程保持运行后再停止

关键结果：

- `app` crate 测试数已增至 8 个，新增的 `config::tests::resolve_config_path_from_workspace_root_for_relative_paths` 与 `config::tests::discover_config_presets_from_workspace_lists_toml_files` 已通过。
- `Load Config` 现在不再依赖 GUI 进程的当前工作目录必须是仓库根目录，因此双击 `app.exe` 或从 `target/` 子目录启动时也能正确读取默认配置。
- GUI 现在可以直接从下拉菜单选择 `configs/node-a.toml`、`configs/node-a-mock.toml`、`configs/node-a-wasapi-wav.toml` 等预设，而不必完全依赖手动输入路径。

仍未完成：

- 预设发现当前只扫描仓库 `configs/` 根目录，不会递归子目录。
- GUI 目前还没有文件选择器；手工加载仓库外的 TOML 仍需要自己输入路径。

### 2026-03-08（Realtime Metrics 可视化面板）

- 在 `crates/app/src/gui.rs` 中为 GUI 增加了 metrics 历史缓存，并把 `Realtime Metrics` 从纯文本网格改成可视化面板。
- 新面板包含四个顶部状态卡，以及 `Audio Levels`、`Sync And Voice Activity`、`Transport Health`、`Timing` 四组图形化指标展示。
- 对关键指标加入了历史折线图：输入/输出/crosstalk RMS、coherence、本地/对端 VAD、transport loss、coarse delay、processing time。
- 对当前帧关键值加入了进度条可视化，例如 coherence、VAD、transport loss、input/output RMS、attenuation、processing time。
- 补充了 `gui::tests::record_snapshot_trims_metric_history`，保证 metrics 历史窗口固定裁剪，不会在长时间运行时无限增长。
- 更新了 `README.md` 与 `docs/windows-test.md`，把 GUI 已经支持可视化 metrics 的事实写回仓库说明和测试预期。

验证：

- `cargo check -p app`
- `cargo test -p app`
- 先结束 `target/` 下运行中的进程，再执行 `cargo build -p app`
- 启动 `target/debug/app.exe` 并观察 5 秒，进程保持运行后再停止

关键结果：

- `app` crate 测试数已增至 9 个，新增的 `gui::tests::record_snapshot_trims_metric_history` 已通过。
- GUI 可在不引入额外 plotting crate 的前提下显示实时趋势和当前值，编译与启动路径均保持稳定。
- 可视化历史窗口目前固定保留最近 `240` 帧，约对应 `2.4` 秒的 `10 ms` 实时运行窗口。

仍未完成：

- 当前图表仍是轻量级自绘 sparkline，不支持缩放、悬停 tooltip 或多段时间窗口切换。
- 指标展示主要覆盖 runtime snapshot 已暴露的数据，尚未扩展到更深的调试内部态。

### 2026-03-08（logs 目录与 panic 落盘）

- 在 `crates/app/src/config.rs` 的 `init_logging` 中增加了文件日志落盘能力；GUI 启动后会在仓库根目录自动创建 `logs/`，并按 `app-<pid>-<timestamp>.log` 生成独立日志文件。
- 当前日志会记录 tracing 输出，并包含源码文件/行号、线程名等上下文，方便定位 GUI 闪退或 runtime 线程异常。
- 额外挂上了 panic hook；如果 GUI 进程发生 panic，会把 panic 信息和 backtrace 追加写入对应日志文件，便于事后查验。
- 保留现有控制台/GUI 行为不变；新增日志目录主要用于离线排障，不依赖用户必须从终端启动程序。
- 更新了 `README.md` 与 `docs/windows-test.md`，把 `logs/` 目录纳入当前能力说明与排障入口。

验证：

- `cargo check -p app`
- `cargo test -p app`
- 按规则结束 `target/` 下运行中的程序后执行 `cargo build -p app`
- 启动 `target/debug/app.exe` 并观察 5 秒，进程保持运行后再停止
- 检查仓库根目录 `logs/` 下新生成的日志文件内容

关键结果：

- 当前机器上已生成 `logs/app-19676-1772912541.log` 与 `logs/app-50068-1772912520.log`，说明按启动批次分文件落盘已经生效。
- 日志文件中已成功记录 GUI 启动和字体 fallback 安装信息，证明 tracing 输出确实写入了 `logs/` 而不是只留在控制台。
- 后续如果再次出现 `Start` 后闪退，可直接优先查看 `logs/` 下最新的 `app-*.log`，而不需要先复现到终端窗口中。

仍未完成：

- 目前日志还没有做按大小或按日期滚动清理；`logs/` 目录会持续累积，后续可能需要补自动清理策略。
- panic/backtrace 已落盘，但尚未做“最近一次崩溃摘要”或 GUI 内直达日志路径的入口。

### 2026-03-08（通过 logs 修复 Start 后闪退）

- 检查了仓库 `logs/` 下现有的全部 GUI 日志，重点分析了 `app-46364-1772912611.log` 和 `app-33140-1772912678.log` 中的 panic/backtrace。
- 崩溃根因不是 WASAPI、UDP 或设备切换，而是 `Realtime Metrics` 可视化面板引入了无穷宽度布局；日志明确显示 `egui` 在 `Ui::new_child` 处 panic，报错为 `max_rect is NaN`，调用栈落在 `draw_metrics_dashboard`。
- 在 `crates/app/src/gui.rs` 中移除了 `ProgressBar::desired_width(f32::INFINITY)`，改为基于有限 `available_width` 的安全宽度；同时为 history chart 的绘制宽度增加了 `is_finite` 防御，避免布局链路再次产生 `NaN` 几何。
- 新增了 `gui::tests::metrics_dashboard_renders_without_nan_geometry`，直接在测试里渲染 metrics 面板，确保这条 GUI 渲染路径不会再因为几何异常 panic。

验证：

- 检查 `logs/` 下全部现有日志文件
- `cargo check -p app`
- `cargo test -p app`
- 按规则结束 `target/` 下运行中的程序后执行 `cargo build -p app`

关键结果：

- 最新有问题的日志已经把 crash 定位到 `draw_metrics_dashboard` 布局，而不是 runtime 音频线程。
- `app` crate 测试数已增至 10 个，新增的 metrics 渲染测试通过，说明当前代码可在测试中完整走通可视化面板的绘制路径。
- 编译与测试在修复后全部通过，说明这次修复没有破坏既有的配置、worker 恢复和 runtime mock 测试链路。

仍未完成：

- 我这里没有做 GUI 点击自动化，因此“用户手动点击 Start 后长时间跑稳”还需要你在本机再试一次。
- 如果后续仍出现闪退，应优先继续查看 `logs/` 下最新的 `app-*.log`；现在至少 panic 根因会完整落盘。

### 2026-03-08（滚动面板与 mock 模式显式提示）

- 根据最新日志重新检查了 GUI 行为；最新 `app-27912-1772912924.log` 显示你实际先后加载了 `node-a-mock`、`node-a-mock-render`、`node-a-wasapi-wav` 等配置，其中 `node-a-mock` 场景下 runtime 虽然显示了你填写的设备名，但音频 backend 仍然是 `mock`。
- 这解释了“a mock 看起来只是固定波形、不随麦克风变化”的现象：`configs/node-a-mock.toml` 本来就是 `audio.backend = "mock"`、`transport_backend = "mock"`，它使用的是内置仿真场景，不会读取真实麦克风。
- 在 `crates/app/src/gui.rs` 中为左侧控制面板和 `Realtime Metrics` 面板都增加了垂直 `ScrollArea`，现在页面内容超出一屏时可以直接用鼠标滚轮滚动。
- 在 GUI 中增加了当前已加载配置的 mode 摘要和 warning：如果加载的是 `audio=mock`，会明确提示“真实麦克风输入会被忽略”；如果输出不是 live endpoint，也会提示 `target_device` 在当前模式下无效。
- 进一步把 capture / render 设备编辑区按 backend 启停：mock 输入模式下禁用输入设备编辑和列表选择；`wav_dump/null` 输出模式下禁用输出设备编辑和列表选择，减少误操作和错觉。

验证：

- 检查最新的 `logs/` 文件，特别是 `app-27912-1772912924.log`
- 按规则结束 `target/` 下运行中的程序后执行 `cargo check -p app`
- `cargo test -p app`
- `cargo build -p app`
- 启动 `target/debug/app.exe` 并观察 5 秒，进程保持运行后再停止

关键结果：

- 最新日志没有再出现 panic；同时日志明确说明 `node-a-mock` 模式下使用的是 mock 场景，而不是实时麦克风。
- `app` crate 现有 10 个测试继续全部通过，说明滚动区和 backend warning 没有破坏既有 GUI 恢复与 metrics 渲染路径。
- GUI 现在既能滚动浏览长页面，也能直接把“为什么 mock 不跟真实麦克风波动”解释清楚，而不是只靠用户从配置文件里自己推断。

仍未完成：

- mock 模式当前仍是固定的内置双人仿真场景，不会混入真实麦克风；如果未来希望做“真实麦克风 + mock 对端参考”的混合模式，需要单独设计新的配置和 runtime 分支。
- 我这里没有自动化验证鼠标滚轮事件本身，只验证了 GUI 新布局能正常启动、渲染和编译通过。

### 2026-03-08（GUI 状态与错误同步进 logs）

- 根据最新日志排障需求，在 `crates/app/src/gui.rs` 里把 GUI 端的重要状态变化也接入了 tracing：配置加载成功/失败、保存失败、runtime reload 请求、`Recovering: ...`、worker stop 等事件现在都会写进 `logs/`。
- 这样即使界面上的状态文字只是短暂闪过，后续也能在日志文件里看到同样的文本，不必再只依赖人工截图或口述。
- 当前不会把每一帧 `Running: frame ...` 都写进日志，避免把真正有用的异常信息淹没。

验证：

- `cargo check -p app`
- `cargo test -p app`
- 按规则结束 `target/` 下运行中的程序后执行 `cargo build -p app`

关键结果：

- 最新代码会把 GUI 的关键异常/恢复状态同步到 `logs/`，后续若再次出现 `node-a-wasapi-wav` 反复恢复或 `Config load failed`，日志里能直接看到原因文本。
- `app` crate 的 10 个测试继续全部通过，说明日志同步没有破坏现有 GUI 行为。

仍未完成：

- 目前日志仍以文本事件为主，没有把完整 GUI 状态快照结构化成 JSON 或单独的诊断文件。

### 2026-03-08（单机 WASAPI GUI 的 UDP concealment 修正）

- 检查了最新日志 `app-32216-1772913562.log`，确认 `node-a-wasapi-wav` 的真实麦克风采集已经成功启动；反复进入 `Recovering` 的根因不是 WASAPI，而是单机无 peer 时 `udp` 接收被当成了致命错误。
- 在 `crates/audio_transport/src/lib.rs` 中把一组 Windows 常见的 UDP “对端不存在/不可达”错误改成非致命分支；当前会回到 concealment，而不是让 GUI worker 反复重建 runtime。
- 为 transport 增加了 2 个单元测试，覆盖哪些 `ErrorKind` 会被当成暂时性 peer 缺失，哪些仍保持致命错误。
- 同时把 `gui.rs` 的 GUI 文本错误同步扩展为完整 error chain，后续如果再出现 `Recovering`，日志里会同时包含上层语义和底层错误链。
- 更新了 `README.md`、`docs/windows-test.md`、`docs/architecture.md` 和 `skills/ek-dual-mic-maintainer/SKILL.md`，把“单机 `node-a-wasapi-wav` 应继续运行而不是因 UDP 接收失败重建”以及“先看 `logs/` 排障”写回交接资料。

验证：

- 检查最新 `logs/app-32216-1772913562.log`
- `cargo test -p audio_transport`
- `cargo test -p app`
- `cargo run -q -p runtime_smoke -- configs/node-a-wasapi-wav.toml 10`

关键结果：

- `runtime_smoke -- configs/node-a-wasapi-wav.toml 10` 已在当前 Windows 机器上成功跑完，不再因为单机无 peer 的 UDP 接收错误失败。
- 最新 transport 测试与 app 测试均通过，说明新的 UDP 容错没有破坏 mock/runtime 既有行为。
- 交接文档现在明确区分了 `mock` 配置与真实麦克风采集配置，并把最新的 logs/ 排障入口写进了 repo skill。

仍未完成：

- 当前 transport 容错只覆盖了一组常见的无 peer UDP 错误；如果未来出现其它 Windows 特定 socket 错误，仍需要继续补充分类。
- 真实双机 UDP 联调仍未完成；这里只修复了单机 capture smoke / GUI 验证路径。
