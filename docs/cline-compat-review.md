# Cline 转换兼容与热路径审查

## 交付范围

已完成转换、钉选与相关日志路径的源码修正，宿主、SDK 和 CPA 未变。修正已进入 1.9 Linux ARM64 测试包；用户已确认的 1.8 普通对话结果适用于原包，新包的实际调用仍待用户验收。

## 参考基线

- CPA：`router-for-me/CLIProxyAPI`，提交 `90c447d3a8727d115aa7552ec93ce023736ed492`，只读副本 `.tools/cpa-reference`
  - `internal/translator/openai/openai/responses/openai_openai-responses_request.go`
  - `internal/translator/openai/openai/responses/openai_openai-responses_response.go`
  - `internal/runtime/executor/openai_compat_executor.go`
- CPR 当前源码：`958e13c370d05e005daa931ba9b98cd5a9c2c312`
  - `gateway-api/src/openai/chat_completions/request.rs` 是 Chat → Responses，不是相反方向
  - 插件仍按锁定 SDK `8c8618aa` 合同实现，不升级依赖
- [Cline Chat 文档](https://docs.cline.bot/api/chat-completions)：支持函数工具与尾部 usage，公开表未确认 `prompt_cache_key`
- [DeepSeek 缓存文档](https://api-docs.deepseek.com/guides/kv_cache/)：缓存依赖可复用前缀，属于上游尽力提供的能力

## 参考实现解决了什么问题

- CPA 的 SSE 解析区分事件名称、数据和完整帧边界，能够识别只有 `event: error` 的错误；不能只把每个 `data:` 当正常生成内容。
- CPA 在 `finish_reason` 后保留接收尾部 usage 的机会，再发送 Responses 完成事件。插件原有的唯一终态设计继续保留，本轮补上 usage-only 帧和缺省 delta 的结束帧。
- CPA 按 index 维护工具身份并缓冲参数。插件采用同样的状态边界：允许 ID、名称和参数分批到达，已发布的调用身份不能改变；已发布名称的重复回报不再报错。
- CPA 的请求转换会合并同轮 assistant 文本与函数调用；插件补齐函数调用之后才出现文本的方向。
- CPR 检查结构和字段后再做协议转换。锁定版本的适配器运行时会合并 observation 的非空更新，因此插件只需发送初始观察信息和发生变化的字段，不必逐事件重复携带。
- CPA 仅在兼容配置明确支持时注入自动缓存键。Cline 公开参数未确认该能力，本轮不照搬；也不移植占位推理、孤立工具结果改写或自动补发模型请求。

## 已修改的行为

### 转换与错误处理

- 工具增量允许缺省或 null 的 `function` / `type`；缺少 index、无效名称、变更 ID、重复调用 ID 仍报错。
- 名称发布前继续拼接分片，发布后允许重复同一完整名称，不拼入两遍；真正变更名称仍报错。
- 参数先到、身份后到时，身份齐全即可发出已缓冲内容。发出后的参数只保留一份最终累计值，不再在两个结构中重复累积。
- 纯文本内容数组归一为字符串，不插入额外换行；图片和混合内容保留数组。相邻、包含工具调用的 assistant 项合并为 Chat 同轮消息，保持文本、工具及参数顺序。
- 分批 usage 合并后按已有输入、输出重新计算缺省总数，避免保留过期的派生总数；只有一侧计数时不抹掉上游明确回报的总数。
- `cached_tokens: null` 不再遮住 DeepSeek 的 `prompt_cache_hit_tokens`。输入用量仍按上游口径保留，不重复累加缓存命中数。
- 支持 usage-only 流帧和只有 finish_reason 的结束帧。SSE `error`、`response.failed`、`response.error` 事件即使没有正常 JSON 正文也不会被忽略；已有结构化错误继续用于区分限流、额度及参数错误。

### 请求和流的开销

- 钉选配置从“每次请求读取私有状态”改为实例进程内缓存。首次实际使用才读取，注册阶段不访问状态；管理读取、保存、删除与缓存更新串行，失败后清空缓存，下次重新读取。
- 出站规则筛选不再为每次请求分配临时规则数组。
- 路由元数据只在变化时编码、提交和记录，service tier 同样按变化提交。
- Cline 转换器直接记录路由与错误，Attempt 日志中间件不再解析转换后的每个 Responses 事件。普通 `/responses` 钉选仍保留原来的响应观察。
- SSE 记住已扫描位置，跨块长行只扫描新增字节；完整帧逐个转换、逐个向下游交付，避免先把一个网络块里的所有帧扩展成 Responses 事件再交付首个事件。

## 对其他请求与整体运行的影响

- 转换器只注册冻结配置 `clineAccountIds` 中的账号，执行时再次核对账号、provider、adapter 和协议。未选中账号不进入转换器。
- 普通出站钉选先检查 OpenAI、HTTP、`POST .../responses`，再查启用的账号规则，最后按正文中的完整模型 ID 匹配。没有该账号规则时不解析正文；有账号规则但模型不匹配时保留原始字节与头，不重新编码正文。
- 已命中的钉选请求会设置 `provider` 和 `providerOptions.gateway`，并移除失效的 Content-Length；这正是钉选的配置效果。Cline 账号的同一规则在 Chat 请求发送前应用。
- 钉选缓存属于一个实例，管理操作成功返回后，新请求使用新配置；已取得旧快照的在途请求继续使用其快照。私有状态按宿主实例 owner 隔离，重启后重新加载；规则管理期间相关请求可能短暂等待状态操作。
- 全插件仍包含原有 Key 模型权限、模型 UA、模型优先级及额度维护。Key 权限会按配置拒绝请求或过滤模型目录，模型 UA 会改写命中规则的头，调度功能会影响账号选择。本轮没有改变这些既有行为，也没有缓存权限判断或改变调度策略来换取速度。
- 非 Cline 请求仍有已绑定中间件的 RPC 转发成本，Key 权限与有模型请求的 UA 配置仍会读取状态。不能把“转换器不接管”理解为“整个插件零开销”。

## 缓存与验收

本轮减少文本历史形态变化，保留消息前缀、工具 ID、工具参数和已有钉选条件，不注入随机缓存键、不伪造推理、不替换用户历史。实际缓存由 Cline 及最终提供方决定，取决于共同前缀、缓存存活时间和路由。缓存命中率、首 Token 延迟和吞吐没有实测数值。

已进行调用关系与差异自查、Rust 格式化及 `git diff --check`。1.9 的前端类型检查与生产构建、ARM64 release 编译、官方 CLI 打包及平台、资源和摘要检查通过。没有新增、修改或运行测试，没有启动应用或发送模型请求。关键待验收路径是多轮工具往返、并行工具流、尾部用量及错误帧、规则更新后的新请求，以及未选中账号的正常请求；运行结果由用户在目标宿主确认。

`previous_response_id`、WebSocket、加密推理历史和原生工具的能力边界未变；这些需要另外的会话、传输或执行能力，不属于本轮格式兼容修正。
