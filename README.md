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
- WASAPI 共享模式真实输入采集 MVP（固定 `48 kHz` / `mono` / `float32` / `10 ms`）
- WASAPI 输出桥接 MVP，可把处理后音频写到现有 Windows render endpoint
- UDP / mock 传输接口
- mock 离线双讲场景，可验证 sync / VAD / NLMS 冻结与更新行为
- 同步、VAD、NLMS 消除、残差抑制的第一版占位实现
- `audio_device_probe`、`offline_replay`、`runtime_smoke` 与 `wav_dump` 工具入口
- Windows GUI 壳，支持查看设备列表、把 `input_device` / `target_device` 保存回配置文件，并在运行中请求 runtime reload / 自动重试恢复
- GUI 启动时会加载 Windows CJK 字体 fallback，中文设备名与中文文案可正常显示
- `Realtime Metrics` 已从纯文本数值升级为可视化面板，包含实时状态卡、历史折线图和关键指标进度条
- GUI 左侧控制面板和 `Realtime Metrics` 面板现在都支持滚轮滚动，长页面不会再被一屏截断
- `app` 现在会把日志写到仓库根目录 `logs/`，每次启动生成独立日志文件，并在 panic 时追加 panic/backtrace 方便排查闪退

当前仍是框架阶段，以下 Windows 专项实现已预留接口但未完成：

- 仓库内自带的系统级虚拟麦设备创建能力
- 更完整的延迟/漂移跟踪
- 更细粒度的设备热切换 / 原地恢复策略；当前仅提供 GUI worker 级 runtime 重建恢复

接手/交接文档：

- `docs/completed-work.md`: 当前已完成内容与持续交接记录
- `docs/windows-test.md`: Windows 端 Codex 的测试流程与预期结果
- `configs/node-a-mock.toml`: 当前推荐的 mock GUI / Windows 验证配置
- `configs/node-a-wasapi-wav.toml`: 当前推荐的真实 WASAPI 采集 + WAV 落盘验证配置
- `configs/node-a-mock-render.toml`: 当前推荐的 mock + render endpoint 输出桥接验证配置
- `skills/ek-dual-mic-maintainer/`: 供后续 Codex 复用的仓库内 skill 源文件

常用命令：

```bash
cargo check --workspace
cargo run -p audio_device_probe
cargo run -p offline_replay -- configs/node-a.toml 600
cargo run -p runtime_smoke -- configs/node-a-wasapi-wav.toml 100
cargo run -p runtime_smoke -- configs/node-a-mock-render.toml 20
cargo run -p wav_dump -- artifacts/offline/output.wav
```

当需要先确认 Windows 机器上的 capture / render endpoint 名称时，
先执行 `cargo run -p audio_device_probe`，再把输出中的 friendly name 填进
`input_device` / `target_device`。

如果更习惯在 GUI 里操作，也可以直接启动 GUI，使用左侧的设备列表点选目标设备。
`Load Config` 现在是下拉菜单，会列出 `configs/` 下已发现的 TOML 预设；也可以继续手填 `Config Path`，再用 `Load Current Path` 按当前路径装载。
GUI 对 `configs/node-a.toml` 这类相对路径会自动按仓库根目录解析，因此从 `target/debug/app.exe` 或 `target/release/app.exe` 启动时也能正确读到仓库里的配置。
`Load Config` / `Load Current Path` 会把当前 TOML 的设备字段重新装入界面，并在 runtime 运行中自动请求 reload。
`Save Device Fields` 会把设备名写回当前 TOML；如果 runtime 正在运行，GUI 也会自动请求 reload。
即使你没有先点保存，`Start` 现在也会先把界面里当前选中的设备字段同步进配置，再启动 runtime。
如果当前加载的是 `audio=mock` 的配置，GUI 会明确提示“真实麦克风输入会被忽略”，避免把 mock 场景误当成真实采集。

当 `audio.backend = "mock"` 且 `transport_backend = "mock"` 时，
`offline_replay` 会使用内置的双人交替说话仿真场景，而不是简单本地回环。

当需要验证真实 Windows 采集但暂时不接虚拟麦时，
可使用 `configs/node-a-wasapi-wav.toml`，它会读取默认输入设备并把运行结果落盘到
`artifacts/windows-wasapi/`。即使没有真实对端节点，当前实现也应继续运行并走
UDP concealment，不应因为单机无 peer 而直接进入持续 `Recovering`。

当需要验证输出桥接链路时，
可使用 `configs/node-a-mock-render.toml`。它会用 mock 输入驱动 runtime，
并把处理后音频写到当前默认 render endpoint，避免真实麦克风回灌。

Windows GUI 入口：

```bash
cargo run -p app --release
```
