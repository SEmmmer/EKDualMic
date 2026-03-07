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

### Tooling And GUI

- `offline_replay` 能用配置启动离线处理并导出调试文件
- `wav_dump` 能统计 WAV 的采样格式、时长、峰值和 RMS
- Windows GUI 已有 `eframe/egui` 外壳，可启动、加载配置、驱动 runtime、显示实时指标

### Mock Validation Path

- 当 `audio.backend = "mock"` 且 `node.transport_backend = "mock"` 时，runtime 不再使用简单本地回环
- 当前会启用内置双人交替说话场景：
  - `peer_raw` 为对端原始参考
  - `local_raw` 为“本机近端 + 延迟后的对端串音 + 极低底噪”的混合
  - 可真实触发 sync 对齐、VAD 冻结和 NLMS 收敛
- 已提供可直接用于 GUI / Windows 验证的 `configs/node-a-mock.toml`

## Known Gaps

- `audio_capture` 的 `wasapi` 后端仍是预留接口，尚未完成真实采集
- `audio_output` 的虚拟麦输出仍是 `virtual_stub`
- `audio_sync` 还没有细粒度漂移补偿，也没有更稳的延迟跟踪
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

验证：

- `cargo check --workspace`
- `cargo test --workspace`
- `cargo run -q -p offline_replay -- configs/node-a.toml 180`
- `cargo run -q -p wav_dump -- artifacts/offline/processed-output.wav`
- `python3 /home/emmmer/.codex/skills/.system/skill-creator/scripts/quick_validate.py /home/emmmer/.codex/skills/ek-dual-mic-maintainer`
- `cargo run -q -p offline_replay -- configs/node-a-mock.toml 10`

关键结果：

- `offline_replay` 180 帧运行成功，最终 `coherence=0.999`
- `artifacts/offline/metrics.tsv` 中后段可见 `coarse_delay_ms=20.000`
- `artifacts/offline/metrics.tsv` 中帧 `174-180` 的 `input_rms` 约 `0.031`，`output_rms` 已降至约 `0.0013-0.0039`

仍未完成：

- WASAPI 真实采集
- 系统级虚拟麦输出
- 漂移补偿与更稳的同步策略
