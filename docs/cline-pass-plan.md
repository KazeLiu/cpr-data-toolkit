# Cline 接入计划

## 目标与边界

- 在「Cline 接入」按 OpenAI API密钥账号和请求模型名称配置提供方及能力条件，统一通过「添加规则」维护，默认提供方与能力可修改
- 在发送前写入 Cline 路由字段，记录参数和上游明确回报的 `finalProvider`，缺失保持未知
- 在「上游额度」增加 Cline Pass，读取官方 `users/me/plan/usage-limits`，复用快照、后台刷新和分享
- 在同一个「Cline 接入」Tab 管理转换账号和 DeepSeek 钉选，下游继续调用 CPR
- 使用受管上游适配器，将 Responses 转成 Cline Chat Completions，并将 JSON / SSE 转回 Responses
- 不修改宿主、SDK、账号凭据、现有模型映射或其他规则，不新增费用估计、自动探测或模型请求

## 合同与职责

当前锁定 SDK `8c8618a` 已提供 upstream_adapter v5、outbound v6、attempt 响应观察与 `account_http`，依赖保持不变。账号范围从握手配置 `clineAccountIds` 注册，页面选择仅保存草稿，须复制到宿主插件参数并应用；页面区分草稿与当前实例配置。操作前先停用依赖转换的账号并排空请求。

- `src/upstream_pin.rs`：配置校验、版本冲突、发送前注入和有界非秘密日志
- `src/cline/`：转换账号状态、受管上游注册与执行、请求映射、跨块 SSE 和 Responses 事件生成
- 清单、入口和管理路由：声明 upstream_adapter、outbound/attempt，注册独立状态与 API
- `web/src/features/UpstreamPin.vue` 与 API：账号选择、规则编辑和命中日志；请求模型名称可修改，来源与规则一致时用宿主绿色语义色呈现
- `src/upstream/parse/cline_pass.rs`：Cline 成功信封和窗口解析
- 现有额度网络层、查询枚举、页面：增加 Cline Pass 模式，沿用受管 HTTP

参考固定为 ClinePassBridge v0.4.0（`f8266a9`）的 `executor.go`、`usage.go` 和能力过滤文档。提供方限制由 Cline 执行，响应后识别不能保证请求从未被送往其他提供方。

## 执行顺序与完成条件

1. 已提交现有代码至 `b382a0ab`，在 `codex/cline-adapter` worktree 分支开发
2. 实现转换文本、图片 URL、函数调用与结果及常用生成选项；有状态续接、原生工具、文件输入明确拒绝
3. 实现 JSON 和 SSE 的文本、推理、工具、用量、唯一终态及标准事实；错误或截断不能伪装成功
4. 已选择账号通过官方 Chat 端点发送，转换后按最终模型钉选；未选择账号及原有 Responses 钉选保持原路径
5. 完成额度解析及查询接线，缺失百分比保持未知，失败保留旧快照且不冒充当前值
6. 完成同一 Tab 的页面和使用说明，区分草稿、实例配置和实际上游，说明 upstream 绑定和配置应用要求
7. 阅读全部相关差异及调用关系，核对状态 schema、管理路径、前后端字段与既有使用合同

## 验证与恢复

实现阶段仅静态审查。用户随后授权打包 Linux ARM64，并在安装失败后授权查看测试站浏览器；不编写或运行自动化测试。真实宿主模型调用需用户验收，涵盖文本流、工具往返、非流式、上游拒绝与截断、实际来源及三档额度。原生 Responses 钉选仍使用 outbound / attempt。移除实例配置中的转换账号后恢复原生路径；额度查询可切回「不查询」。

## 实现记录

转换、钉选、额度后端、清单、页面和说明已接线。受管执行使用宿主已选账号，不读取原始凭据；JSON 与 SSE 均生成标准事实及 Responses wire。明文推理可随完整历史回灌，摘要与加密推理不冒充原始推理。原 Responses 路径保留，Cline 普通模型目录不用于推测 Pass 账号权限。

正式发行版本为 1.8.1，分支曾交付的 1.7、1.8 和 1.9 为本地测试包编号。协议基于 [Cline 官方 Chat 文档](https://docs.cline.bot/api/chat-completions)、[OpenAI Responses 流事件](https://developers.openai.com/api/reference/resources/responses/streaming-events) 和固定 SDK 的受管上游合同。

首包前后端构建与包检查通过，但用户在 CPR 3.33.0 首次安装时报“上游适配器注册失败”。固定宿主源码确认：`prepare_instance` 注册能力时，`PluginPrivateState.owner` 可能尚未存在，`host.state.get` 会返回 PermissionDenied；状态 fence 在提交后才由 `activate_state` 激活。原实现不应在注册阶段读取私有状态。

修复后注册只用 `Handshake.configuration`，草稿状态留到管理页面激活后读取。宿主桥不提供实例配置写入合同，因此页面提供“复制实例配置”，管理员在宿主设置中应用；未吞掉权限错误，也不借用宿主管理 HTTP。版本仍为同次失败修复的 1.7.0，新产物放在 `dist/1.7-registration-fix/`，旧首包保留。

注册修复包的前端类型检查与构建、Linux ARM64 release 构建、官方 CLI 打包及归档/资源/摘要检查均通过。测试站浏览器确认宿主为 3.33.0，用户选择自行上传验证；未执行线上安装、配置修改或模型请求。

1.8 接受相同 `finish_reason` 重复上报，冲突原因与结束后的正文或工具增量仍拒绝，Responses 终态保留在流收尾时统一生成。DeepSeek 1017 预设改为选择上游账号后直接开关，复用既有规则保存合同，不覆盖同一账号与模型的自定义规则；实例配置说明改为实际的两层设置弹窗路径。

1.8 前端类型检查与生产构建、Linux ARM64 release 构建及固定官方 CLI 打包均完成，确认包内为 ELF64 AArch64、glibc 最高 2.17、当前页面资源及 1.8.0 清单。原生包摘要和 ZIP 完整性检查通过，交付目录为 `D:/code/cpr-data-toolkit/dist/1.8/`，工作树保留同份产物。DSH 流收尾、预设开关与实际提供方由用户安装后验收；本次未启动宿主或发送模型请求。

1.9 按后续反馈移除 1017 预设入口，改为「添加规则」，保留已有规则。规则和日志使用「请求模型名称」「上游来源信息」，隐藏请求 ID、尝试序号和无能力过滤占位；未命中仅显示「未命中」。新增规则默认提供方 `deepseek`、能力 `vision, implicit-caching`，请求模型名称留空待填写。编辑保存通过可选 `originalModel` 精确定位旧规则，在一次版本比较写入中替换；缺少旧规则或新名称已被同账号其他规则占用时拒绝修改。未提供 `originalModel` 的旧调用继续按原有新增或覆盖合同处理，状态 schema 不变。上游明确回报来源且与已应用规则一致才显示绿色，其他情况保留普通文字。源码检查与打包检查分别记录，未运行测试或启动页面。

## 合并检查点

1.9 前端类型检查与生产构建、Linux ARM64 release 编译和固定官方 CLI 打包完成。包内为 ELF64 AArch64、glibc 最高 2.17，包含当前页面资源与 1.9.0 清单，文件及归档摘要核对通过。安装、Cline 调用、工具往返和页面交互由用户测试；确认后再合并与发布，本次不启动服务、不运行测试、不发送模型请求。

实现已保存为开发分支提交 `2dc3dc5e`，本次按用户指定版本 1.8.1 合入主线。合并保留 1.7.2 的状态迁移、统一文案和复选框修复；既有命名空间 schema 不变，只新增 `cline_adapter`。正式包使用现有三平台工作流构建，发行和安装入口见 [1.8.1 发布说明](releases/1.8.1.md)。
