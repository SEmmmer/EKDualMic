# Config

配置使用 TOML，核心字段与设计文档一致，并增加了框架阶段需要的 backend 选择项。

## `[node]`

- `name`: 节点名
- `listen_addr`: 本地监听地址；当 `transport_backend = "udp"` 时，当前要求写成显式 `IP:port`
- `peer_addr`: 对端地址；当 `transport_backend = "udp"` 时，当前要求写成显式 `IP:port`
- `transport_backend`: `udp` / `mock`

双机局域网推荐写法：

- A/B 两边都监听 `0.0.0.0:38001`
- A 侧把 `peer_addr` 填成 `B机器IP:38001`
- B 侧把 `peer_addr` 填成 `A机器IP:38001`
- 如果在 GUI 中只输入纯 IP，例如 `192.168.1.22`，当前实现会自动沿用该配置原有的端口

## `[audio]`

- `backend`: `wasapi` / `mock`
- `input_device`: 输入设备名；当 `backend = "wasapi"` 时，支持写具体 friendly name，也支持写 `"default"` 以选择当前默认输入设备
- `sample_rate`: 当前固定为 `48000`
- `channels`: 当前固定为 `1`
- `frame_ms`: 当前固定为 `10`

## `[output]`

- `backend`: `virtual_stub` / `wav_dump` / `null`
- `target_device`: 当 `backend = "virtual_stub"` 且运行在 Windows 上时，支持写 render endpoint 的 friendly name，也支持写 `"default"` 选择当前默认输出设备；如果目标是外部虚拟声卡的输入端点（例如 VB-Cable 的 `CABLE Input`），处理后音频会被桥接过去
- `monitor_processed_output`: 当 `backend = "virtual_stub"` 时，`true` 表示实时监听处理后的 `output_frame`，`false` 表示监听原始 `capture_raw`；当前默认 `true`
- `wav_path`: 当 `backend = "wav_dump"` 时使用；`virtual_stub` 不会写这个路径

## `[sync]`

- `jitter_buffer_frames`
- `coarse_search_ms`
- `drift_compensation`

## `[cancel]`

- `filter_length`
- `step_size`
- `leakage`
- `update_threshold`
- `anti_phase_enabled`: 是否启用基于对端对齐帧的前馈反向波抵消
- `anti_phase_max_gain`: 反向波前馈抵消的最大增益
- `anti_phase_smoothing`: 反向波增益的平滑系数；当前要求在 `[0, 1)` 内

## `[vad]`

- `enabled`
- `local_threshold`
- `peer_threshold`
- `smoothing`

## `[residual]`

- `enabled`
- `strength`: 控制动态残余抑制和低电平噪声门的强度

## `[debug]`

- `dump_wav`
- `dump_metrics`
- `dump_dir`
- `log_level`
- `mock_peer_delay_ms`: `audio = mock` 且 `transport = mock` 时，控制本机串音相对对端参考的仿真延迟
- `mock_peer_gain`: `audio = mock` 且 `transport = mock` 时，控制注入到本机麦输入中的对端串音增益

## `[gui]`

- `auto_start`
- `refresh_hz`

当前 GUI 也会把 `listen_addr` / `peer_addr` 读入左侧表单，并通过 `Save Runtime Fields` 或 `Start` 持久化回配置。
当前 GUI 左侧还提供 `Noise Reduction` 控制区，可直接调整 `cancel.step_size`、`cancel.update_threshold`、`cancel.anti_phase_*`、`residual.enabled`、`residual.strength` 和 `output.monitor_processed_output`，再通过 `Apply Noise Controls` 或 `Save Runtime Fields` 落盘并请求 reload。

## Mock 场景说明

当 `audio.backend = "mock"` 且 `node.transport_backend = "mock"` 同时成立时，
运行时会启用内置的双人交替说话仿真：

- `peer_raw` 是对端原始参考流
- `local_raw` 是“本机近端 + 延迟后的对端串音泄漏 + 极低环境底噪”的混合
- 这样 `offline_replay` 能真实触发 sync 对齐、VAD 冻结和 NLMS 收敛
