# CPR 关键写入能力核对

核对日期：2026-10-12。源码基线为 Codex Proxy RS `v3.25.0`，提交 `05ba116eef849a0f4fa2d1940be82182bb2f7683`。本次重新查询远程 `main`，其 HEAD 与该提交一致；没有更新项目基线。

这是公开合同、回调分派、服务注册与业务实现的源码核对。未连接用户的 CPR 实例，未运行宿主集成测试，也未写入真实账号、统计或价格。用户安装版本和部署平台仍待确认。

## 结论

| 功能 | 公开写入能力 | 对本项目的影响 |
| --- | --- | --- |
| API 导入 | 已有 `host.auth.save` | 可开发来源适配、预览和幂等导入；协议映射与真实调用仍需验收 |
| 历史用量导入 | 未找到外部历史明细或聚合补入入口 | 需要 CPR 提供受支持的写入合同并让原生统计读取这些数据 |
| 手动调整 | 未找到 token 与请求数的修正入口 | 需要 CPR 提供允许缺少模型、费用、耗时的统计修正合同 |
| 模型定价 | 已有公开 `settings` 服务 | 可直接开发读取、同步及批量维护，无须先新增定价写入入口 |

因此当前需要向 CPR 开发者沟通的核心缺口是外部用量补入与统计修正。前期“定价管理尚未确定插件接入路径”的记录已由本次核对更新。

## 账号导入：已有合同

SDK 提供 `host.auth.list/get_runtime/get/save`。新建账号使用 `AuthSaveRequest::Create`，Provider 显式指定，账号 ID 由宿主生成；替换携带精确 `credential_revision`。凭据经二进制 RPC 载荷传递。

这说明账号写入入口存在，不代表所有 CCS 配置都已兼容。来源协议、URL、Key 和必要凭据事实仍需要映射，并在目标宿主验证调用。

证据：[账号合同](https://github.com/zyycn/codex-proxy-rs/blob/05ba116eef849a0f4fa2d1940be82182bb2f7683/backend/crates/gateway-plugin/sdk/src/call/host.rs#L77-L150)、[宿主账号回调分派](https://github.com/zyycn/codex-proxy-rs/blob/05ba116eef849a0f4fa2d1940be82182bb2f7683/backend/crates/gateway-plugin/runtime/src/callback/mod.rs#L485-L492)。

## 模型定价：公开服务可以主动调用

插件后端通过 `call.host.service::<O>(input)` 调用 `call::services::settings` 的类型化操作：

| 操作类型 | 服务名称 | 用途 |
| --- | --- | --- |
| `Pricing` | `settings.pricing` | 读取内置、同步与人工价目 |
| `PreviewPricingSync` | `settings.preview_pricing_sync` | 获取宿主定价来源的同步预览 |
| `Sync` | `settings.sync_pricing` | 确认同步选中的模型 |
| `Update` | `settings.update_pricing` | 替换价格、设置倍率、重置覆盖或删除非内置价目 |

已核对完整分派链：SDK 类型化方法 → `host.services.call` → 宿主公开服务注册表 → `SettingsService` → 原生存储与配置发布。网关启动时绑定该注册表。页面仍只调用插件自己的管理路由，由插件后端发起服务调用，不需要获取管理员 Cookie。

证据：[SDK 操作合同](https://github.com/zyycn/codex-proxy-rs/blob/05ba116eef849a0f4fa2d1940be82182bb2f7683/backend/crates/gateway-plugin/sdk/src/call/services/settings.rs#L214-L319)、[类型化调用](https://github.com/zyycn/codex-proxy-rs/blob/05ba116eef849a0f4fa2d1940be82182bb2f7683/backend/crates/gateway-plugin/sdk/src/client/middleware/service.rs#L103-L111)、[宿主回调](https://github.com/zyycn/codex-proxy-rs/blob/05ba116eef849a0f4fa2d1940be82182bb2f7683/backend/crates/gateway-plugin/runtime/src/callback/services.rs#L47-L88)、[服务注册](https://github.com/zyycn/codex-proxy-rs/blob/05ba116eef849a0f4fa2d1940be82182bb2f7683/backend/crates/gateway-admin/src/public_service/settings.rs#L58-L75)、[启动绑定](https://github.com/zyycn/codex-proxy-rs/blob/05ba116eef849a0f4fa2d1940be82182bb2f7683/backend/apps/gateway/src/bootstrap/server.rs#L145-L148)。

现有定价合同的边界：

- 价目以 Provider 和精确上游模型 ID 为键；当前原生 Provider 为 `openai` 与 `xai`，作用域为全局价目，不是每站点或每账号独立价格
- 更新校验允许新增非内置模型 ID，需要提供 `standard` 价格；没有要求模型名必须属于 GPT 系列。这不等于承诺相应上游协议和模型均可调用或完整估价
- 单次更新接受 1～500 个模型，应用同一个 `change`；不同模型的不同单价需要分组或逐项提交，跨请求不构成一个原子事务
- 内置模型不能删除，可以重置人工覆盖；非内置模型可删除
- 宿主 models.dev 同步只接受可表示的 OpenAI/xAI 文本价目。其他品牌的价格可以由插件从已选来源读取并映射到实际 Provider 和模型 ID，再通过 `Update` 保存；来源与映射规则仍需设计
- 改价影响后续本地估算及相应限额，不自动重算历史费用
- SDK 的 Rust/Serde 数据合同与管理 HTTP 的 `{ action: ... }` JSON 是不同边界，不能把 HTTP 请求结构直接当作类型化服务输入

证据：[原生写入与配置发布](https://github.com/zyycn/codex-proxy-rs/blob/05ba116eef849a0f4fa2d1940be82182bb2f7683/backend/crates/gateway-admin/src/use_case/settings.rs#L142-L303)、[定价 API 及同步边界](https://github.com/zyycn/codex-proxy-rs/blob/05ba116eef849a0f4fa2d1940be82182bb2f7683/docs/api.md#模型定价)。

## 历史用量和手动修正：未找到公开写入合同

对 SDK 的宿主资源合同、回调分派、公开服务注册表以及管理端路由进行了核对，未找到接受外部用量、日聚合或正负修正量的操作。原生用量路由提供记录、详情、汇总和洞察的 `GET` 查询，没有对应的导入或修正路由。

SDK 还支持 `HostClient::dispatch_http`，可以由后端使用宿主签发身份调用已存在的相对管理路由，不需要管理员 Cookie；但它不能补出尚不存在的用量写入业务操作。因此本次缺口不能仅通过换用 HTTP 分派解决。

已有相近能力不具备所需语义：

- `observer.request_completed` 投递真实请求完成事件，是有界实时观察，不能向宿主提交历史请求或修正量
- `host.state.*` 写入插件私有状态，不会把该状态合入 CPR 原生用量视图
- Client Key 预算接口修改日/周金额上限或重置当前窗口，不提供 token、请求数或历史统计的修正
- 模型执行回调会实际进入路由与计费链，不能用虚构执行代替历史数据补入

证据：[只读用量路由](https://github.com/zyycn/codex-proxy-rs/blob/05ba116eef849a0f4fa2d1940be82182bb2f7683/backend/crates/gateway-api/src/admin/observability/routes.rs#L7-L32)、[公开回调分派](https://github.com/zyycn/codex-proxy-rs/blob/05ba116eef849a0f4fa2d1940be82182bb2f7683/backend/crates/gateway-plugin/runtime/src/callback/mod.rs#L400-L503)、[公开服务目录](https://github.com/zyycn/codex-proxy-rs/blob/05ba116eef849a0f4fa2d1940be82182bb2f7683/backend/crates/gateway-admin/src/public_service/registry.rs#L24-L30)、[SDK 能力与 HTTP 分派](https://github.com/zyycn/codex-proxy-rs/blob/05ba116eef849a0f4fa2d1940be82182bb2f7683/backend/crates/gateway-plugin/sdk/docs/capabilities.md#公开服务)。

## 补接口时的统计口径

当前指标实现会从请求事实累计请求数和 token，并分别维护首字耗时的总和、样本数与分布。缺失耗时不会成为零毫秒样本。因此“增加请求总数就必然算错首字均值”不能作为现有实现的结论。

新增外部统计和手动修正时，可让这些数字进入总量和趋势，保留真实耗时样本的均值。缺少模型或费用的部分保持未知，并显示统计覆盖范围；用户提交前仍看到已确定的提醒。具体合并规则、费用覆盖显示与减量边界由新宿主合同确定，不在插件中假造明细。

证据：[请求计数累加](https://github.com/zyycn/codex-proxy-rs/blob/05ba116eef849a0f4fa2d1940be82182bb2f7683/backend/crates/gateway-store/src/postgres/observability/analytics/metrics.rs#L105-L118)、[耗时采样](https://github.com/zyycn/codex-proxy-rs/blob/05ba116eef849a0f4fa2d1940be82182bb2f7683/backend/crates/gateway-store/src/postgres/observability/analytics/metrics.rs#L187-L212)、[独立样本计数](https://github.com/zyycn/codex-proxy-rs/blob/05ba116eef849a0f4fa2d1940be82182bb2f7683/backend/crates/gateway-store/src/postgres/observability/analytics/metrics.rs#L272-L303)。

## 可以发给 CPR 开发者的说明

> 我在开发 cpr-data-toolkit 插件，希望把 CC Switch 等来源的历史 token 与请求统计去重后补入 CPR 原生用量统计，并允许用户手动填写 token、请求次数的正负修正量。
>
> 已核对 v3.25.0（05ba116）的公开 SDK 和路由：账号可走 host.auth.save，模型定价可走 settings.pricing / preview_pricing_sync / sync_pricing / update_pricing；这两项已有接入能力。目前没有找到外部用量补入和统计修正的公开写入入口。如果已有受支持的路径，能否提供合同和调用示例；若尚无，希望补充可供插件调用的类型化服务或管理 API。
>
> 需要覆盖两种输入：保留来源时间和统计粒度的历史明细/日聚合，以及不要求模型、费用、耗时的 token 与请求数修正量。数据应能影响 CPR 原生总量与趋势，并保留来源/批次身份用于幂等去重。未知费用、模型和耗时保持未知，耗时均值继续使用真实样本；减量校验和是否影响 Client Key 当前预算需要明确合同。
>
> 希望写入返回可查询的批次或修正回执，以便超时后确认结果，避免重复累计；撤销能力也希望一起讨论。插件负责来源解析、映射、预览和警告交互，不要求宿主直接解析 CCS 文件。

以上是沟通诉求，不是现有接口名称或已确定的宿主实现方案。全局价目已满足当前定价接入的基本能力；如之后确认需要各站点独立定价，再单独讨论该范围。

## 本次验证记录

- 查询上游远程 HEAD，与本地提交一致；`git describe --tags --exact-match HEAD` 返回 `v3.25.0`
- 读取公开合同、SDK 客户端、宿主回调、服务登记、启动绑定、原生定价实现、用量路由与统计采样实现
- 阅读现有服务集成测试代码，其中包含 `settings.update_pricing` 的主动分派与业务校验；本次未运行这些测试
- 参考仓库仅扩展稀疏检出的阅读范围，业务文件没有修改
- 尚未在用户目标宿主安装插件、验证真实保存/读取或完成四项功能的端到端验收
