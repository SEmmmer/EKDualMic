# Windows Test Guide

这份文档给接手 Windows 端验证的 Codex 使用。当前仓库仍处于“框架 + 可验证 mock 链路”阶段，但已经具备第一版真实 WASAPI 采集能力和输出桥接能力，所以测试必须区分：

- 可完成的测试：编译、GUI 外壳、mock 离线验证、mock GUI 运行、真实 WASAPI 采集 + WAV 落盘、写入现有 render endpoint 的输出桥接
- 尚未完成的测试：仓库内自带的系统级虚拟麦设备创建、真实双机端到端联调

## Read First

测试前先读：

1. `README.md`
2. `Dual-Mic-Crosstalk-Canceller-README.md`
3. `docs/completed-work.md`
4. `docs/config.md`
5. 如果要看当前架构边界，再读 `docs/architecture.md`

## Current Reality

- Windows GUI 可以启动
- 默认 `configs/node-a.toml` / `configs/node-b.toml` 仍指向 `wasapi` + `virtual_stub`
- `audio_capture` 的 `wasapi` 后端已经实现固定 `48 kHz` / `mono` / `float32` / `10 ms` 的 shared-mode 采集 MVP
- 当前推荐的真实采集验证配置是 `configs/node-a-wasapi-wav.toml`
- `audio_output` 的 `virtual_stub` 在 Windows 上已不再是空实现；它现在会把处理后音频写到现有 render endpoint
- 如果把 `target_device` 指向外部虚拟声卡的输入端点，例如 `CABLE Input`，处理后音频可以被桥接到那条现有虚拟麦链路
- 仓库仍然不会自己创建新的系统级 capture endpoint，所以“完全内建的虚拟麦设备”仍未完成
- 当前推荐的输出桥接验证配置是 `configs/node-a-mock-render.toml`
- GUI worker 现在会在配置加载失败、runtime 初始化失败或运行中音频 I/O 失败后进入 `Recovering: ...` 状态，并定期重试
- 运行中的 GUI 现在支持 `Reload Runtime`；如果先点选设备再点 `Save Device Fields`，会在保存 TOML 后自动请求 reload，无需关闭应用
- 未修改的 `configs/node-a.toml` / `configs/node-b.toml` 仍包含占位输入设备名；如果本机不存在该名字，启动时报“找不到配置的 capture device”属于正常现象，不应误记成回归

## Environment Checklist

- Windows 10 或 Windows 11
- Rust toolchain 与仓库要求一致
- 可运行 `cargo`
- 如需观察 GUI，桌面会话必须可用
- 如需做未来阶段的真实音频测试，再准备物理麦和虚拟声卡；当前阶段不是硬要求

## Test Matrix

### 1. Build Sanity

执行：

```bash
cargo check --workspace
cargo test --workspace
```

期望：

- 全部通过
- `app` 中应至少包含 mock 场景收敛测试并通过
- `runtime_smoke` 工具可正常编译

### 2. Device Probe

执行：

```bash
cargo run -q -p audio_device_probe
```

期望：

- 能列出当前激活的 `Capture Devices`
- 能列出当前激活的 `Render Devices`
- 默认设备应带 `[default]` 标记

用途：

- 把输出中的 capture friendly name 填到 `input_device`
- 把输出中的 render friendly name 填到 `target_device`
- 在接外部虚拟声卡时，先用这个命令确认例如 `CABLE Input` 这样的名字是否与配置一致

### 3. Offline Mock Replay

执行：

```bash
cargo run -q -p offline_replay -- configs/node-a.toml 180
cargo run -q -p wav_dump -- artifacts/offline/processed-output.wav
```

期望：

- 生成 `artifacts/offline/metrics.tsv`
- 生成 `artifacts/offline/local_raw.wav`
- 生成 `artifacts/offline/peer_raw.wav`
- 生成 `artifacts/offline/peer_aligned.wav`
- 生成 `artifacts/offline/output.wav`
- 生成 `artifacts/offline/processed-output.wav`

应重点检查：

- `metrics.tsv` 中后段能看到 `coarse_delay_ms` 接近配置的 `mock_peer_delay_ms`
- `metrics.tsv` 中至少有一段 `frozen=false`
- 对端单讲窗口里，`output_rms` 明显小于 `input_rms`
- `wav_dump` 能正常读出 `48 kHz`、`float32`

### 4. Headless WASAPI Capture Smoke Test

使用 `configs/node-a-wasapi-wav.toml`。

执行：

```bash
cargo run -q -p runtime_smoke -- configs/node-a-wasapi-wav.toml 100
cargo run -q -p wav_dump -- artifacts/windows-wasapi/processed-output.wav
```

期望：

- 不应再出现 “WASAPI capture backend is reserved/not implemented”
- 能生成 `artifacts/windows-wasapi/metrics.tsv`
- 能生成 `artifacts/windows-wasapi/local_raw.wav`
- 能生成 `artifacts/windows-wasapi/peer_raw.wav`
- 能生成 `artifacts/windows-wasapi/peer_aligned.wav`
- 能生成 `artifacts/windows-wasapi/output.wav`
- 能生成 `artifacts/windows-wasapi/processed-output.wav`
- `wav_dump` 能正常读出 `48 kHz`、`float32`

应重点检查：

- `runtime_smoke` 启动日志里 `capture=` 应显示真实输入设备名，而不是 mock 名称
- 在没有真实对端节点时，`metrics.tsv` 的 `loss_rate` 接近 `1.0` 是正常现象，因为参考流主要来自 concealment
- 在没有真实对端节点时，runtime 仍应保持运行；不应因为 `UDP receive failed` 直接掉进持续 `Recovering`
- `input_rms` 应反映麦克风底噪或说话输入，不应恒为 NaN/异常值

### 5. Headless Output Bridge Smoke Test

使用 `configs/node-a-mock-render.toml`。

执行：

```bash
cargo run -q -p runtime_smoke -- configs/node-a-mock-render.toml 5
```

期望：

- 不应出现 render device 未实现或 `virtual_stub` 空写入相关错误
- runtime 可以完成至少几帧 mock 输入 + render endpoint 输出
- `artifacts/windows-render/` 下应生成 `metrics.tsv`、`local_raw.wav`、`peer_raw.wav`、`peer_aligned.wav`、`output.wav`

应重点检查：

- 因为 `target_device = "default"`，输出会写到当前默认 render endpoint；建议保持系统音量较低
- 这个测试验证的是“输出桥接代码可工作”，不是“仓库已经自带虚拟麦设备”

### 6. GUI Shell Smoke Test

执行：

```bash
cargo run -p app --release
```

期望：

- GUI 窗口可以打开
- 可编辑配置路径
- 左侧应能显示 `Capture Devices` / `Render Devices` 列表，并标记默认设备
- 中文设备名和中文文案不应再显示为方块/乱码
- 左侧应能显示 `Audio Input Device` / `Output Target Device` 可编辑字段
- `Load Config` 应为下拉菜单，并列出 `configs/` 下已发现的 TOML 预设
- 应可通过 `Load Config` 或 `Load Current Path` 读出当前配置中的设备字段，并通过 `Save Device Fields` 写回 TOML
- 点击 `Load Config` 后，状态栏和配置反馈应明确显示已从哪个路径装入配置
- 即使未先点 `Save Device Fields`，点击 `Start` 也应先把当前界面中的设备字段同步进配置，再启动 runtime
- 左侧控制面板和 `Realtime Metrics` 面板都应支持鼠标滚轮滚动
- 当加载的是 `audio=mock` 或 `output=wav_dump/null` 的配置时，GUI 应明确提示哪些设备字段在该模式下会被忽略
- runtime 运行中应可看到 `Reload Runtime` 按钮
- `Realtime Metrics` 不应再只是纯文本表格；应能看到状态卡、历史折线图和关键指标进度条
- 指标面板与开始/停止按钮可见
- GUI 启动后，仓库根目录 `logs/` 下应出现新的 `app-<pid>-<timestamp>.log`
- GUI 中出现的关键状态变化，例如 `Config load failed`、`Recovering: ...`、`Reload requested`，也应同步写入最新日志文件

### 7. GUI Mock Runtime Test

使用 `configs/node-a-mock.toml`。

步骤：

1. 启动 GUI：`cargo run -p app --release`
2. 把配置路径改成 `configs/node-a-mock.toml`
3. 点击 `Load Config`
4. 点击 `Start`
5. 观察状态是否进入 `Running: frame ...`
6. 观察指标是否持续刷新
7. 点击 `Stop`

期望：

- 不应出现 WASAPI 未实现错误
- `Sequence` 持续增加
- `Coarse Delay` 稳定在接近 `20 ms`
- `Transport Loss` 在 mock 场景下应接近 `0`
- `Estimated Crosstalk RMS` 在对端单讲阶段应大于 `0`
- 停止后应落盘到 `artifacts/windows-mock/`

### 8. GUI WASAPI Capture Smoke Test

使用 `configs/node-a-wasapi-wav.toml`。

步骤：

1. 启动 GUI
2. 把配置路径改成 `configs/node-a-wasapi-wav.toml`
3. 点击 `Load Config`
4. 点击 `Start`
5. 观察状态是否进入 `Running: frame ...`
6. 观察指标是否持续刷新
7. 点击 `Stop`

期望：

- 不应出现 WASAPI 未实现错误
- `Sequence` 持续增加
- `Input RMS` 随底噪/说话发生变化
- `Transport Loss` 在没有真实对端时接近 `100%` 属于正常现象
- 在没有真实对端时，不应因为 UDP 接收错误反复重建 runtime
- 停止后应落盘到 `artifacts/windows-wasapi/`

### 9. GUI Runtime Reload And Recovery Test

可使用 `configs/node-a-mock.toml` 做无风险验证，也可在真实设备场景下验证。

步骤：

1. 启动 GUI
2. 把配置路径改成一个不存在的 TOML，例如 `configs/does-not-exist.toml`
3. 点击 `Start`
4. 观察状态进入 `Recovering: attempt ... failed to load config ...`
5. 把配置路径改回有效配置，例如 `configs/node-a-mock.toml`
6. 点击 `Reload Runtime`
7. 观察状态在不关闭 GUI 的情况下恢复到 `Running: frame ...`

期望：

- worker 不应因为首次失败直接退出
- `Reload Runtime` 后应能用新的配置路径重建 runtime
- 恢复成功后指标继续刷新
- `Load Config` 点击后不应表现为“无反应”；至少应更新状态文字或配置反馈
- 即使 GUI 进程的当前工作目录不在仓库根目录，`configs/node-a.toml` 这类相对路径也应能正确读到仓库中的配置
- 如果 GUI 闪退或 panic，优先检查仓库根目录 `logs/` 下最近生成的日志文件

如果要验证运行中设备热切换：

1. 先用一个可运行的配置启动 GUI
2. 在左侧设备列表中点击新的 capture 或 render 设备
3. 点击 `Save Device Fields`
4. 观察状态短暂进入 `Recovering: reloading runtime ...`，随后恢复到 `Running: frame ...`

期望：

- 不需要关闭整个 GUI 进程
- `Save Device Fields` 在 runtime 运行中应自动触发 reload
- 若设备暂时不可用，worker 应持续停留在 `Recovering: ...` 并重试，而不是只报一次错后退出

### 10. Default Config Placeholder Validation

如果需要确认默认双机配置仍然需要人工填写真实设备名，可用未改动的 `configs/node-a.toml` 做一次显式验证：

1. 启动 GUI
2. 保持 `configs/node-a.toml`
3. 点击 `Load Config`
4. 点击 `Start`

期望之一：

- 如果本机没有名为 `Microphone (Headset A)` 的设备，状态进入恢复态
- 报错信息应表明“找不到配置的 capture device”
- GUI 不应直接退出，而应持续显示 `Recovering: ...`，直到你修正配置或点击 `Stop`

期望之二：

- 如果你先把 `input_device` 改成真实设备名或 `"default"`，runtime 可以启动
- 但因为输出仍是 `virtual_stub`，这不代表系统级虚拟麦已经完成

## Evidence To Save

完成测试后，建议在交接说明或提交说明里记录：

- 执行过的命令
- 使用的配置文件
- 是否是 mock 测试、WASAPI headless 冒烟、GUI 冒烟，还是默认配置占位验证
- 关键指标摘要
- 生成的 artifact 路径
- 与预期不符的地方

## When To Update This File

以下情况必须更新本文件：

- 新增或修改了 Windows 测试入口
- WASAPI 采集开始可用
- 虚拟麦输出开始可用
- GUI 的操作路径、默认配置、产物路径或验收标准发生变化
- 运行中 reload / 断开恢复行为发生变化
