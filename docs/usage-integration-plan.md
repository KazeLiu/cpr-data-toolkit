# 0.6 原生用量接入

## 目标与合同

SDK 和官方打包 CLI 固定到 CPR v3.29.4（`bad2a245ab3522f8332d39b0c4ea3d7fb6a61f43`），使用 RPC v4、management v2 和 middleware v5，最低宿主 v3.29.0。公开合同见宿主 `backend/crates/gateway-plugin/sdk/docs/usage-data.md`

## 执行顺序

- [x] 升级 SDK、清单、工具链与官方打包 CLI
- [x] CCS 有效明细及日聚合转换为统计贡献，保留时间、模型、缓存口径及来源费用
- [x] 导入使用稳定键覆盖，按来源与筛选范围核对，分批提交，明确失败停止并保留进度
- [x] 接通手动 Token／请求数修正，固定归属时间与操作 ID，响应丢失后同 ID 覆盖
- [x] 按选定时间范围和 Provider 逐页试算并写回历史费用，保留未知项及跳过原因
- [x] 更新使用说明，完成前端生产构建、两个 Linux 平台 release 构建与官方打包

## 数据与失败处理

CCS 没有完整的宿主请求上下文，使用 `usage.aggregates.write`，不生成请求或 Key。日聚合按来源 IANA 时区解释，只有完整日期被筛选范围覆盖时才纳入，明细与日聚合日期交叠时拒绝导入。稳定来源标识区分不同 CCS 安装；筛选预览固定同一任务。当前覆盖方式按宿主贡献归属时间替换所选时段内本来源全部旧贡献，再导入筛选结果，不限旧贡献的应用和模型；范围外、CPR 原生请求和其他来源保留。跨批次并非事务，中断后继续同一任务。筛选和疑似重复规则见 [CCS 筛选导入计划](ccs-filtered-import-plan.md) 与 README

手动修正只改变首次提交时刻的总量，不猜测费用、模型和成功率。负修正在尚未写入该操作时核对归属 15 分钟桶原值；确定拒绝与结果未确认分别处理。历史费用同步仅处理原生已结束请求，试算不能确定的项跳过，不填零，不改预算

## 验证与交付

阅读调用关系和最终差异；用户授权构建与打包，不新增或运行测试，不安装插件、不写本地真实用量。产物为 Linux ARM64 和 x86_64 的 `.tar.gz`、官方 `.sha256` 及含安装说明的 ZIP。用户自行安装并验收真实数据

Rust 1.99.1 的 ARM64 链接参数需要 cargo-zigbuild 0.23.4，已更新本机项目工具和 CI 固定版本；Zig 继续使用 0.14.1。对应上游修复见 [cargo-zigbuild #452](https://github.com/rust-cross/cargo-zigbuild/pull/452)

### 已完成的构建与包检查

- `pnpm --dir web build`：类型检查与生产构建通过
- `cargo zigbuild --release --locked --target aarch64-unknown-linux-gnu.2.17`：通过
- `cargo zigbuild --release --locked --target x86_64-unknown-linux-gnu.2.17`：通过
- 固定 v3.29.4 的官方 `cpr-plugin package`：两个平台通过
- 安装包结构检查：版本 0.6.0、最低宿主 3.29.0、management v2、middleware v5、两个实际 ELF 架构、glibc 2.17、后端执行权限和三份页面资源均符合清单
- 两份 ZIP 均包含各自原生包、官方校验文件和安装说明，CRC 检查通过
- `git diff --check`：通过

产物在 `dist/0.6/`：`cpr-data-toolkit-0.6-linux-arm64.zip`（2534581 字节）与 `cpr-data-toolkit-0.6-linux-x86_64.zip`（2695196 字节），同目录提供可直接上传的 `.tar.gz` 和官方 `.sha256`

未完成全量 ESLint／Clippy、行为测试、浏览器交互或本地宿主数据验收。构建和包检查不覆盖这些验收项；真实导入总量、重复导入、手动修正及费用写回由用户安装后确认
