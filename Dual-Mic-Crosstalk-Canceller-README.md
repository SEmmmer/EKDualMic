# Dual-Mic Crosstalk Canceller

## 项目简介

本项目要实现一个 **Windows 双机协同的实时串音消除器**，用于“双人共用一张电脑桌、同时直播/语音聊天/打游戏”的场景。

两台电脑各运行一个本地节点。每个节点都要：

1. 采集 **本机原始麦克风音频**
2. 通过局域网把 **本机原始麦克风音频** 发给对端
3. 接收对端发来的 **对端原始麦克风参考音频**
4. 使用“本机原始麦 + 对端原始麦参考”做 **实时自适应串音消除**
5. 输出一个 **处理后的虚拟麦克风**，供 Discord、游戏语音、OBS、会议软件等统一使用

本项目的目标不是普通环境降噪，而是尽可能消除 **旁边另一个真人说话被自己麦克风收进去** 的问题。

---

## 背景与问题定义

当前场景中，两位用户并排坐在同一张桌子旁，各自使用一套耳机麦克风。  
问题不是回放回声，也不是键盘噪声，而是：

- A 的麦克风会收进 B 的人声
- B 的麦克风会收进 A 的人声
- 这种串音会同时影响：
  - OBS 直播
  - Discord / 游戏语音
  - 其他任意使用系统麦克风的软件

普通噪声抑制、门限、压缩器、会议耳机算法、Blue VO!CE 这类手段都不够稳定。  
因此必须实现一个真正的 **双麦参考消除系统**。

---

## 项目目标

### 核心目标

开发一个可实时运行的双机协同串音消除器，满足以下要求：

- 可以在 Windows 上运行
- 可以同时服务于 OBS、Discord、游戏语音等所有软件
- 对端单独说话时，本机输出中的对端串音明显下降
- 本机单独说话时，不能明显损伤本机自己的声音
- 双讲时宁可残留少量串音，也不能把本机人声打坏
- 软件整体可长期稳定运行，而不是只能用于离线实验

### 最终交付目标

交付一个可运行的 MVP，包含：

- 双端节点程序
- 实时音频采集
- 局域网音频传输
- 延迟/漂移对齐
- 自适应串音消除
- 双讲检测与滤波冻结
- 处理后音频输出到虚拟麦克风
- 基础日志、调试导出、配置文件

---

## 明确不做的事

### v1 不做

- 不从零写内核驱动
- 不一开始就写 APO
- 不先做深度学习说话人分离
- 不依赖 Blue VO!CE、AGC、压缩器、噪声门 作为主方案
- 不先做复杂 GUI
- 不先做跨平台
- 不先做云端或互联网跨公网传输

### v1 禁止项

发送给对端的参考音频必须是 **未处理的原始麦克风音频**。  
以下处理在 v1 主链路中默认禁止：

- AGC
- Compressor
- Expander / Gate
- Limiter
- Blue VO!CE
- 发送端噪声抑制
- 发送端编码压缩
- 任何会改变参考信号动态特性的前处理

这些处理会破坏参考信号与串音路径建模，降低自适应消除效果。

---

## 硬性约束

### 运行平台

- Windows 10 / Windows 11
- 两台电脑位于同一局域网
- 每台电脑各接入自己的物理麦克风
- 每台电脑最终都需要输出到自己的“处理后系统麦克风”

### 音频格式

内部处理统一使用：

- Sample Rate: `48000`
- Channels: `1`
- Sample Format: `float32`
- Frame Size: `10 ms`
- Samples Per Frame: `480`

### 工程语言

- **首选 Rust**
- 必要时允许少量 FFI / 平台绑定
- 不要为了省事把核心 DSP 逻辑写成不可维护的脚本堆叠

### 处理优先级

1. 实时稳定
2. 音质可接受
3. 对端串音抑制有效
4. 延迟尽量低
5. 架构便于后续升级

---

## 总体架构

每台机器运行一个完全相同的节点程序。

### 单节点流水线

`本机原始麦采集 -> 分帧 -> 发送到对端`

`接收对端原始麦参考 -> 抖动缓冲 -> 对齐 -> 自适应消除 -> 残差抑制 -> 输出到虚拟麦`

### 双机拓扑

- 节点 A：
  - 采集 A 的原始麦
  - 接收 B 的原始麦参考
  - 输出 A 的处理后虚拟麦
- 节点 B：
  - 采集 B 的原始麦
  - 接收 A 的原始麦参考
  - 输出 B 的处理后虚拟麦

### 关键原则

- 双端对称设计
- 数据帧固定为 10 ms
- 参考音频必须走独立链路
- 消除前必须先做延迟与漂移对齐
- 双讲时必须冻结自适应更新

---

## 核心算法要求

## 1. 延迟与漂移对齐

这是核心模块，不是附属功能。

由于两台电脑各自采样、传输、缓冲，参考流与本机串音路径之间一定存在：

- 固定延迟
- 抖动
- 时钟漂移
- 轻微采样率偏差

必须实现一个 `sync` 模块，负责：

- 对端参考流 jitter buffer
- 粗延迟估计
- 细粒度漂移补偿
- 输出与本机当前帧对齐的参考流

### 要求

- 粗延迟估计可基于互相关 / GCC-PHAT
- 搜索窗口建议先做 `±30 ms`
- 粗延迟不必每帧更新，避免频繁跳动
- 细漂移用缓慢调节的重采样或插值补偿
- 不允许写死固定延迟

---

## 2. 自适应串音消除

### 问题模型

设本机为 A，对端为 B：

- 本机麦输入：`xA = sA + crosstalk(B->A) + noise`
- 对端参考：`rB = sB + leak(A->B) + noise`

目标是估计 `B->A` 这条传递路径，并从 `xA` 中减去估计出的 `B` 串音分量。

### v1 算法要求

第一版先实现传统自适应滤波：

- MVP：`NLMS`
- 后续可升级：`频域分块 NLMS / PBFDAF`

### 参数建议

- Filter Length: `1024 ~ 2048 taps`
- Step Size: 自适应，初始可从 `0.02 ~ 0.08` 开始
- Leakage: 小正值，避免发散
- 支持更新冻结
- 支持状态复位

### 重要说明

**禁止简单“反相相减”。**

不能直接把对端参考乘一个系数后从本机麦中减掉。  
必须做真正的路径建模和自适应更新。

---

## 3. 双讲检测

双讲保护是成败关键。

### 原则

只有在以下条件同时成立时，才允许更新自适应滤波器：

- 对端在说话
- 本机没有说话
- 本机输入与参考流具有足够相关性

一旦检测到：

- 本机在说话
- 双讲
- 当前相关性异常

就必须：

- 冻结滤波器更新
- 继续使用上一次稳定滤波器做输出
- 禁止把本机自己的声音“学进”滤波器

### 推荐输入信号

- `vad_local`
- `vad_peer`
- `coherence(local, aligned_peer)`
- 能量门限
- 相关性得分平滑结果

### 行为要求

- 双讲时不追求最大消除深度
- 双讲时优先保护本机人声自然度
- 不允许出现明显的“把自己声音削空”现象

---

## 4. 残差抑制

自适应消除后仍可能残留：

- 少量对端人声
- 环境底噪
- 高频伪影
- 对齐误差带来的残差

因此需要一个轻量的残差抑制阶段。

### 要求

- 放在自适应消除之后
- 默认做轻处理
- 不允许明显机器人音
- 不允许大幅拉低本机清晰度

### v1 可接受方案

- 轻量 spectral gate
- Wiener-like suppression
- 可插拔残差抑制模块

### 说明

残差抑制是辅助模块，不是主算法。  
主算法仍然是 **参考通道自适应串音消除**。

---

## I/O 与系统集成

## 1. 音频采集

使用 Windows 正规音频 API 采集本机物理麦克风。  
要求：

- 可枚举输入设备
- 可选择目标麦克风
- 固定为 48 kHz mono float32
- 输出 10 ms 帧
- 支持日志和异常恢复

## 2. 网络传输

使用局域网 UDP 传输参考音频。

### 每个数据包包含

- `sequence_number`
- `capture_timestamp`
- `sample_rate`
- `frame_samples`
- `optional vad info`
- `raw pcm payload`

### v1 约束

- 不做音频压缩编码
- 不做公网 NAT 穿透
- 不做复杂重传
- 丢包优先用轻量 concealment 或保持上一帧，不做高复杂恢复

## 3. 输出到虚拟麦克风

v1 不从零实现驱动。

### v1 要求

- 处理后音频必须能作为系统级“麦克风输入”被其他软件使用
- 优先对接现成虚拟音频输入/桥接方案
- 输出模块应做成抽象接口，便于未来替换为自定义虚拟设备

### v2 预留

后续可升级为：

- 自定义虚拟 capture endpoint
- 系统级更稳定的集成方式
- 更低延迟的 Windows 音频链路集成

---

## 代码结构要求

建议仓库结构如下：

```text
/
├─ README.md
├─ Cargo.toml
├─ configs/
│  ├─ node-a.toml
│  └─ node-b.toml
├─ crates/
│  ├─ audio_capture/
│  ├─ audio_transport/
│  ├─ audio_sync/
│  ├─ audio_vad/
│  ├─ audio_cancel/
│  ├─ audio_residual/
│  ├─ audio_output/
│  ├─ common_types/
│  └─ app/
├─ tools/
│  ├─ offline_replay/
│  └─ wav_dump/
└─ docs/
   ├─ architecture.md
   ├─ config.md
   └─ tuning.md
```

### 各模块职责

#### `audio_capture`
- 枚举输入设备
- 打开本机物理麦克风
- 采集并输出 10 ms 标准帧

#### `audio_transport`
- UDP 发送/接收
- 包头解析
- jitter buffer
- 丢包统计

#### `audio_sync`
- 粗延迟估计
- 漂移估计
- 重采样/插值补偿
- 输出对齐后的参考流

#### `audio_vad`
- 本机/对端语音活动检测
- 平滑和置信度输出

#### `audio_cancel`
- 自适应滤波
- 双讲检测
- 滤波更新冻结
- 主串音消除逻辑

#### `audio_residual`
- 残差抑制
- 轻量后处理

#### `audio_output`
- 处理后音频写入系统输出链路
- 对接虚拟麦

#### `app`
- 配置加载
- 主循环
- 线程管理
- 日志与调试命令

---

## 主循环行为

主处理循环必须按帧工作。伪代码如下：

```rust
loop every 10ms {
    let local_raw = capture.read_frame();
    transport.send(local_raw);

    let peer_raw = transport.recv_or_conceal();
    let peer_aligned = sync.align(peer_raw, local_raw);

    let vad_local = vad.detect_local(local_raw);
    let vad_peer = vad.detect_peer(peer_aligned);
    let corr = sync.coherence(local_raw, peer_aligned);

    if vad_peer.is_high() && vad_local.is_low() && corr > UPDATE_THRESHOLD {
        cancel.update(peer_aligned, local_raw);
    } else {
        cancel.freeze_update();
    }

    let canceled = cancel.process(local_raw, peer_aligned);
    let output = residual.process(canceled);

    output_device.write_frame(output);
    debug.maybe_dump(local_raw, peer_raw, peer_aligned, output);
}
```

---

## 配置要求

使用 `TOML` 配置文件。

示例：

```toml
[node]
name = "node-a"
listen_addr = "0.0.0.0:38001"
peer_addr = "192.168.1.22:38001"

[audio]
input_device = "Microphone (Your Headset)"
sample_rate = 48000
channels = 1
frame_ms = 10

[sync]
jitter_buffer_frames = 3
coarse_search_ms = 30
drift_compensation = true

[cancel]
filter_length = 1536
step_size = 0.04
leakage = 0.0001
update_threshold = 0.65

[vad]
enabled = true
local_threshold = 0.6
peer_threshold = 0.6

[residual]
enabled = true
strength = 0.2

[debug]
dump_wav = true
dump_metrics = true
log_level = "info"
```

---

## 调试与可观测性要求

必须提供调试能力，否则无法调参。

### 至少需要输出

- `local_raw.wav`
- `peer_raw.wav`
- `peer_aligned.wav`
- `output.wav`

### 至少需要记录的指标

- 当前粗延迟估计值
- 漂移补偿值
- VAD 状态
- 滤波器是否处于冻结状态
- 消除前后能量对比
- 丢包率
- 输出削波次数
- 每秒处理耗时统计

### 日志要求

- 关键状态变更必须打日志
- 不允许只有 println
- 提供 `info / debug / trace` 分级日志

---

## 测试要求

必须先做 **离线回放测试**，再做实时联调。

### 离线测试集至少包含

1. A 单讲，B 静音
2. B 单讲，A 静音
3. A/B 轮流讲话
4. A/B 双讲
5. 键盘、鼠标、桌面碰撞等突发噪声
6. 不同说话音量
7. 网络轻微抖动场景

### 验收标准

#### 功能性
- 程序可长时间稳定运行
- 两端可互联
- 输出可被 Discord / OBS / 游戏语音使用

#### 音频效果
- 对端单讲时，本机输出中对端串音有明显下降
- 本机单讲时，自身声音不得明显被吃掉
- 双讲时，不得出现长时间掉字、抽吸、失真
- 不得出现明显爆音、削波、时不时炸麦

#### 工程表现
- 配置可调
- 调试信息完整
- 模块职责清晰
- 代码可维护

---

## 里程碑

## M1：打通数据链路
- 本机采集 10 ms 帧
- UDP 互发 raw PCM
- 双端各自保存原始 WAV
- 打通基本配置与日志

## M2：做同步层
- jitter buffer
- 粗延迟估计
- 基础对齐输出
- 离线验证对齐质量

## M3：做主消除器
- NLMS 自适应滤波
- 更新/冻结机制
- 双讲检测
- 单讲场景验证

## M4：做残差处理与系统接入
- 残差抑制
- 对接虚拟麦
- Discord / OBS / 游戏内联调

## M5：做稳定性与调参
- 漂移补偿优化
- 调试面板或调试命令
- 参数热更新
- 崩溃恢复与异常处理

---

## 开发要求

### 代码风格
- 模块边界明确
- 关键 DSP 逻辑写注释
- 不要把算法、I/O、网络全写进一个文件
- 不要把参数写死在代码里
- 所有 magic number 必须进入配置或常量定义

### 性能要求
- 以实时为第一优先级
- 避免无意义拷贝
- 避免在实时线程中做阻塞 I/O
- 避免在音频回调里做复杂日志写盘

### 错误处理
- 网络异常要可恢复
- 对端掉线要有 graceful fallback
- 输入设备断开要有明确报错
- 不允许静默失败

---

## 未来升级方向

以下内容不是 v1 必做，但架构需要预留：

- 更强的频域自适应滤波
- 更鲁棒的双讲检测
- 说话人特征约束
- 深度学习目标说话人提取
- 自定义虚拟音频设备
- 更低延迟系统集成
- GUI 配置界面
- 自动标定与自动调参

---

## 一句话总结

这个项目要做的不是普通“降噪”，而是一个 **双机协同、参考通道驱动、面向真实直播与语音软件使用的实时串音消除器**。  
第一版必须优先解决：

- 原始参考传输
- 延迟/漂移对齐
- 自适应串音消除
- 双讲保护
- 虚拟麦输出

不要一开始把精力浪费在驱动、GUI、美化音效、复杂模型上。先做出一个 **能真实改善双人共桌串音问题** 的 MVP。
