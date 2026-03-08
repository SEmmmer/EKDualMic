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

### 2026-03-08（GUI 支持双机 IP 互连配置）

- 为 `crates/app/src/gui.rs` 增加了 `Local Listen Address` 和 `Peer Address` 表单字段，并把它们接入 `Load Config` / `Load Current Path` / `Start` / `Save Runtime Fields` 的整条持久化路径；现在不需要手改 TOML，也能在 GUI 中直接完成双机按 IP 互连的配置。
- 运行时表单保存逻辑已从“只保存设备字段”升级为“同时保存设备字段和网络字段”；当前点击 `Start` 时也会先把界面里的 `listen_addr` / `peer_addr` 同步回配置，再启动 runtime。
- 对输入做了轻量归一化：如果在 GUI 的 `Peer Address` 或 `Local Listen Address` 中只输入纯 IP，例如 `192.168.1.22`，保存时会自动沿用该配置原有的端口，例如补成 `192.168.1.22:38001`。
- 为 GUI 增加了 transport backend 语义：当当前配置使用 `transport_backend = "mock"` 时，会明确提示网络地址字段被忽略，并禁用这两个字段，避免把 mock 场景误认为真实双机 UDP。
- 在 `crates/app/src/config.rs` 为 UDP 配置增加了前置校验：`node.listen_addr` 和 `node.peer_addr` 现在必须是合法的 `IP:port`，并且端口不能为 `0`，这样地址错误会在加载/保存配置时被直接指出，而不是等到 runtime bind/send 阶段才暴露。
- 更新了 `README.md`、`docs/config.md`、`docs/windows-test.md` 和 repo skill，写回了推荐双机局域网用法：两边都监听 `0.0.0.0:38001`，并把 `peer_addr` 指向对端机器的 `IP:38001`。

验证：

- `cargo fmt --all`
- `cargo check -p app`
- `cargo test -p app`

关键结果：

- GUI 现在已经具备“按 IP 配置双机互连”的 operator 入口，transport 底层不再只是由仓库内预设 TOML 间接驱动。
- 新增测试覆盖了：配置重新加载会把网络字段带回 UI；保存 runtime 字段时会把纯 IP 自动补全为带端口的地址；UDP 地址格式校验可以直接拦截非法输入。

仍未完成：

- 我这里还没有做两台真实 Windows 机器的人工端到端联调，因此“配置入口已经打通”不等于“双机语音效果已验收完成”。
- 当前 GUI 仍然要求手工知道对端机器 IP；还没有设备发现、广播配对或二维码之类的发现机制。

### 2026-03-08（Realtime Metrics 默认四列并支持尺寸切换）

- 按最新反馈继续收紧了 `crates/app/src/gui.rs` 的 metrics 区布局：`Realtime Metrics` 标题旁新增 `Metrics Size` 按钮组，提供 `Small`、`Medium`、`Large` 三档视图切换。
- 默认档位现在是 `Small`，会把 4 个指标面板压缩成一行 4 个，同时减小状态卡高度、图表高度和进度条高度，避免 metrics 区继续占掉过多垂直空间。
- `Medium` 会回到 2 列面板，`Large` 会进一步放大到 1 列面板，方便在大字体或排障时查看单个图表细节。
- 为 GUI 测试增加了默认布局断言，确认默认启动时 `metrics_panel_size = Compact` 且 metrics 区使用 4 列布局。
- 同步更新了 `README.md` 和 `docs/windows-test.md`，把“默认 Small 模式下一行 4 个面板”和“支持尺寸切换按钮”写回说明。

验证：

- `cargo fmt --all`
- `cargo check -p app`
- `cargo test -p app`
- `cargo build --workspace --release`

关键结果：

- metrics 区默认布局现在从“2 个大面板一行”变成“4 个小面板一行”，更适合在当前 GUI 窗口宽度下同时观察所有关键指标。
- 现有 metrics 渲染测试与新增默认布局测试都通过，说明这次缩小和重排没有重新引入 `NaN` 几何问题。

仍未完成：

- 当前尺寸切换只改变面板列数和主要可视化尺寸，还没有做自由拖拽缩放或持久化用户偏好。

### 2026-03-08（GUI i18n 与顶部语言菜单）

- 在 `crates/app/src/gui.rs` 中为 GUI 增加了轻量 i18n 支持，当前覆盖 English / 中文 两种界面语言。
- 顶部区域已从简单标题条改成菜单栏，新增 `Language` / `语言` 菜单，可在 `English` 与 `中文` 之间即时切换。
- 当前翻译范围覆盖了：顶栏、配置区按钮与提示、设备区标题、状态文本前缀、`Realtime Metrics` 标题与尺寸按钮、以及主要指标卡片/图表标签。
- 当前没有修改日志与底层错误链的语言；GUI 里动态错误详情仍可能保留英文，因为底层 anyhow/OS 错误本身就是英文。
- 同步更新了 `README.md` 和 `docs/windows-test.md`，把顶部菜单栏和双语切换写回交接资料。

验证：

- `cargo fmt --all`
- `cargo check -p app`
- `cargo test -p app`
- `cargo build --workspace --release`

关键结果：

- GUI 现在可以在不重启进程的情况下切换中英文界面。
- 新增的语言测试确认默认语言仍是 English，同时已具备中文核心标签和 metrics 尺寸按钮翻译。

仍未完成：

- 当前语言选择还没有持久化到配置文件，重启后会回到默认中文。
- 详细错误消息仍以底层英文 error chain 为主，没有单独做中文错误映射。

### 2026-03-08（GUI 默认语言切换为中文）

- 按最新操作习惯，把 `crates/app/src/gui.rs` 中 GUI 的默认 `UiLanguage` 从 English 改成了中文。
- `NodeGuiApp::default()` 和测试辅助构造也同步切到中文默认，避免测试仍然假定英文初始态。
- 当前顶部菜单里的语言切换逻辑没有变；只是首次打开 GUI 时不再先显示英文。

验证：

- `cargo test -p app`
- `cargo build --workspace --release`

关键结果：

- 新启动的 GUI 默认就是中文界面，仍然可以随时切回 English。

### 2026-03-08（改成系统字体链，不打包字体资源）

- 按最新要求，`crates/app/src/gui.rs` 的 Windows CJK 字体逻辑已改成只在运行时读取系统字体文件，不再依赖任何仓库内或编译期打包的字体资源。
- 当前 regular 文本的优先级链是：`NotoSansSC-VF.ttf`（思源黑体）优先，其次 `msyh.ttc`（微软雅黑），再退到 `simhei.ttf` / `simsun.ttc` 等其他系统字体。
- `Start` 按钮单独切到 bold 家族，当前优先使用系统里的 `msyhbd.ttc`；如果该字体不可用，再退回其他同机可用的 CJK 字体。
- 这意味着最终生成的 `app.exe` 仍然是单文件应用，不会额外把字体资源编进包里；字体完全取决于目标 Windows 机器本身的 `C:\Windows\Fonts`。

验证：

- `cargo test -p app`
- `cargo build --workspace --release`

### 2026-03-08（内置默认配置预设与设备下拉选择）

- 为 `crates/app/src/config.rs` 增加了内置配置预设回退：`node-a.toml`、`node-b.toml`、`node-a-mock.toml`、`node-a-wasapi-wav.toml`、`node-a-mock-render.toml` 现在都会被编进二进制。即使把 `app.exe` 单独带走、外部没有仓库 `configs/` 目录，GUI 仍可直接加载这些常用预设。
- `discover_config_presets()` 现在在找不到 workspace 时会返回内置预设列表；`load_config()` 在磁盘读取失败但命中已知预设名时，会自动回退到编译进 exe 的配置文本。
- `save_config()` 现在会自动创建父目录，因此便携模式下首次保存 `configs/node-a.toml` 这类路径时，不再要求外部先手工建好 `configs/` 目录。
- 相对 `wav_path` / `dump_dir` / `logs/` 的默认落点也收敛到了“优先 workspace 根目录，否则 exe 所在目录”，避免脱离仓库后把运行产物意外写到别的当前工作目录。
- 在 `crates/app/src/gui.rs` 中把 `Audio Input Device` / `Output Target Device` 从纯文本输入改成了下拉菜单；当前会直接列出默认输入/输出以及已探测到的设备 friendly name，减少手输设备名导致的启动错误。
- `README.md`、`docs/windows-test.md`、repo skill 以及本交接文档已同步说明：当前 `app.exe` 已具备“内置默认配置预设 + 首次保存自动落盘”的便携基础，但运行日志和调试产物仍然会写到 exe 旁边或 workspace 下的外部目录。

验证：

- `cargo fmt --all`
- `cargo check -p app`
- `cargo test -p app`
- `cargo build --workspace --release`

关键结果：

- 当前 GUI 已不再强依赖仓库外部的 `configs/` 目录，单个 `app.exe` 可以直接带走并用内置预设启动。
- 设备选择入口从“文本输入 + 下方设备列表”变成了更直接的下拉选择，operator 路径更短。

仍未完成：

- 目前内置的是默认配置预设，不包含自定义用户配置；用户修改后的配置仍会以外部 TOML 文件形式落盘。
- `audio_device_probe`、`runtime_smoke` 等工具仍然是独立可执行文件，没有一起合并进单个 `app.exe`。

### 2026-03-08（配置文件夹导入、去重与冲突重命名）

- 按最新 operator 需求，`crates/app/src/gui.rs` 里的 `Load Current Path` 已移除，改成 `Import Config Folder`。点击后会直接打开 Windows 文件夹选择界面，用户只需选中一个 config 文件夹，就会批量扫描其中的 `.toml` 配置文件。
- 在 `crates/app/src/config.rs` 增加了配置导入预览和实际导入逻辑。当前会把所选文件夹中的 `.toml` 与现有本地配置和内置预设一起比较，执行两级去重：
  - 如果内容与任意已有配置完全一致，则直接跳过，不重复导入。
  - 如果文件名相同但内容不同，则先生成冲突预览并在 GUI 中警告用户。
- 用户确认导入冲突项后，系统会自动做稳定的后缀重命名，例如 `node-a.toml -> node-a-1.toml`；若原文件已经是 `node-a-1.toml`，则继续变成 `node-a-2.toml`，不会出现 `node-a-1-1.toml` 这种嵌套后缀。
- 导入成功后，GUI 会刷新 `Load Config` 预设列表，并把首个新导入的配置设为当前 `Config Path` 后重新装载；同时会在配置反馈区显示导入、去重、重命名、跳过的摘要。
- 为此新增了 `rfd` 依赖，用于在 Windows 上弹出原生文件夹选择对话框。

验证：

- `cargo check -p app`
- `cargo test -p app`
- `cargo build --workspace --release`

关键结果：

- GUI 现在已经支持“从外部 config 文件夹批量导入配置”而不是只能手写单个路径。
- 导入时的去重和冲突重命名规则已经有单元测试覆盖，包含 `name -> name-1 -> name-2` 递增，而不是 `name-1-1`。

仍未完成：

- 当前冲突确认是 GUI 内部弹窗，不会把 diff 逐字段展示给用户；它只展示来源文件名、已有路径和建议重命名结果。

### 2026-03-08（WASAPI render mix format 修正与 Recording Test tab）

- 根据最新日志 `app-4940-1772943246.log`，当前 `virtual_stub` 场景的失败点已经明确：runtime 不是卡在 capture，而是卡在 `audio_output` 初始化阶段。日志连续出现 `failed to initialize WASAPI render for '扬声器 (PRO X 2 LIGHTSPEED)' ... 参数错误 (0x80070057)`，说明目标 render endpoint 不接受旧实现强塞的自定义共享模式格式。
- 在 `crates/audio_output/src/lib.rs` 中把 Windows render 初始化从“手工构造固定的 `48 kHz / float32 / 自定义通道数` `WAVEFORMATEX`”改成了“读取设备实际 mix format 并按其格式建流”。当前同时支持常见的 `float32`、`PCM16`、`PCM24`、`PCM32` 输出写入。
- 输出写入路径也同步增强：会把内部 `48 kHz / mono / float32` 帧按设备实际采样率做轻量重采样，并按目标声道数复制 / 交织到设备 buffer 中，而不是继续把单声道 `f32` 直接生硬地 memcpy 到 render buffer。
- 为 `audio_output` 增加了 3 个单元测试，覆盖多声道复制、尾部补零和 `48 kHz -> 44.1 kHz` 的最小重采样行为。
- 同时在 `crates/app/src/gui.rs` 的中央区域增加了 `Recording Test` tab。这个 tab 专门用于录制 / 监听排障，提供：
  - 快速切换到 `node-a-wasapi-wav`、`node-a.toml`、`node-a-mock-render.toml` 的按钮
  - 当前 `wav_path` / `dump_dir` / 设备 / 地址的集中展示
  - 对“WAV 干净但实时监听有噪声”这种场景的明确诊断提示

验证：

- `cargo check -p audio_output`
- `cargo test -p audio_output`
- `cargo test -p app`
- `cargo build --workspace --release`

关键结果：

- 现在从代码和日志上都已经明确：之前的 `0x80070057` 不是用户配置错了，而是 render 端格式协商过于理想化。
- 新实现已经通过编译和测试，后续你在本机再试时，`virtual_stub` 场景不应继续被同一条 render 初始化错误挡住。

仍未完成：

- 我这里没有做人工耳机 / 扬声器监听复测，所以“代码路径已改正”不等于“主观听感已经完全验收”。
- 当前重采样仍是轻量级线性实现，优先目的是兼容设备 mix format 和消除明显噪声，不是高保真播放器级重采样。

### 2026-03-08（分析 `artifacts/windows-wasapi` 录音并修正采集格式协商）

- 直接检查了 `artifacts/windows-wasapi/` 下的 `local_raw.wav`、`output.wav`、`peer_raw.wav`、`peer_aligned.wav` 和 `metrics.tsv`。结论很明确：
  - `local_raw.wav` 与 `output.wav` 的能量和峰值几乎一致，说明异常不是后处理新增的，而是在 `local_raw` 阶段就已经存在。
  - `peer_raw` / `peer_aligned` 能量很低，`estimated_crosstalk_rms` 也几乎为 0，因此这批录音里的问题不在 sync/cancel/residual。
  - `local_raw.wav` 中能看到一段连续贴边到 `-1.0` 的平顶样本，属于真实削顶而不是随机毛刺；同时现有 `clip_events` 统计因为只算 `> 1.0`，把这些刚好打满的削顶漏掉了。
- 基于这次录音分析，把 `crates/audio_capture/src/lib.rs` 的 WASAPI 采集实现从“强行请求固定 `48 kHz / mono / float32` 共享模式格式”改成了“先读取设备实际 mix format，再自行转换到内部固定格式”：
  - 当前支持常见 `float32`、`PCM16`、`PCM24`、`PCM32` 采集格式解码
  - 对多声道输入做 downmix 到 mono
  - 当设备采样率不是 `48 kHz` 时，使用轻量线性重采样到内部固定采样率
- 同时把 `crates/app/src/runtime.rs` 里的 `clip_events` 判定改成 `>= 0.999`，让这类刚好削到满幅的异常能在实时指标里被看到，而不是继续显示 0。
- 为 `audio_capture` 新增了 2 个单元测试，覆盖 `PCM16` 立体声下的 mono downmix，以及 `float32` 单声道数据的原样解码。

验证：

- `cargo check -p audio_capture`
- `cargo test -p audio_capture`
- `cargo test -p app`
- `cargo build --workspace --release`

关键结果：

- 现在从录音文件和代码链路两边都已经明确：先前 `windows-wasapi` 录到的“电流音 / 失真”并不是后处理造成的，而是 WASAPI 采集端固定格式协商过于理想化带来的原始流失真风险。
- 新实现已改为“采集端、输出端都先跟设备实际格式协商，再转换到内部固定格式”，与此前修过的 render mix format 逻辑保持一致。

仍未完成：

- 我这里没有重新人工录一段你同样说话内容的对照样本，因此还需要你在本机用新版本再录一次，确认 `local_raw.wav` 不再出现同样的平顶削波形态。

### 2026-03-08（运行中设备切换即时生效与更强输入整形）

- 根据最新 operator 反馈，继续收紧了 `crates/app/src/gui.rs` 的设备切换行为。现在 `Audio Input Device` / `Output Target Device` 下拉框或设备列表一旦改值，GUI 会立即把新设备写回当前配置，并在 runtime 正在运行时自动请求 reload；不再要求用户额外点一次 `Save Runtime Fields` 才能让监听设备切换生效。
- 同时把 `crates/app/src/runtime.rs` 里的 `CaptureConditioner` 再加强了一档：进一步降低输入预衰减目标、收紧软限幅阈值，优先压低仍残留的一点过载感。
- 结合最新 `artifacts/node-a/local_raw.wav` 的分析，这轮变更的定位已经从“修复明显削顶”转为“继续降低残余过载感，并让用户能更快切换到非同设备监听路径做对比”。

验证：

- `cargo test -p app`
- `cargo check --workspace`

关键结果：

- 运行中切换监听设备现在已经是即时保存 + reload 路径，行为更接近系统级“换设备即生效”的预期。
- 现有 `app` 测试全部继续通过，说明新的即时切换逻辑没有破坏原有恢复/重载路径。

仍未完成：

- 目前的“即时生效”本质上仍然是 GUI worker 级 runtime 重建，不是底层 WASAPI stream 的原地无缝切换，因此切换瞬间会有一次短暂重建窗口。

### 2026-03-08（根据最新样本再压 3 档，并只补偿实时输出增益）

- 根据最新 `artifacts/node-a/local_raw.wav` 的量化结果，这批样本已经没有爆音：峰值约 `0.7527`、`clip_ratio = 0`、`metrics.tsv` 里的 `max_clip = 0`。这说明之前的削顶问题已经收住，但按最新主观反馈，还需要继续把听感再压稳一点。
- 因此把 `crates/app/src/runtime.rs` 中的 `CaptureConditioner` 又往下压了 3 档左右：进一步降低 `INPUT_PAD`、下调 `TARGET_PEAK`，并把软限幅拐点再提前，让本地输入链更保守。
- 同时在 `crates/audio_output/src/lib.rs` 中增加了“仅对实时输出 sink 生效”的补偿增益和输出软限幅。这样不会再抬高麦克风原始链路，也不会把录下来的 WAV 再次推热，但耳机实时监听不会因为前级压得更狠而一下子变得过小。

验证：

- `cargo test -p audio_output`
- `cargo test -p app`
- `cargo check --workspace`
- `cargo build --workspace --release`

关键结果：

- 当前策略已经明确分成两层：前级更保守地压住输入过载，后级只对实时输出做补偿增益。
- 这样能继续降低爆音风险，同时不把“整体放大”重新施加到麦克风录音链本身。

仍未完成：

- 这轮调整后的主观听感还需要你在本机再录一段最新样本确认；从代码和测试上看，已经是比上一轮更保守的配置。

### 2026-03-08（新增 `capture_raw.wav` 以区分采样方法与前级整形）

- 在 `crates/app/src/runtime.rs` 的调试导出链中新增了 `capture_raw.wav`。它记录的是 `CaptureSource` 刚读出来的原始帧，还没有经过 `CaptureConditioner`、VAD、消除或残差抑制。
- 现有的 `local_raw.wav` 仍然保留，但它现在明确表示“已经过本地前级整形后的近端输入”。
- 这样后续就能把 `artifacts/node-a/capture_raw.wav` 与 `artifacts/node-a/obssample.wav` 直接做 A/B，对应回答“是不是采样方法本身有问题”，而不是再把 OBS 样本和已经过我们前级整形的 `local_raw.wav` 混在一起比。

验证：

- `cargo test -p app`
- `cargo check --workspace`
- `cargo build --workspace --release`

关键结果：

- 下一轮录音开始后，仓库里的调试证据会更完整：`capture_raw.wav` 看采样原始链，`local_raw.wav` 看前级整形后结果，`output.wav` 看最终输出。

### 2026-03-08（参考 OBS 思路，移除采集端过早硬裁剪）

- 对照最新的 `artifacts/node-a/capture_raw.wav` 和用户提供的 `artifacts/node-a/obssample.wav` 继续排查后，发现一个更关键的实现差异：我们在 `crates/audio_capture/src/lib.rs` 里过早把采集样本硬裁到 `[-1, 1]`，这会把设备原本仍可恢复的浮点过载直接削成平顶。
- 这和 OBS 官方 `win-wasapi` 路径的总体思路不一致。OBS 是先按设备 `GetMixFormat()` 的原生格式建流，并把样本继续往后传，不会在采集解码这一层先做这种硬裁。
- 因此这轮在 `audio_capture` 中移除了两处过早的硬裁：
  - `decode_capture_packet_to_mono()` 不再在 downmix 之后立刻 `clamp(-1.0, 1.0)`
  - `push_capture_samples()` 在直通与重采样两条路径里也不再先做 `clamp(-1.0, 1.0)`
- 这样设备若输出的是浮点过载样本，后面的 `CaptureConditioner` 仍有机会用线性衰减把它拉回安全范围，而不是在采集层直接把波形削成平顶。

验证：

- `cargo test -p audio_capture`
- `cargo test -p app`
- `cargo check --workspace`
- `cargo build --workspace --release`

关键结果：

- 采集链现在更接近 OBS 的处理顺序：先保留原始幅度，再由后级决定如何衰减，而不是在采集层先硬剪波形。

### 2026-03-08（切换到更接近 OBS 的直通监听路径，并将仓库许可证切到 GPL）

- 根据用户最新反馈，当前排障目标不再是“录音文件有没有削顶”，而是“为什么 OBS 监听干净，而我们的实时监听仍有明显电流音”。在这种前提下，继续用自定义动态整形去硬改输入波形已经没有效率，因此这轮把监听路径进一步改成更接近 OBS 的思路：
  - `crates/app/src/runtime.rs` 的 `CaptureConditioner` 不再做持续性的动态压缩/软削，默认仅保留极端过载时的紧急线性保护
  - `crates/audio_output/src/lib.rs` 的实时监听增益也降回接近直通，只保留最终写设备前的简单裁剪保护
- 同时根据用户的明确要求，把工作区 `Cargo.toml` 的许可证元数据从 `MIT OR Apache-2.0` 切换成了 `GPL-2.0-or-later`，并把 OBS 仓库中的 `COPYING` 文件加入本仓库。
- `README.md` 已同步写明当前仓库许可证为 `GPL-2.0-or-later`。

验证：

- `cargo test -p audio_output`
- `cargo test -p app`
- `cargo check --workspace`
- `cargo build --workspace --release`

关键结果：

- 当前实时监听链已经比之前更少自定义染色，更接近 OBS “设备原始采样 -> 最小保护 -> 输出”的风格。
- 许可证元数据已经切到 GPL，后续若继续直接参考/移植 OBS 的更多实现，仓库层面不会再卡在旧的 MIT/Apache 声明上。

### 2026-03-08（将实时监听从 DSP 输出改为直接监控 `capture_raw`）

- 按最新要求继续大刀阔斧收紧：`crates/app/src/runtime.rs` 中 `virtual_stub` 场景的实时监听已不再输出 DSP 链末端的 `output_frame`，而是直接输出 `capture_raw`。
- 也就是说，当前实时监听链已经从“采集 -> sync/vad/cancel/residual -> render”改成了更接近 OBS source monitoring 的“采集 -> render”，而 DSP 主链仍然继续运行并写调试产物。
- 这次改动的目的很明确：如果用户当前主要关心的是“监听是否还有电流音”，就先把监听路径和处理路径彻底解耦，不再让监听效果被 DSP 中间环节拖累。

验证：

- `cargo test -p app`
- `cargo check --workspace`
- `cargo build --workspace --release`

关键结果：

- `virtual_stub` 的实时监听现在已经更接近 OBS 的原始 source monitoring 语义，而不是“处理后再监听”。

### 2026-03-08（恢复处理后监听，并加入反向波前馈抵消）

- 根据最新双机反馈，当前主要问题已经从“单机监听链本身有电流音”转成了“双机互连后希望直接听到处理结果，并开始做更明确的反向波抵消”。因此这轮把 `crates/app/src/runtime.rs` 中 `virtual_stub` 的默认监听源切回处理后的 `output_frame`。同时新增了 `output.monitor_processed_output` 配置，默认 `true`；如需继续排查原始监听链，仍可手动切回 `capture_raw`。
- 在 `crates/audio_cancel/src/lib.rs` 中给现有 NLMS 增加了前馈式反向波抵消层。当前会先从 `local_raw` / `peer_aligned` 估计一个平滑的直接抵消增益，再把这条前馈预测与原有 NLMS 预测叠加，输出总预测信号后做误差更新。
- 在 `crates/audio_residual/src/lib.rs` 中把原来非常轻的残余衰减升级成动态残余抑制：当本地未说话而对端活跃时，会结合 `peer_vad`、`coherence` 和 `estimated_crosstalk_rms` 对低电平残余和底噪做更强的噪声门 / 扩展处理。
- `crates/common_types/src/lib.rs` 现在新增了 `cancel.anti_phase_enabled`、`cancel.anti_phase_max_gain`、`cancel.anti_phase_smoothing` 和 `output.monitor_processed_output` 配置字段；`crates/app/src/config.rs` 也补了对应的配置校验。

验证：

- `cargo test -p audio_cancel`
- `cargo test -p audio_residual`
- `cargo test -p app`
- `cargo check --workspace`

关键结果：

- 现在双机实时监听默认会直接播放处理后输出，而不是继续听原始采集。
- 串音消除链已经不再只有 NLMS 和轻残余衰减，还多了一层明确的前馈反向波抵消与更强的动态残余抑制。

仍未完成：

- 当前反向波抵消仍是“单帧标量增益 + NLMS”混合模型，不是频域多带或更长 FIR 的完整产品级实现。
- GUI 还没有单独暴露 `monitor_processed_output` 和新的 anti-phase 参数；当前主要通过 TOML 默认值生效。

### 2026-03-08（继续加强处理后监听链的串音压制）

- 根据最新反馈“处理后监听里仍能听到对端传过来的声音”，继续加强了 `crates/app/src/runtime.rs`、`crates/audio_cancel/src/lib.rs` 和 `crates/audio_residual/src/lib.rs` 的压制链，而不是停留在第一版前馈反向波上。
- `runtime` 里的 NLMS 更新窗口不再简单要求 `peer_vad && !local_vad`。现在改成“只要对端活跃、coherence 足够高、且当前帧不是明显的 near-end dominant，就允许自适应更新”，避免远端泄漏本身把 `local_vad` 撑高后把更新一直冻死。
- `audio_residual` 现在新增了第二次反向波残余抵消：会对 `canceled` 与 `peer_aligned` 再估一个平滑的残余相关增益，先做一轮 residual anti-phase subtraction，再进入动态噪声门/扩展器。
- 这次的目的很明确：不只是“把底噪门下去”，而是继续压低仍然和对端参考高度相关的残余串音，尽量减少“另一边的声音还听得到”的情况。

验证：

- `cargo test -p audio_residual`
- `cargo test -p app`
- `cargo run -q -p offline_replay -- configs/node-a-mock.toml 180`

关键结果：

- 离线 mock 回放继续通过，`offline_replay` 本轮输出 `output_rms=0.00092`、`coherence=0.999`，说明加强后的残余链没有把 mock 场景跑坏。
- 新增的 residual 单元测试已经覆盖了三件事：本地说话不被明显削弱、静音时低电平残余会被更狠地压下去、以及第二次反向波抵消确实能进一步压低与对端参考相关的残余。

仍未完成：

- 这一轮仍然是单通道、时域、标量增益为主的增强，不是频域多带自适应或更长时窗的完整产品级 AEC。
- 还需要你在真实双机上继续听处理后监听；如果仍有明显串音残留，下一步就该把参考路径做成多段延迟 / 多带后滤波，而不是只继续调单一门限。

### 2026-03-08（更激进的默认消除参数与 GUI 降噪滑块）

- 根据最新“处理后监听里仍能明显听到对端声音”的反馈，继续把默认参数整体推得更激进：`CancelConfig::default()` 现在改成更高的 `step_size`、更低的 `update_threshold`、更深的 `anti_phase_max_gain` 和更快的 `anti_phase_smoothing`；`ResidualConfig::default()` 的 `strength` 也从原来的轻量值抬到了明显更强的默认档位。
- 对应的样例配置 `configs/node-a.toml`、`configs/node-b.toml`、`configs/node-a-mock.toml`、`configs/node-a-mock-render.toml`、`configs/node-a-wasapi-wav.toml` 也已显式写入这组更强的新默认值，保证 GUI/内置 preset/release exe 一致。
- 在 `crates/app/src/gui.rs` 左侧控制面板新增了 `Noise Reduction` 控制区，当前可直接调：
  - `Monitor processed output`
  - `Adaptation speed`
  - `Update threshold`
  - `Anti-phase depth`
  - `Anti-phase smoothing`
  - `Residual strength`
  - 以及启用/关闭 `anti-phase` 和 `residual suppressor`
- 这些控件会写回当前 TOML，并通过 `Apply Noise Controls` 或现有 `Save Runtime Fields` 生效；如果 runtime 正在运行，保存后会自动请求 reload。

验证：

- `cargo test -p app`
- `cargo check --workspace`
- `cargo run -q -p offline_replay -- configs/node-a-mock.toml 180`

关键结果：

- 本轮 `offline_replay` 的输出已经继续收紧到 `output_rms=0.00004`、`coherence=0.999`，比上一轮更低，说明更激进的默认参数在 mock 串音场景里确实进一步压低了残余输出。
- GUI 相关测试确认新参数字段会跟随配置一起 reload / save，不会只停留在界面状态里。

仍未完成：

- 还没有把这些降噪滑块做成运行中无缝热调；当前仍然依赖一次配置保存 + runtime reload。
- 真实双机监听里如果仍有明显串音，下一步就该进入多带后滤波 / 更细粒度参考延迟建模，而不是再无限往上拧单个标量参数。

### 2026-03-08（为降噪滑块补充通俗说明）

- 在 `crates/app/src/gui.rs` 的 `Noise Reduction` 控制区里，为每个开关和滑块都补了更直白的说明文案。
- 当前文案会直接告诉 operator “向左 / 向右会发生什么”，例如：
  - `Adaptation speed` 向右会更快学习对端泄漏，但过右可能让自己声音变薄
  - `Update threshold` 向左通常会更强地消除对端声音
  - `Anti-phase depth` 向右会更强抵消对端参考，但过右可能出现空洞感
  - `Residual strength` 向右会更狠地压最后的残留和底噪，但过右会发闷
- 这样后续调试时不再需要对照代码猜每个参数的方向和副作用。
- 同时新增了 GUI 交互测试，不再只验证“字段存在”或“配置能保存”。当前测试会实际渲染 `Noise Reduction` 区，模拟点击复选框和拖动滑块，并断言 `monitor_processed_output`、`update_threshold`、`anti_phase_depth`、`residual_strength` 等值确实通过 GUI 发生变化。

验证：

- `cargo test -p app`
- `cargo build --workspace --release`

### 2026-03-08（将传输丢失改成 2400 帧滑动窗口）

- 根据最新反馈，`transport_loss_rate` 之前使用的是累计统计口径，不是最近窗口，因此在长时间运行后会变得不够敏感。
- 现在 `crates/common_types/src/lib.rs`、`crates/audio_transport/src/lib.rs` 和 `crates/app/src/runtime.rs` 已改成按最近 `2400` 个传输帧计算 `transport_loss_rate`。
- GUI 的 metrics 历史窗口仍然保持 `240` 个 runtime snapshot，不和 transport loss 的统计窗口混在一起；`crates/app/src/gui.rs` 的传输面板也新增了文字说明，明确“丢失率按最近 2400 帧计算，图表仍只画最近 240 个快照”。

验证：

- `cargo test -p audio_transport`
- `cargo test -p app`
- `cargo test --workspace`

### 2026-03-08（切换到 master/slave/peer 预设与严格配对模式）

- 配置模型已从旧的 `node-a/node-b` 入口扩展成显式模式：
  - `node.session_mode = master_slave / peer / both`
  - `node.role = master / slave / peer`
- 当前用户入口预设已切成三个文件：
  - `configs/master.toml`
  - `configs/slave.toml`
  - `configs/peer.toml`
  其中 `peer.toml` 默认是 `peer/peer`；如需 `both/both`，把 `session_mode` 改成 `both` 即可。
- `audio_transport` 现在会把本端 `session_mode + role` 编码进 UDP 包头，并在接收端严格校验对端身份：
  - `master` 只能接 `slave`
  - `slave` 只能接 `master`
  - `peer + peer` 只能接 `peer + peer`
  - `peer + both` 只能接 `peer + both`
  错误配对不再静默运行，而会直接报不兼容错误。
- `runtime` 现在同时产出两路处理后流：
  - `local_processed`
  - `peer_processed`
  这让新的输出路由可以工作。
- 输出路由已扩展为：
  - `local_only`
  - `off`
  - `mix_to_primary`
  - `split_local_peer`
- 路由限制已纳入配置校验：
  - `master` 只允许 `mix_to_primary` / `split_local_peer`
  - `slave` 只允许 `local_only` / `off`
  - `peer/peer` 只允许 `local_only`
  - `both/both` 只允许 `mix_to_primary` / `split_local_peer`
- GUI 左侧现在也能直接编辑 `Session Mode`、`Role`、`Output Routing`、`Primary Output Device` 和 `Secondary Output Device`，并跟随现有保存 / reload 流程一起落盘。

验证：

- `cargo test -p audio_transport`
- `cargo test -p app`
- `cargo test --workspace`
- `cargo build --workspace --release`

关键结果：

- 新的三预设和模式/角色/路由字段已经贯通到配置、transport、runtime 和 GUI。
- 现在 master/slave、peer/peer、both/both 的限制不是只靠文档说明，而是真正在运行时被强校验。

### 2026-03-08（Realtime Metrics 改成极简 / 默认 / 大 / 我瞎了四档）

- 根据最新 GUI 反馈，把 `crates/app/src/gui.rs` 中原来的 `Metrics Size` 三档切换改成了四档：
  - `极简`
  - `默认`
  - `大`
  - `我瞎了`
- 标题旁的静态提示文案也从较重的 `指标尺寸` 收成了淡色的 `尺寸`，只作为说明文本，不再显得像可点击控件。
- 档位映射现在是：
  - `默认` = 之前的小尺寸四列布局
  - `大` = 之前的中尺寸两列布局
  - `我瞎了` = 之前的大尺寸单列布局
  - `极简` = 新增纯文字摘要模式，不显示折线图和进度条
- `draw_metrics_dashboard` 现在会在 `极简` 档切到专门的文字摘要渲染路径，只保留概览、音频电平、同步/VAD、传输/耗时四块文本数据，避免继续用可视化图表占空间。
- 对应的 GUI 测试也已经补上，确认：
  - 默认启动仍是原来的四列小尺寸布局
  - 中文标签已更新为 `极简 / 默认 / 大 / 我瞎了`
  - `极简` 档可以正常渲染

验证：

- `cargo test -p app`
- `cargo build --workspace --release`

### 2026-03-08（配置文件下拉自动装载与内嵌思源黑体）

- 按最新 GUI 反馈，把左侧原来的 `Config Path` 文本框和 `Load Config` / `刷新配置` 按钮收掉了，改成单一的 `配置文件` 下拉选择。
- 现在在 `crates/app/src/gui.rs` 中，用户一旦切换预设项，就会立刻调用现有的 `load_selected_config_path()` 路径自动装载配置；不再需要额外再点一次“加载配置”。
- `Import Config Folder` 仍然保留，用于把外部 config 文件夹批量导入到当前预设列表；导入完成后列表会自动刷新，并把首个新导入项装载到表单里。
- 字体策略也按最新要求改回“内嵌”而不是“读系统字体”：`crates/app/src/gui.rs` 现在直接通过 `include_bytes!` 把 `assets/fonts/SourceHanSansCN-Regular.otf` 和 `assets/fonts/SourceHanSansCN-Bold.otf` 编进二进制。
- 默认界面文本统一走思源黑体 regular，`Start` 按钮继续单独走 bold；这样 `app.exe` 不再依赖目标机器的 `C:\Windows\Fonts` 是否刚好装有对应字体。

验证：

- `cargo test -p app`
- `cargo build --workspace --release`
