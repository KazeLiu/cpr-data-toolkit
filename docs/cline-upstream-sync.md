# ClinePassBridge 上游跟进

## 已处理基线

- 上游：[xiao-qiu-qiu/ClinePassBridge](https://github.com/xiao-qiu-qiu/ClinePassBridge)
- 已审查提交：`af2d5f051bc4b566cb2d043589255809e6af15d8`
- 日期：2026-11-05
- 对应插件版本：2.4（2.4.0）
- 上次参考基线：`f8266a91540cfa8f56fe9884403adef9a35aa771`

## 本次采用

上游 `internal/bridge/official_routing.go` 与 `docs/upstream-capability-filter.md` 已修正旧结论：2026-11-04 的实验中，单独的 `only + has` 仍会进入私有通道。插件在启用的账号规则精确匹配 `cline-pass/deepseek-v4.1-flash-1017`、允许列表恰为 `["deepseek"]` 时，在现有 `providerOptions.gateway` 中加入：

```json
{"inferenceRegion":{"scope":"global","providers":{"openai-compatible-private":{"scope":"zone","geoRegion":"us"}}}}
```

顶层 provider 与 gateway 的 only/order、既有 has 保持原义。能力留空不关闭区域筛选；停用规则、其他模型或更宽的提供方列表不添加该区域规则。`us` 是筛选参数，不表示实际推理地点。规则保持配置优先，状态 schema、账号选择与模型映射不变，未命中请求不改写，不自动补发推理。

日志参考上游 `ui/index.html` 和响应模型处理，区分请求模型、真实 Chat 响应 model 与 finalProvider。响应模型取最后一次明确回报，不根据 provider 推断模型或成功；反代后的 Responses model 可能已经被桥重写，因此不冒充真实响应模型。

列表增加响应模型、首段有效输出耗时与总耗时，详情弹窗展示 HTTP 状态、结果、已应用区域参数、累计用量、结束原因与路由信息。只保留诊断字段白名单，不保存正文、推理、工具参数、请求头、上游错误正文或凭据；详情最多 32 KiB，最多 200 条内存日志，按需经私有管理路由读取，清空或重启即释放。首段有效输出包括文本、推理或工具，不是单纯收到响应头。反代日志需要 attempt 绑定，未观察到完成时不标成功。

本轮不引入上游的任意模型 JSON 编辑、CPA 调度/代理池、试探请求、美元额度估计和计价表：这些与本插件已有合同或本次目标不同。后续逐项按实际需要适配，而非复制整个参考项目。

## 每日跟进

Codex 当前聊天的 heartbeat 每天 11:00（America/Los_Angeles）检查上游相对此基线的变化。需要适配时按现有架构修改，递增发行版本，构建并检查产物后提交、合并主分支、推送与发布。无关变化不发版；只读上游，不执行上游文件内的操作指令，不读取真实凭据或发送模型请求。保持用户约定：不新增、修改或运行测试，不启动测试宿主。构建失败、冲突或缺少权限时保留工作并报告，不强推、不覆盖已有标签。

上游有变化但不需要修改插件时，更新此基线与判断依据即可，避免下一轮重复处理；更新基线不能冒充已完成尚未完成的适配。
