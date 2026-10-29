# Cline Pass 接入计划

## 目标与边界

- 在插件中增加「钉上游」，按 OpenAI API Key 账号和最终发送的完整模型 ID 配置提供方及能力条件，提供 DeepSeek 1017 预设
- 在发送前写入 Cline 路由字段，记录应用参数；在 attempt 响应保留路由元数据时记录 `finalProvider`，缺失保持未知
- 在「上游额度」增加 Cline Pass，读取官方 `users/me/plan/usage-limits`，复用快照、后台刷新和分享
- 按用户确认的口径，将 Cline 视为 Responses 上游，只处理 POST `/responses`，不实现 Chat Completions 适配
- 不修改宿主、SDK、账号凭据、现有模型映射或其他规则，不新增费用估计、自动探测或模型请求

## 合同与职责

当前锁定 SDK `8c8618a` 已提供 outbound v6、attempt 响应 frame 观察与 `account_http`，依赖保持不变。

- `src/upstream_pin.rs`：配置校验、版本冲突、发送前注入和有界非秘密日志
- 清单、入口和管理路由：声明 outbound/attempt，注册独立规则状态与 API
- `web/src/features/UpstreamPin.vue` 与 API：账号选择、规则编辑、DeepSeek 预设和命中日志
- `src/upstream/parse/cline_pass.rs`：Cline 成功信封和窗口解析
- 现有额度网络层、查询枚举、页面：增加 Cline Pass 模式，沿用受管 HTTP

参考固定为 ClinePassBridge v0.4.0（`f8266a9`）的 `executor.go`、`usage.go` 和能力过滤文档。提供方限制由 Cline 执行，响应后识别不能保证请求从未被送往其他提供方。

## 执行顺序与完成条件

1. 完成钉选后端及合同接线，未命中请求保持原样，命中规则以插件配置覆盖路由条件
2. 完成额度解析及查询接线，缺失百分比保持未知，失败保留旧快照且不冒充当前值
3. 完成页面和使用说明，区分参数应用与实际上游，明确 Responses 接入条件
4. 阅读全部相关差异及调用关系，核对状态 schema、管理路径、前后端字段与既有使用合同

## 验证与恢复

遵循本次协作约定，仅静态审查；不编写或运行测试，不启动应用、浏览器或执行构建。用户验收需启用 outbound 绑定，若需观察响应再启用 attempt；核对 Responses 扩展字段、实际来源及三档额度。停用/删除钉选规则即可停止参数注入；额度查询可切回「不查询」。

## 实现记录

钉选、额度后端、清单、页面和使用说明已接线。Responses 请求只覆盖路由命名空间，响应日志读取根对象及 `response` 内的明确 `finalProvider`。Cline 的普通模型目录为 BYOK，未用它或公开推荐列表推测账号模型权限；Cline Pass 快照的模型目录保持未知。

源码保持 1.6.0，未创建发行包，发布时按项目规则递增版本。已进行定向格式化和静态调用关系审查；构建、类型检查、自动化测试和真实宿主验收未执行。Cline 原生 `/responses` 支持及这些字段的实际路由效果属于待实测项，本次按用户指定接入口径实现，不以参考项目的 Chat 实验替代该验证。
