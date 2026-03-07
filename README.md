# EK Dual Mic

按 [`Dual-Mic-Crosstalk-Canceller-README.md`](./Dual-Mic-Crosstalk-Canceller-README.md) 搭建的 Rust workspace 骨架，当前阶段聚焦：

- Windows only
- Rust `nightly-2025-07-12`
- `eframe/egui` 原生桌面 GUI
- 双机协同串音消除的模块化工程框架

当前仓库已经包含：

- 可扩展的 workspace / crate 切分
- TOML 配置结构
- 实时处理主循环骨架
- UDP / mock 传输接口
- mock 离线双讲场景，可验证 sync / VAD / NLMS 冻结与更新行为
- 同步、VAD、NLMS 消除、残差抑制的第一版占位实现
- `offline_replay` 与 `wav_dump` 工具入口
- Windows GUI 壳

当前仍是框架阶段，以下 Windows 专项实现已预留接口但未完成：

- WASAPI 真实输入采集
- 系统级虚拟麦输出
- 更完整的延迟/漂移跟踪

接手/交接文档：

- `docs/completed-work.md`: 当前已完成内容与持续交接记录
- `docs/windows-test.md`: Windows 端 Codex 的测试流程与预期结果
- `configs/node-a-mock.toml`: 当前推荐的 mock GUI / Windows 验证配置

常用命令：

```bash
cargo check --workspace
cargo run -p offline_replay -- configs/node-a.toml 600
cargo run -p wav_dump -- artifacts/offline/output.wav
```

当 `audio.backend = "mock"` 且 `transport_backend = "mock"` 时，
`offline_replay` 会使用内置的双人交替说话仿真场景，而不是简单本地回环。

Windows GUI 入口：

```bash
cargo run -p app --release
```
