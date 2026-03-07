# Windows Test Guide

这份文档给接手 Windows 端验证的 Codex 使用。当前仓库仍处于“框架 + 可验证 mock 链路”阶段，所以测试必须区分：

- 可完成的测试：编译、GUI 外壳、mock 离线验证、mock GUI 运行
- 预期失败或尚未完成的测试：真实 WASAPI 采集、系统级虚拟麦输出

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
- 当前点击 GUI 的 `Start` 后，若使用默认配置，预期会报 “WASAPI capture backend is reserved/not implemented”
- 这在当前阶段是正常现象，不应误记成回归

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

### 2. Offline Mock Replay

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

### 3. GUI Shell Smoke Test

执行：

```bash
cargo run -p app --release
```

期望：

- GUI 窗口可以打开
- 可编辑配置路径
- 指标面板与开始/停止按钮可见

### 4. GUI Mock Runtime Test

使用 `configs/node-a-mock.toml`。

步骤：

1. 启动 GUI：`cargo run -p app --release`
2. 把配置路径改成 `configs/node-a-mock.toml`
3. 点击 `Start`
4. 观察状态是否进入 `Running: frame ...`
5. 观察指标是否持续刷新
6. 点击 `Stop`

期望：

- 不应出现 WASAPI 未实现错误
- `Sequence` 持续增加
- `Coarse Delay` 稳定在接近 `20 ms`
- `Transport Loss` 在 mock 场景下应接近 `0`
- `Estimated Crosstalk RMS` 在对端单讲阶段应大于 `0`
- 停止后应落盘到 `artifacts/windows-mock/`

### 5. Expected Failure Test With Default Config

如果需要确认“当前真实 Windows I/O 尚未实现”的状态没有被误改，可用默认配置做一次显式验证：

1. 启动 GUI
2. 保持 `configs/node-a.toml`
3. 点击 `Start`

期望：

- 状态进入错误态
- 报错信息应表明 `wasapi` 采集尚未实现

## Evidence To Save

完成测试后，建议在交接说明或提交说明里记录：

- 执行过的命令
- 使用的配置文件
- 是否是 mock 测试还是默认配置失败验证
- 关键指标摘要
- 生成的 artifact 路径
- 与预期不符的地方

## When To Update This File

以下情况必须更新本文件：

- 新增或修改了 Windows 测试入口
- WASAPI 采集开始可用
- 虚拟麦输出开始可用
- GUI 的操作路径、默认配置、产物路径或验收标准发生变化
