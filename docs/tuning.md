# Tuning

第一轮调参建议先从单讲场景开始，不要一上来就在双讲上追求极致消除深度。

## NLMS

- `filter_length`: 先用 `1536`
- `step_size`: 先从 `0.03 ~ 0.05`
- `leakage`: 先用 `1e-4`

## Sync

- `jitter_buffer_frames`: 一般从 `3` 开始
- `coarse_search_ms`: 先限制在 `30`
- 漂移补偿先保守，避免频繁跳动

## VAD / Freeze

- 先确保“本机单讲时不要被削空”
- `update_threshold` 不要过低
- 宁可双讲时残留一点串音，也不要让滤波器错误学习

## Residual

- `strength` 先用 `0.15 ~ 0.25`
- 先轻压，不要强门限
- 一旦出现机器人音，先回退 residual，再查 sync / cancel

