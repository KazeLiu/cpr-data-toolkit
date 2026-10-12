# CPR 数据工具箱：初步调研与讨论材料

调研日期：2026-10-12。这里只记录本次阅读结果与候选方案，不代表业务需求已定稿。

后续讨论已明确四项范围：历史用量补入、手动增减 token 与请求次数、站点导入账号、批量模型定价维护。当前需求见 `requirements-discussion.md`；本笔记中独立用量分析页等早期候选不再作为功能目标，接口限制仍作为实现调研依据。

同日进一步核对已确认模型定价可经 SDK 的公开 `settings` 服务读取、同步及更新；历史用量补入与手动修正未找到公开写入入口。最新证据和开发者沟通草稿见 `write-capability-audit.md`。

## 阅读范围与固定基线

用户指定的两个项目已阅读 README、依赖配置及与本任务有关的关键源码。为确认数据来源和写入边界，补充核对了宿主 SDK 与 CC Switch 上游源码。

| 项目 | 本次检出的提交 | 重点阅读 |
| --- | --- | --- |
| [codex-proxy-plugins](https://github.com/zyycn/codex-proxy-plugins/tree/38fe532914f7332428db0eee14491f64ad998176) | `38fe532914f7332428db0eee14491f64ad998176` | 清单、后端处理器组合、管理注册、宿主回调、observer、页面桥和依赖 |
| [codex-proxy-ui](https://github.com/zyycn/codex-proxy-ui/tree/db615f3d72975e0d09847a07d03ed5fe7ba8c8b0) | `db615f3d72975e0d09847a07d03ed5fe7ba8c8b0` | v0.4.0 的公开入口、安装、主题与组件目录 |
| [codex-proxy-rs](https://github.com/zyycn/codex-proxy-rs/tree/05ba116eef849a0f4fa2d1940be82182bb2f7683) | `05ba116eef849a0f4fa2d1940be82182bb2f7683` | v3.25.0 的 SDK 能力、账号合同、运行时账号保存、API Key 凭据、主题和打包 CLI |
| [CC Switch](https://github.com/farion1231/cc-switch/tree/99277921d9363c78e656ada0ca3bf39e30938b23) | `99277921d9363c78e656ada0ca3bf39e30938b23` | Tauri API、数据库 schema、导出、缓存 token 归一、跨源去重、Codex 协议配置 |

未修改参考仓库业务文件。源码检出仅供阅读，没有安装或运行这些项目。未读取用户本机 CC Switch 的真实数据库、API Key 或 token。

## 插件接入的已确认事实

示例插件后端使用公开 Rust SDK，`PluginBuilder` 将清单能力与处理器绑定；管理页面通过 `window.codexProxyPlugin.request` 调用自己的已注册管理路由。前端为 Vue 3 SFC + TypeScript，使用 `@codex-proxy/ui`。

当前示例依赖 Rust 1.97、Node.js 24、pnpm 12.6，SDK 与打包 CLI 固定到宿主 v3.25.0，示例清单要求 `>=3.25.0, <4.0.0`。这些是参考基线，不代表新插件已经生成或验收了同样的清单。GitHub 主页面缓存曾显示旧的 3.20.0 要求，本笔记以本次检出的源码为准。[固定提交的 README](https://github.com/zyycn/codex-proxy-plugins/blob/38fe532914f7332428db0eee14491f64ad998176/README.md)

UI v0.4.0 提供表格、分页、表单、选择、卡片、对话框和反馈等公开组件。样式须显式导入；组件与主题不依赖宿主的 store、路由或 API。UI 目前通过 GitHub 分发，消费方应固定版本和锁文件；业务统计图如有需要，另行选择实现。[UI 安装说明](https://github.com/zyycn/codex-proxy-ui/blob/db615f3d72975e0d09847a07d03ed5fe7ba8c8b0/playground/guide/installation.md)

页面标题、主题和整页滚动由宿主承接，插件内容区不重复实现宿主外壳。[宿主主题约定](https://github.com/zyycn/codex-proxy-rs/blob/05ba116eef849a0f4fa2d1940be82182bb2f7683/docs/theme.md)

## “CC Switch API”的两种含义

暂按“导入 CC Switch 保存的供应商连接信息”理解用户需求：供应商名称、base URL、API Key、模型及协议设置。这与调用 CC Switch 自身管理 API 是不同任务。

CC Switch 的 `providersApi` 和 `usageApi` 在前端通过 Tauri `invoke` 调用 Rust 命令。已核对的本地代理路由以推理转发、健康检查和状态为主；未发现可供外部插件直接读取供应商和历史统计的公开 REST 路由，不能预设 `GET /api/providers` 或 `/api/usage` 存在。[供应商 API](https://github.com/farion1231/cc-switch/blob/99277921d9363c78e656ada0ca3bf39e30938b23/src/lib/api/providers.ts) · [用量 API](https://github.com/farion1231/cc-switch/blob/99277921d9363c78e656ada0ca3bf39e30938b23/src/lib/api/usage.ts) · [代理路由](https://github.com/farion1231/cc-switch/blob/99277921d9363c78e656ada0ca3bf39e30938b23/src-tauri/src/proxy/server.rs)

CC Switch 默认将供应商与用量保存在 `~/.cc-switch/cc-switch.db`，存储位置可以更改。供应商的主键为 `(id, app_type)`，配置在 `settings_config` JSON 中，还有独立候选端点表。[配置与存储说明](https://github.com/farion1231/cc-switch/blob/99277921d9363c78e656ada0ca3bf39e30938b23/docs/user-manual/zh/5-faq/5.1-config-files.md) · [数据库 schema](https://github.com/farion1231/cc-switch/blob/99277921d9363c78e656ada0ca3bf39e30938b23/src-tauri/src/database/schema.rs)

Codex 供应商也可能使用 Chat Completions 或 Anthropic 上游，不能只抽出 URL 和 Key 就承诺兼容。供应商的协议配置、TOML 中选中的 `model_provider` 与对应表需要实际解析。[Codex 适配器](https://github.com/farion1231/cc-switch/blob/99277921d9363c78e656ada0ca3bf39e30938b23/src-tauri/src/proxy/providers/codex.rs)

## 输入方式的可行性比较

以下均为候选方案，尚未决定第一版采用哪种。

| 方式 | 适用情形 | 需要处理的边界 |
| --- | --- | --- |
| 上传 CC Switch 一致性 `.db` 备份 | CC Switch 与宿主不在同一电脑，或希望一次迁移 | 上传容量、schema 兼容、敏感文件清理；不能直接复制运行中主库而漏掉 WAL |
| 上传 CC Switch 的完整 SQL 导出 | 使用 CC Switch 已有导出入口，跨设备迁移 | 在隔离临时库中限制 SQL 操作；大文件处理；保留秘密材料的边界 |
| 插件后端只读连接来源库 | 宿主能访问 CC Switch 数据目录，后续增量同步 | Windows 与 Linux 路径、Docker 挂载、只读事务、schema 变化与同步游标 |
| 本机辅助程序向远程插件推送脱敏或必要数据 | 宿主远程部署，但需要持续同步 | 额外安装、认证、连接管理与版本兼容，首版工作量明显增加 |

CC Switch 的完整手动 SQL 导出调用 `export_sql_string()`，会从一致性快照完整导出；用于 WebDAV 等同步的导出跳过 `proxy_request_logs`、`usage_daily_rollups` 和会话同步相关表。因此云同步文件不能等同于完整用量导出。[备份与导出实现](https://github.com/farion1231/cc-switch/blob/99277921d9363c78e656ada0ca3bf39e30938b23/src-tauri/src/database/backup.rs)

## 账号能否导入宿主

公开 SDK 提供 `host.auth.list/get_runtime/get/save`。新建时显式指定 Provider，由宿主生成账号 ID；替换必须携带精确的凭据 revision。原始凭据放在二进制 RPC 载荷中。宿主原生 Provider 当前为 OpenAI 与 xAI，不能把 CC Switch 的任意供应商类型直接作为同名宿主 Provider 创建。[SDK 能力合同](https://github.com/zyycn/codex-proxy-rs/blob/05ba116eef849a0f4fa2d1940be82182bb2f7683/backend/crates/gateway-plugin/sdk/docs/capabilities.md)

宿主的 OpenAI API Key 凭据包含 `schema_version`、`installation_id`、`api_key`、`base_url` 和传输设置；SDK 保存的是宿主凭据事实，不能直接塞入 CC Switch 原始 `settings_config`。映射和实际调用仍需开发阶段验证。[凭据类型](https://github.com/zyycn/codex-proxy-rs/blob/05ba116eef849a0f4fa2d1940be82182bb2f7683/backend/crates/providers/openai/src/credential/api_key.rs) · [账号保存实现](https://github.com/zyycn/codex-proxy-rs/blob/05ba116eef849a0f4fa2d1940be82182bb2f7683/backend/crates/gateway-plugin/runtime/src/callback/accounts.rs)

推论：以 Responses 兼容的 Codex URL + API Key 账号作为首版范围，能减少协议适配工作。OAuth、Claude、Gemini、Chat Completions、余额查询脚本是否支持，需要单独决定和验证，当前不承诺支持。

## 历史 token 消耗的数据与口径

本次 CC Switch 源码的 schema 版本为 20，但用户安装版本尚未核实，不能把当前源码结构当作用户实际库结构。

| 数据 | 来源 | 导入时应保留 |
| --- | --- | --- |
| 请求明细 | `proxy_request_logs` | request ID、应用、供应商、请求/实际/计价模型、token 分类、缓存语义、费用、时间、状态、数据来源 |
| 较旧用量 | `usage_daily_rollups` | 来源日期、聚合维度、请求/成功数、token 分类、费用与聚合粒度；不虚构逐请求详情 |
| 定价依据 | `model_pricing` 与供应商元信息 | 原始费率和倍率，仅在选择复算功能后设计导入 |
| 供应商余额或订阅配额 | 独立用量查询机制 | 与已消费 token、费用历史区分，不混为同一指标 |

Token 的新增输入语义来自 `input_token_semantics`：当前定义有旧版/未知、总输入包含缓存、新增输入三类。源应用先将输入归一为新增输入，再与输出、缓存读和缓存写相加。例如总输入 1000 已含缓存读 800、输出 200、缓存写 0时，总量应为 1200，而非 2000。[归一公式](https://github.com/farion1231/cc-switch/blob/99277921d9363c78e656ada0ca3bf39e30938b23/src-tauri/src/services/sql_helpers.rs)

CC Switch 会同时保存代理转发日志和本地会话记录，查询使用有效用量过滤规则排除部分重复；重复判定涉及应用、模型、时间窗口和各类 token。仅按 `request_id` 去重导入批次，仍可能把源库内的同一次消费算两遍。[有效用量过滤](https://github.com/farion1231/cc-switch/blob/99277921d9363c78e656ada0ca3bf39e30938b23/src-tauri/src/services/usage_stats.rs)

旧明细会按完整本地日聚合后清理；统计历史要同时考虑日聚合和仍保留的明细，并保留来源时区口径。日聚合表未保留明细的全部维度，不能伪造丢失的信息。[聚合与清理实现](https://github.com/farion1231/cc-switch/blob/99277921d9363c78e656ada0ca3bf39e30938b23/src-tauri/src/database/dao/usage_rollup.rs)

## 历史数据在宿主中的位置

已核对的公开 SDK 提供账号、额度事实、预算管理、私有状态与实时观察。`request_completed` 为有界实时事件，合同明确不提供历史使用记录查询；在这些公开能力及回调中，未找到外部历史用量写入原生账本的专用入口。

因此建议第一版把“CC Switch 导入历史”展示在插件扩展页，并保留来源标签。不要通过修改宿主数据库或伪造请求事件使它混入原生计费。若要求它出现在宿主原生用量页，需先讨论新增宿主接口的范围。[SDK 基础事实与实时观察边界](https://github.com/zyycn/codex-proxy-rs/blob/05ba116eef849a0f4fa2d1940be82182bb2f7683/backend/crates/gateway-plugin/sdk/docs/capabilities.md)

`host.state.*` 可以保存导入映射、批次、游标和有界统计，但不能据此假定适合无限增长的完整历史库。历史规模、保留期与插件自有存储方式尚待讨论。账号保存与插件状态写入不是一个事务，部分成功和结果未知必须设计恢复。

## 建议第一版功能，待确认

1. 来源检查：识别格式、schema、供应商数量、用量日期范围与不支持项
2. API 导入：先支持可映射的 Codex URL + API Key，展示脱敏候选和逐项导入结果
3. 冲突处理：区分新增、已导入、需更新与不兼容，重复导入不重复建账号
4. 历史用量：展示 token 分类、请求数、估算费用；按时间、供应商、模型筛选
5. 导入记录：查看批次和失败原因，能够重试明确失败项并处理结果未知

自动增量同步、供应商余额查询、OAuth 迁移、协议转换、告警和预算联动可以后续独立评估。

## 回来优先讨论的五件事

1. “API”是否指供应商地址与 Key；“token 消耗”是否还包含余额或订阅配额
2. Codex Proxy RS 部署在哪里：Windows、WSL、Docker、Linux 服务器或 macOS；CC Switch 数据在哪里
3. 首版只支持 Codex API Key，还是同时支持其他应用或 OAuth
4. 使用备份文件一次导入，还是要直接读取来源库并持续同步
5. 历史用量展示在插件页是否足够，还是必须并入宿主原生统计；保留明细还是仅保留汇总

官方插件打包 CLI 当前列出的目标为 Linux x86_64、Linux aarch64 和 macOS aarch64，没有 Windows 原生目标。`D:\Code` 可以作为开发目录，但安装包必须按宿主运行平台生成。宿主实际部署环境尚未核实。[打包 CLI 平台](https://github.com/zyycn/codex-proxy-rs/blob/05ba116eef849a0f4fa2d1940be82182bb2f7683/backend/apps/plugin-cli/README.md)

## 本次验证范围

- 已确认目录为新建目录，适用路径上未找到已有项目 AGENTS.md
- 已记录四个参考检出的完整提交，阅读了上述入口与关键合同
- 已创建初步 AGENTS.md、README、忽略规则和本笔记；写入后重新读取核对
- 尚未安装依赖、编译插件、读取真实来源数据、向宿主导入账号、运行真实模型或核验历史金额

下一步是确认功能范围和部署环境，再开始最小可安装原型。
