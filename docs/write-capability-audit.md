# CPR 关键写入能力核对

核对日期：2026-10-13。用户确认当前 CPR 为 `v3.26.1`。本次按公开发布标签核对，标签对象为 `2d5732df7f09117f219614aca8f79e55fd038e4e`，解引用后的源码提交为 `5358de53ddd4bda529cb63c422fd5c6af070f9c9`。后续 SDK 接入计划使用该固定提交，不使用 main。

本次核对公开 SDK、回调分派、服务注册、管理路由及 API 文档。没有连接用户的 CPR 实例或修改真实账号、统计、价格；源码合同与目标宿主运行验收分开记录。初次 v3.25.0 调研基线保留在 `initial-research.md`。

## 结论

| 功能 | 公开写入能力 | 对本项目的影响 |
| --- | --- | --- |
| API 导入 | 已有 `host.auth.save` | 可开发来源适配、预览和幂等导入；协议映射与真实调用仍需验收 |
| 历史用量导入 | v3.26.1 仍未找到外部历史明细或聚合补入入口 | 计划建议先做解析、去重和预览，确认导入提示「上游接口未开放」；原生写入保留为后续依赖 |
| 手动调整 | v3.26.1 仍未找到 Token 与请求数的修正入口 | 按用户要求只做界面，点击「确认调整」提示「上游接口未开放」，不变更统计 |
| 模型定价 | 已有公开 `settings` 服务 | 可直接开发读取、同步及批量维护，无须先新增定价写入入口 |
| 历史费用重算 | v3.26.1 没有公开的历史费用补全或覆盖写入入口 | 更新当前价目不能写回旧请求费用，需要宿主新增公开接口后由插件调用 |
| 模型UA定制 | 已有公开 request 中间件和同名头覆盖合同 | 可按请求模型名覆盖 UA，未命中保留 Key 有效身份 |

当前宿主依赖仍是外部用量补入与统计修正。五个标签页的阶段范围和审批口径见 `implementation-plan.md`。

## 账号导入：已有合同

SDK 提供 `host.auth.list/get_runtime/get/save`。新建账号使用 `AuthSaveRequest::Create`，Provider 显式指定，账号 ID 由宿主生成；替换携带精确 `credential_revision`。凭据经二进制 RPC 载荷传递。

这说明账号写入入口存在，不代表所有 CCS 配置都已兼容。来源协议、URL、Key 和必要凭据事实仍需要映射，并在目标宿主验证调用。

证据：[v3.26.1 账号合同](https://github.com/zyycn/codex-proxy-rs/blob/5358de53ddd4bda529cb63c422fd5c6af070f9c9/backend/crates/gateway-plugin/sdk/src/call/host.rs)、[宿主账号回调分派](https://github.com/zyycn/codex-proxy-rs/blob/5358de53ddd4bda529cb63c422fd5c6af070f9c9/backend/crates/gateway-plugin/runtime/src/callback/mod.rs)。

## 模型定价：公开服务可以主动调用

插件后端通过 `call.host.service::<O>(input)` 调用 `call::services::settings` 的类型化操作：

| 操作类型 | 服务名称 | 用途 |
| --- | --- | --- |
| `Pricing` | `settings.pricing` | 读取内置、同步与人工价目 |
| `PreviewPricingSync` | `settings.preview_pricing_sync` | 获取宿主定价来源的同步预览 |
| `Sync` | `settings.sync_pricing` | 确认同步选中的模型 |
| `Update` | `settings.update_pricing` | 替换价格、设置倍率、重置覆盖或删除非内置价目 |

已核对 SDK 类型化操作、`host.services.call` 回调和 settings 服务注册。页面只调用插件自己的管理路由，由插件后端发起服务调用，不获取管理员 Cookie。

证据：[SDK 操作合同](https://github.com/zyycn/codex-proxy-rs/blob/5358de53ddd4bda529cb63c422fd5c6af070f9c9/backend/crates/gateway-plugin/sdk/src/call/services/settings.rs)、[宿主回调分派](https://github.com/zyycn/codex-proxy-rs/blob/5358de53ddd4bda529cb63c422fd5c6af070f9c9/backend/crates/gateway-plugin/runtime/src/callback/mod.rs)、[settings 服务注册](https://github.com/zyycn/codex-proxy-rs/blob/5358de53ddd4bda529cb63c422fd5c6af070f9c9/backend/crates/gateway-admin/src/public_service/settings.rs)。

现有定价合同的边界：

- 价目以 Provider 和精确上游模型 ID 为键；当前原生 Provider 为 `openai` 与 `xai`，作用域为全局价目，不是每站点或每账号独立价格
- 更新校验允许新增非内置模型 ID，需要提供 `standard` 价格；没有要求模型名必须属于 GPT 系列。这不等于承诺相应上游协议和模型均可调用或完整估价
- 单次更新接受 1～500 个模型，应用同一个 `change`；不同模型的不同单价需要分组或逐项提交，跨请求不构成一个原子事务
- 内置模型不能删除，可以重置人工覆盖；非内置模型可删除
- 宿主 models.dev 同步只接受可表示的 OpenAI/xAI 文本价目。计划中 OpenAI 兼容上游的其他品牌模型先按精确上游模型 ID 手工批量维护；新价格来源另行讨论
- 改价影响后续本地估算及相应限额，不自动重算历史费用
- SDK 的 Rust/Serde 数据合同与管理 HTTP 的 `{ action: ... }` JSON 是不同边界，不能把 HTTP 请求结构直接当作类型化服务输入

证据：[v3.26.1 定价 API 及同步边界](https://github.com/zyycn/codex-proxy-rs/blob/5358de53ddd4bda529cb63c422fd5c6af070f9c9/docs/api.md#模型定价)。

### 历史费用同步：宿主接口尚缺

2026-10-14 按用户要求再次核对固定的 v3.26.1 源码。SDK settings 合同与服务注册仍仅提供当前价目读取、同步及更新；请求记录、详情与汇总路由均为 GET，没有历史费用补全或覆盖操作。插件不能通过这些入口把旧请求中的空费用写成当前价目的估算值，也不直接修改宿主数据库。

用户已确认 0.5 暂时只做界面：模型定价页「models.dev 社区价目」右侧增加「价格同步」，弹窗默认只补全缺失费用，也可选择按当前价目覆盖全部历史费用。确认仅提示「上游接口未开放」，不写回历史费用、不修改宿主。真实写回仍依赖 CPR 宿主新增公开接口。

证据：[SDK settings 合同](https://github.com/zyycn/codex-proxy-rs/blob/5358de53ddd4bda529cb63c422fd5c6af070f9c9/backend/crates/gateway-plugin/sdk/src/call/services/settings.rs)、[公开服务注册](https://github.com/zyycn/codex-proxy-rs/blob/5358de53ddd4bda529cb63c422fd5c6af070f9c9/backend/crates/gateway-admin/src/public_service/settings.rs)、[请求记录管理路由](https://github.com/zyycn/codex-proxy-rs/blob/5358de53ddd4bda529cb63c422fd5c6af070f9c9/backend/crates/gateway-api/src/admin/observability/routes.rs)。

## 历史用量和手动修正：未找到公开写入合同

对 v3.26.1 的 SDK 宿主资源合同、完整回调分派、公开服务注册表、settings 操作以及管理端观测路由进行了核对，未找到接受外部用量、日聚合或正负修正量的公开操作。SDK 类型化服务模块仍只有 settings；公开注册表登记 settings 服务；观测路由提供记录、详情、汇总和洞察的 GET 查询，没有导入或修正路由。管理根路由也没有登记独立的统计写入模块。

SDK 还支持 `HostClient::dispatch_http`，可以由后端使用宿主签发身份调用已存在的相对管理路由，不需要管理员 Cookie；但它不能补出尚不存在的用量写入业务操作。因此本次缺口不能仅通过换用 HTTP 分派解决。

已有相近能力不具备所需语义：

- `observer.request_completed` 投递真实请求完成事件，是有界实时观察，不能向宿主提交历史请求或修正量
- `host.state.*` 写入插件私有状态，不会把该状态合入 CPR 原生用量视图
- Client Key 预算接口修改日/周金额上限或重置当前窗口，不提供 token、请求数或历史统计的修正
- 模型执行回调会实际进入路由与计费链，不能用虚构执行代替历史数据补入

证据：[只读用量路由](https://github.com/zyycn/codex-proxy-rs/blob/5358de53ddd4bda529cb63c422fd5c6af070f9c9/backend/crates/gateway-api/src/admin/observability/routes.rs)、[管理根路由](https://github.com/zyycn/codex-proxy-rs/blob/5358de53ddd4bda529cb63c422fd5c6af070f9c9/backend/crates/gateway-api/src/admin/mod.rs)、[公开回调分派](https://github.com/zyycn/codex-proxy-rs/blob/5358de53ddd4bda529cb63c422fd5c6af070f9c9/backend/crates/gateway-plugin/runtime/src/callback/mod.rs)、[公开服务目录](https://github.com/zyycn/codex-proxy-rs/blob/5358de53ddd4bda529cb63c422fd5c6af070f9c9/backend/crates/gateway-admin/src/public_service/registry.rs)、[SDK 服务模块](https://github.com/zyycn/codex-proxy-rs/blob/5358de53ddd4bda529cb63c422fd5c6af070f9c9/backend/crates/gateway-plugin/sdk/src/call/services/mod.rs)、[HTTP 分派作者入口](https://github.com/zyycn/codex-proxy-rs/blob/5358de53ddd4bda529cb63c422fd5c6af070f9c9/backend/crates/gateway-plugin/sdk/src/client/middleware/http.rs)。

## 模型UA定制：请求中间件可以覆盖身份头

request 中间件提供 Client Key、请求模型及原始头视图。通过增量 remove／append 修改指定头，不替换正文或响应流。OpenAI 与 xAI 先生成账号和客户端画像的上游头，再应用插件同名头覆盖；因此 OpenAI 兼容 API Key 上游可使用这一路径。

首版范围复用宿主 request 绑定，按客户端公开模型名匹配。选中的 UA 与可选 originator、version 作为单次请求修改，不保存回 Key 的统一画像。宿主默认画像不会覆盖它，但其他改头插件或隐私规则的最终效果仍需在目标配置下验收。

CPR v3.26.1 发布说明还明确：中间件链不启用可替换前导的重试例外。加入 UA request 中间件后不能假定请求仍享有未启用中间件时的该例外，也不在插件内自行重放请求。

证据：[请求中间件合同](https://github.com/zyycn/codex-proxy-rs/blob/5358de53ddd4bda529cb63c422fd5c6af070f9c9/backend/crates/gateway-plugin/sdk/docs/capabilities.md#模型请求与-attempt)、[请求头和修改类型](https://github.com/zyycn/codex-proxy-rs/blob/5358de53ddd4bda529cb63c422fd5c6af070f9c9/backend/crates/gateway-plugin/sdk/src/call/middleware/mod.rs)、[宿主绑定范围](https://github.com/zyycn/codex-proxy-rs/blob/5358de53ddd4bda529cb63c422fd5c6af070f9c9/docs/api.md#能力绑定与范围)、[v3.26.1 发布说明](https://github.com/zyycn/codex-proxy-rs/releases/tag/v3.26.1)。

## 补接口时的统计口径

初次 v3.25.0 调研确认原生统计分别维护首字耗时的总和、样本数与分布，不能据此断言增加请求数必然算错首字均值。此次 v3.26.1 核对的 API 文档仍将缺失延迟保持未知，而不是补成零。

新增外部统计和手动修正时，可让这些数字进入总量和趋势，保留真实耗时样本的均值。缺少模型或费用的部分保持未知，并显示统计覆盖范围；用户提交前仍看到已确定的提醒。具体合并规则、费用覆盖显示与减量边界由新宿主合同确定，不在插件中假造明细。

证据：[v3.25.0 统计采样实现](https://github.com/zyycn/codex-proxy-rs/blob/05ba116eef849a0f4fa2d1940be82182bb2f7683/backend/crates/gateway-store/src/postgres/observability/analytics/metrics.rs)、[v3.26.1 记录范围与统计口径](https://github.com/zyycn/codex-proxy-rs/blob/5358de53ddd4bda529cb63c422fd5c6af070f9c9/docs/api.md#记录范围与统计口径)。

## 可以发给 CPR 开发者的说明

> 我在开发 cpr-data-toolkit 插件，希望把 CC Switch 等来源的历史 token 与请求统计去重后补入 CPR 原生用量统计，并允许用户手动填写 token、请求次数的正负修正量。
>
> 已核对 v3.26.1（5358de5）的公开 SDK、回调分派、服务注册和管理路由：账号可走 host.auth.save，模型定价可走 settings.pricing / preview_pricing_sync / sync_pricing / update_pricing，UA 可走 request 中间件。目前仍没有找到外部用量补入和统计修正的公开写入入口。如果已有受支持的路径，请提供合同和调用示例；若尚无，希望补充可供插件调用的类型化服务或管理 API。
>
> 需要覆盖两种输入：保留来源时间和统计粒度的历史明细/日聚合，以及不要求模型、费用、耗时的 token 与请求数修正量。数据应能影响 CPR 原生总量与趋势，并保留来源/批次身份用于幂等去重。未知费用、模型和耗时保持未知，耗时均值继续使用真实样本；减量校验和是否影响 Client Key 当前预算需要明确合同。
>
> 希望写入返回可查询的批次或修正回执，以便超时后确认结果，避免重复累计；撤销能力也希望一起讨论。插件负责来源解析、映射、预览和警告交互，不要求宿主直接解析 CCS 文件。

以上是沟通诉求，不是现有接口名称或已确定的宿主实现方案。全局价目已满足当前定价接入的基本能力；如之后确认需要各站点独立定价，再单独讨论该范围。

## 本次验证记录

- 使用 `git ls-remote --tags` 确认 v3.26.1 标签及解引用源码提交；发布页也指向 5358de5
- 阅读该版本 SDK 能力、账号合同、middleware 头类型、settings 操作、HTTP 分派、宿主回调、服务登记、管理根路由、观测路由、API 和打包 CLI 文档
- 未拉取或修改本机 CPR 参考仓库，没有安装依赖、运行测试、启动应用或调用真实上游
- 本节记录合同核对阶段的检查；随后用户批准了实现计划，首版源码已编写，目标宿主运行验收仍未进行，当前实施状态见 `implementation-plan.md`
