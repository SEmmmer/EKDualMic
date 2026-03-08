# EK Dual Mic

按 [`Dual-Mic-Crosstalk-Canceller-README.md`](./Dual-Mic-Crosstalk-Canceller-README.md) 搭建的 Rust workspace 骨架，当前阶段聚焦：

- Windows only
- Rust `nightly-2025-07-12`
- `eframe/egui` 原生桌面 GUI
- 双机协同串音消除的模块化工程框架

许可证：当前仓库已切换为 `GPL-2.0-or-later`，详见 [COPYING](C:/Users/emmmer.SUPERXLB/git/EKDualMic/COPYING)。

当前仓库已经包含：

- 可扩展的 workspace / crate 切分
- TOML 配置结构
- 默认配置预设已内置进 `app.exe`，即使脱离仓库目录也能直接加载 `configs/node-a.toml` / `node-b.toml` / `node-a-mock.toml` 等常用入口
- 实时处理主循环骨架
- WASAPI 共享模式真实输入采集 MVP（固定 `48 kHz` / `mono` / `float32` / `10 ms`）
- WASAPI 采集现在会按设备实际 mix format 读取，再转换成内部固定的 `48 kHz / mono / float32 / 10 ms`
- WASAPI 输出桥接 MVP，可把处理后音频写到现有 Windows render endpoint
- WASAPI 输出桥接现在会按设备实际 mix format 建流，并在需要时把 `48 kHz` 单声道处理流扩展/重采样到设备输出格式，减少监听噪声和 render 初始化失败
- `virtual_stub` 实时监听现在默认监听处理后的 `output_frame`，用于直接验收双机串音消除结果；如需排查原始监听链，可把 `output.monitor_processed_output = false` 切回 `capture_raw`
- UDP / mock 传输接口
- mock 离线双讲场景，可验证 sync / VAD / NLMS 冻结与更新行为
- 同步、VAD、NLMS 消除、反向波前馈抵消与动态残余抑制的第一版实现
- `audio_device_probe`、`offline_replay`、`runtime_smoke` 与 `wav_dump` 工具入口
- Windows GUI 壳，支持查看设备列表、把 `input_device` / `target_device` 保存回配置文件，并在运行中请求 runtime reload / 自动重试恢复
- GUI 现在也支持直接编辑并保存 `listen_addr` / `peer_addr`，可在两台机器之间按 IP 互连
- `Audio Input Device` / `Output Target Device` 已改成下拉选择，默认输入/输出和已探测设备都可直接选中
- 运行中切换输入/输出设备时，GUI 现在会立即保存并请求 runtime reload，不再要求额外点一次 `Save Runtime Fields`
- GUI 启动时会按系统字体优先级加载 Windows CJK 字体链：优先思源黑体，其次微软雅黑，再到其他系统字体；字体在运行时直接从 `C:\Windows\Fonts` 读取，不会打包进 `app.exe`
- GUI 顶部现在有菜单栏，可在 English / 中文 两种界面语言之间切换；当前默认语言为中文
- GUI 中央区域现在分成 `Metrics` 和 `Recording Test` 两个 tab，录制/监听排障可以在 `Recording Test` 中单独完成
- `Realtime Metrics` 已从纯文本数值升级为可视化面板，包含实时状态卡、历史折线图和关键指标进度条；当前默认 `Small` 布局会把 4 个指标面板排成一行
- GUI 左侧现在新增 `Noise Reduction` 控制区，可直接调 `Adaptation speed`、`Update threshold`、`Anti-phase depth`、`Anti-phase smoothing`、`Residual strength`，并切换是否监听处理后输出
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
`Load Config` 现在是下拉菜单，会列出 `configs/` 下已发现的 TOML 预设；也可以继续手填 `Config Path`，再用 `Import Config Folder` 打开 Windows 文件夹选择界面批量导入整个 config 文件夹。
GUI 对 `configs/node-a.toml` 这类相对路径会优先按仓库根目录解析；如果脱离仓库单独携带 `app.exe`，则会回退到 exe 所在目录，并直接使用内置配置预设。
`Load Config` 会把当前 TOML 的设备字段和网络字段重新装入界面，并在 runtime 运行中自动请求 reload。
`Save Runtime Fields` 会把设备名和 `listen_addr` / `peer_addr` 一起写回当前 TOML；如果 runtime 正在运行，GUI 也会自动请求 reload。
即使你没有先点保存，`Start` 现在也会先把界面里当前选中的设备字段和网络字段同步进配置，再启动 runtime。
如果当前目录下还没有对应的 `configs/*.toml`，首次保存时会自动创建目录并把当前配置落盘。
通过 `Import Config Folder` 导入时，完全相同内容的配置不会重复导入；如果出现同名但内容不同的配置，GUI 会先警告，确认后自动按 `name-1.toml`、`name-2.toml` 这样的规则重命名，而不会生成 `name-1-1.toml`。
如果当前加载的是 `audio=mock` 的配置，GUI 会明确提示“真实麦克风输入会被忽略”，避免把 mock 场景误当成真实采集。
如果当前加载的是 `transport=mock` 的配置，GUI 也会明确提示 `listen_addr` / `peer_addr` 会被忽略。

双机局域网互连时，推荐这样填写：

- 两台机器都把 `listen_addr` 设成 `0.0.0.0:38001`
- A 机器把 `peer_addr` 设成 `B机器IP:38001`
- B 机器把 `peer_addr` 设成 `A机器IP:38001`
- GUI 左侧现在可直接编辑这两个字段，不必手改 TOML

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
