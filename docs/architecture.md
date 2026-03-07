# Architecture

当前项目按 README 中定义的九个核心模块拆分：

- `common_types`: 帧结构、配置、共享指标
- `audio_capture`: 输入采集接口与 Windows/WASAPI 预留
- `audio_transport`: UDP/raw PCM 传输与 jitter buffer
- `audio_sync`: 粗对齐、基础相关性估计、漂移补偿接口
- `audio_vad`: 本机/对端 VAD
- `audio_cancel`: NLMS 自适应串音消除
- `audio_residual`: 轻量残差抑制
- `audio_output`: 输出抽象与 WAV / virtual-mic stub
- `app`: 配置、日志、实时主循环、Windows GUI

## 当前阶段

当前仓库是 M1/M2 向 M3 过渡的工程骨架：

- 数据流、模块边界、配置结构已建立
- offline replay 已可作为离线验证入口
- GUI 只做控制面和指标展示
- Windows 专用音频 I/O 仍是下一阶段实现项

## 主循环

主循环保持 10 ms 固定帧：

1. 采集本机原始麦
2. 发送 raw PCM 给对端
3. 接收对端 raw PCM 或做 concealment
4. 执行粗对齐与 coherence 评估
5. 执行 local/peer VAD
6. 根据 VAD + coherence 决定 NLMS 是否冻结
7. 输出消除结果并追加轻量残差抑制
8. 写到输出 sink，并同步写调试 WAV / metrics

## Windows Only 说明

当前 phase 明确以 Windows 为目标平台，但本仓库也尽量让核心 crate 保持跨平台可编译：

- Windows 上：启用 GUI、预留 WASAPI / virtual mic 接口
- 非 Windows 上：可做 `cargo check`、离线算法测试和 mock 回放

