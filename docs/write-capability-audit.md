# CPR 关键写入能力核对

## 基线与结论

当前基线为 CPR **v3.29.4**，SDK 与官方 CLI 固定到 `bad2a245ab3522f8332d39b0c4ea3d7fb6a61f43`。本机参考仓库按用户授权切换到 `main` 并拉取；公开 v3.29.4 与参考仓库的 SDK 业务合同及 CLI 一致。v3.29.0～v3.29.4 本插件使用的用量、请求中间件与会话合同没有差异，最低宿主设为 v3.29.0

| 功能 | 公开合同 | 插件接入 |
| --- | --- | --- |
| API 导入 | `host.auth.save` | 保留创建、精确 revision 替换和来源映射 |
| 历史用量 | `usage.aggregates.list/write` | CCS 有效明细和日聚合按稳定来源键写入原生统计 |
| 手动调整 | `usage.aggregates.write` | 有符号 Token／请求数贡献，操作状态保存在插件私有状态 |
| 模型定价 | `settings.pricing/preview_pricing_sync/sync_pricing/update_pricing` | 保留全局价目读取、同步和批量维护 |
| 历史费用 | `usage.records.list`、`pricing.evaluate`、`usage.records.write` | 按范围逐页试算后更新费用，不能确定的记录跳过 |
| 模型UA定制 | request middleware v5 | 保留增量头改写与有界日志 |

源码入口：[用量合同](https://github.com/zyycn/codex-proxy-rs/blob/bad2a245ab3522f8332d39b0c4ea3d7fb6a61f43/backend/crates/gateway-plugin/sdk/docs/usage-data.md)、[类型化操作](https://github.com/zyycn/codex-proxy-rs/blob/bad2a245ab3522f8332d39b0c4ea3d7fb6a61f43/backend/crates/gateway-plugin/sdk/src/call/services/usage/operations.rs)、[公开服务注册](https://github.com/zyycn/codex-proxy-rs/blob/bad2a245ab3522f8332d39b0c4ea3d7fb6a61f43/backend/crates/gateway-admin/src/public_service/usage_data.rs)

## 用量归一与筛选范围同步

- CCS `input_tokens` 先按来源语义归一为新增输入，再加缓存读写形成宿主含缓存输入；总量只加一次缓存，未知指标不写成零
- 请求明细保留 CCS 原始秒级时间；来源时区默认 Asia/Shanghai，可修改，表单起止时间和日界线均按此 IANA 时区解释。日聚合仅在完整日期被覆盖时纳入
- 同一应用、供应商和来源日期的明细与日聚合交叠时拒绝导入；源内代理／会话重复沿用已核对的 CCS 有效用量规则
- 稳定来源标识区分不同 CCS 安装，记录键由应用、供应商及明细 ID 或日聚合主键生成 UUID v5；宿主键带插件与来源前缀。该 UUID 用于业务同步身份，不依赖整份文件摘要
- 从完整来源筛选应用、展示模型及时间，先过滤后分页。预览冻结变更计划，确认和继续使用同一快照；筛选条件不进入记录键，相同键覆盖而不累计
- 管理员确认覆盖后，按宿主 `bucket_start_ms` 替换所选时段内当前 CCS 来源的全部旧贡献，再导入筛选结果；应用和模型仅筛选新数据，不限制旧贡献清理。来源前缀区分不同安装，不需要反解旧键的应用和粒度；范围外、CPR 原生请求和其他来源保留，时间留空表示本来源全部历史
- 原生疑似重复通过公开 ListRecords 分页核对，仅双方唯一对应时排除并清理原来源贡献，不修改原生请求。当前 SDK 按开始时间过滤，实际时间比较使用 completed_at_ms；较早开始的长请求不推断重复，范围及排除限制见 README
- CCS 缺少完整宿主请求上下文，使用统计贡献而不伪造原生请求；不猜测 Provider、账号、Key、操作、传输或延迟样本。缺少的维度不参与相应筛选

单次写入上限为 500 项、2 MiB，单批事务任一项失败则回滚；本插件每次写 100 项。跨批次不是事务，中断期间总量可能不完整；继续同一任务或按同一来源标识重新准备同一范围可恢复。空筛选结果不执行清理

## 手动调整与恢复

修正归属首次提交时刻，只写 Token 总量与请求数。操作 ID、归属时刻和增减量先保存为 pending，再写宿主贡献；成功后标记 saved，确定拒绝时标记 rejected。响应未确认时保留原操作，重新进入页面可继续核对，相同完整键覆盖不会重复累计

负修正先查询相同操作是否已写入；未写入时读取归属 15 分钟桶的真实统计，拒绝超过该桶已有量的减量。没有模型、费用和耗时，不猜测成功数、不改变预算；派生指标与总量可能不一致

## 历史费用写回

每次按确定的起止时间、Provider、方式和游标读取 50 条原生请求，跳过运行中请求；补全模式只处理 `unavailable`，已知零费用保持不变。以原有 Token、模型和服务档位调用当前价目试算，仅将 `estimated` 的费用通过 `UsageRecordChange::Update` 写回

试算不能确定的 Provider、价格、Token 或图像用量返回跳过原因，不补零。计算只包含 Token 费用，覆盖会替换原费用并清除原计价快照，不计算工具等附加费，不重跑请求结算或修改 Key 预算。CCS 聚合贡献不参与该流程

查询游标不是跨调用快照，外部管理操作与本插件没有共同事务；同步期间不要同时修改同一批历史数据。响应丢失后停止后续页，管理员明确继续时重新核对当前页；已确认进度不把未知结果计为成功

## 协议与交付边界

新版宿主使用 RPC v4，管理能力 v2、请求中间件 v5，旧包不能只改版本号。0.6 用匹配 SDK 与 CLI 重新构建，保留 API 导入、定价和 UA 入口。CLI 仍支持 Linux x86_64、Linux ARM64、macOS ARM64；本次只交付用户选定的两个 Linux 平台

合同阅读与编译打包不等于宿主数据验收；本次不安装插件、不写真实数据，用户安装后验证来源总量、重复导入和原生页面展示
