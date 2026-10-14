# CPR 数据工具箱

Codex Proxy RS 的连接导入、统计预览和请求身份维护插件，项目标识为 `cpr-data-toolkit`，开发目录为 `D:\code\cpr-data-toolkit`。

项目包含 Rust 后端和 Vue 管理页，SDK 固定到 CPR v3.26.1。当前版本为 **0.5**（包内 `0.5.0`），新增社区价目过滤和历史费用同步选项；历史费用同步确认后提示「上游接口未开放」，不写回历史记录。安装入口见 `docs/installation.md`。

阅读入口：

- `AGENTS.md`：已核对的接入合同、数据边界和验证约定
- `docs/implementation-plan.md`：已批准的功能口径、实现进度和验收边界
- `docs/requirements-discussion.md`：当前五项需求与已确定的功能边界
- `docs/initial-research.md`：固定源码基线与初次调研；其中候选功能以最新需求讨论为准
- `docs/write-capability-audit.md`：关键写入能力核对、源码证据与发给 CPR 开发者的说明

## 五个标签页

- **API 导入**：解析 CCS 完整手动 SQL 导出，提取 Claude、Codex 和 Gemini 的名称、地址与 Key，选择后通过宿主账号合同接入 OpenAI Responses 兼容上游，不按来源协议拦截。表格支持多选、跨页全选、复制 API，以及主动查看或复制隐藏的 Key。相同地址与 Key 只接入一次，名称不同在弹窗中选择；更新已有账号使用精确凭据版本
- **用量导入**：按 CCS 有效用量规则去除代理／会话重复，归一缓存语义，展示来源模型、时间、Token 与原始 USD 费用。明细和日聚合分开，不在缺少来源时区时合并总量。确认导入仅提示「上游接口未开放」
- **手动调整**：填写 Token 总量和请求次数的正负增减量，读取今天／近 7 天／近 30 天的真实原值并预览。确认提醒派生指标可能不一致，随后提示「上游接口未开放」，保留输入且不修改统计
- **模型定价**：读取 CPR 全局价目、预览并确认宿主同步、批量维护标准价；价目表支持全选当前筛选结果，社区和同步表支持全选本页。models.dev 社区价目支持关键词及模型 ID 过滤，核对实际上游模型 ID 后写入，缓存价格缺失或留空按零处理，输入与输出价格必填。「价格同步」提供补全缺失费用或覆盖全部历史费用的选项，默认补全，确认仅提示「上游接口未开放」
- **模型UA定制**：忽略大小写，按顺序使用第一条命中的启用规则覆盖 `User-Agent`；可选配套 `originator`、`version`，未填写的头不变更，未命中不改写。预览和安全验证用绿／红色块显示结果；请求改写日志展示时间、模型、命中规则及替换前后 UA

## 配置模型 UA

安装并启用插件后，在 CPR 为本插件启用 `middleware` 的 `request` 绑定，建议限定到个人反代使用的 Client Key。空绑定范围表示全局；只保存规则但没有启用绑定，不会处理正常模型请求。

1. 打开「模型UA定制」，新增 `grok`、`gemini` 等包含规则并填写实际 UA
2. 将更具体的规则放在前面，保存后新请求使用新规则
3. 「匹配预览」检查当前编辑，「安全验证」选择 Key 并读取可见模型，检查已保存规则
4. 从客户端发送真实模型请求，再点击「请求改写日志」的「刷新日志」，检查是否进入插件以及使用了哪条规则

安全验证共用正式中间件的请求头改写逻辑，覆盖到插件请求出口，**不调用真实 `/v1/responses`，不发送上游推理请求**。它不覆盖真实入口鉴权、宿主绑定筛选、账号选择、默认画像组装和最终网络传输；模拟入口 UA 可选，留空时不推测真实 Key 默认 UA。最终 HTTP／WebSocket 出站头仍需在目标宿主确认。

### 用日志定位 UA 未生效

日志只记录实际进入 request 中间件的请求，预览与安全验证不产生记录。时间按浏览器本地时区显示，规则栏区分「已改写」「未命中」「模型缺失」「规则读取失败」；下游调用报错时额外显示「下游调用失败」。配套头发生变化时，可在前后 UA 栏展开查看 `originator`、`version`。

替换前是插件入口头，不是宿主根据账号和 Key 画像生成的默认头；替换后是本插件交给宿主的头。如果日志显示「已改写」而上游仍收到其他 UA，应继续核对后续中间件和最终出站头。如果发送请求后刷新仍无记录，先检查插件启用状态、request 绑定和 Client Key 范围。

日志在插件进程内存保留最近 200 条，每页 25 条，可主动清空，重启后清空。只保存模型、请求关联 ID、规则及三个身份头，不保存 Authorization、Cookie、请求正文或 API Key；每个身份头最多采集 4096 字节，超出会标注「日志截断」，实际请求头不受日志截断影响。

## 导入来源与保存结果

1. 选择 CCS 完整 SQL 导出，在「API 导入」表格查看来源名称、应用和 API 地址
2. 点击 API 后的复制按钮可复制完整地址；Key 默认隐藏，点击眼睛查看，再次点击隐藏，旁边的复制按钮可直接复制 Key
3. 勾选来源或点击「全选」，再点击「一键接入 OpenAI Responses 兼容上游」；全选包含所有页，超过 100 个上游时按批顺序处理
4. 相同 API 与 Key 合并为一个上游，名称不同时在弹窗中选择；若匹配多个 CPR 账号，选择要更新的具体账号后确认接入

使用 CCS 的完整手动 SQL 导出，不使用云同步导出。当前适配 schema 20；未声明版本的导出须通过兼容结构检查。文件上限 64 MiB，按 256 KiB 分块上传；原始文件、临时 SQLite 库和解析结果只在后端内存暂存，闲置 15 分钟后清理，重启即清空。最多同时保留两个上传和两个解析来源，可点击「清理来源」提前释放。

0.3 补充 SQLite 创建索引所需的 `Reindex` 授权，仅限隔离内存库的 `main`；完整导出中的正常索引不再被该项白名单误拦，外部文件、附加数据库、扩展及其他未允许操作仍被拒绝。

预览只返回 Key 是否存在，主动查看或复制时才通过私有管理路由读取明文；管理响应禁用缓存，明文不写入日志或浏览器持久存储，隐藏、离开标签、重新预览、清理来源或完成接入后释放。接入直接使用来源地址，不改 API 路径或验证上游协议；缺少有效地址或 Key 的来源在表格中显示原因，OAuth 登录材料不能作为 Key 导入。

保存结果逐项显示；账号保存与来源映射不是同一事务，结果未知时停止后续项，先在宿主核对再重新预览，不自动重复创建。多个宿主账号匹配同组连接时必须选择具体宿主账号，其他账号不会被删除。

CPR v3.26.1 的历史用量补入、统计修正和历史费用重算仍等待公开写入合同，预览不会进入原生历史或预算。停用插件只停止后续 UA 改写，已经保存的账号和价格不会自动撤销。

## 筛选社区价目与历史费用选项

在「models.dev 社区价目」输入名称、模型 ID 或价格关键词，也可在「社区模型 ID」表头输入包含条件，点击「筛选」或按 Enter 应用。两项条件同时生效，忽略大小写，在整个选定来源中过滤后再分页；翻页保持已应用的条件，点击「清空筛选」恢复目录。

社区价目按钮右侧的「价格同步」打开历史费用同步弹窗，每次打开默认选择「只补全缺失费用」，已有 0 费用不视为缺失；另可选择「覆盖全部历史费用」。当前确认只显示「上游接口未开放」，不请求费用写入或改变历史记录。

## 构建与打包入口

需要 Node.js 24 或更高、pnpm 12.6.0、Rust 1.97.0，以及宿主目标的 Rust target 和链接工具。前端 Git 依赖 `@codex-proxy/ui` 需要执行其构建脚本，已在 `web/pnpm-workspace.yaml` 明确允许；不要用 `--ignore-scripts` 安装后直接构建页面。

在项目根目录执行以下命令前，先确认宿主 target；开发电脑为 Windows 不代表宿主也是 Windows。

```powershell
pnpm --dir web install --frozen-lockfile
pnpm --dir web build
$target = Read-Host 'CPR 宿主的 Rust target'
cargo build --release --locked --target $target
```

打包 CLI 同样固定到 v3.26.1 的提交：

```powershell
cargo install --locked --git https://github.com/zyycn/codex-proxy-rs.git --rev 5358de53ddd4bda529cb63c422fd5c6af070f9c9 codex-proxy-plugin-cli --root .tools/plugin-cli
.tools/plugin-cli/bin/cpr-plugin package --manifest plugin.json --binary "target/$target/release/cpr-data-toolkit" --target $target --resource-map web=web/dist --output-dir dist
```

该版本 CLI 支持 `x86_64-unknown-linux-gnu`、`aarch64-unknown-linux-gnu` 和 `aarch64-apple-darwin`，没有 Windows 原生包目标。资源映射将 `web/dist` 中的三个页面文件放入包内 `web/`；输出包位于 `dist/`。详见[固定版本打包合同](https://github.com/zyycn/codex-proxy-rs/blob/5358de53ddd4bda529cb63c422fd5c6af070f9c9/backend/apps/plugin-cli/README.md)。

### 本机 Windows 交叉构建

本次准备了项目内 Node 24.21.0、pnpm 12.6.0、cargo-zigbuild 0.21.7、Zig 0.14.1 和官方打包 CLI，并安装 Rust 1.97.0 的 ARM64 Linux target。工具位于忽略的 `.tools/`；在本机复用这些工具时，从项目根目录执行：

```powershell
$env:Path = "$PWD/.tools/node-v24.21.0-win-x64;$PWD/.tools/cargo-zigbuild/bin;$env:Path"
.tools/node-v24.21.0-win-x64/node.exe .tools/pnpm/node_modules/pnpm/bin/pnpm.mjs --dir web build
$env:CARGO_ZIGBUILD_ZIG_PATH = "$PWD/.tools/zig-pypi/ziglang/zig.exe"
$env:CARGO_ZIGBUILD_CACHE_DIR = "$PWD/.tools/zig-cache"
cargo +1.97.0 zigbuild --release --locked --target aarch64-unknown-linux-gnu.2.17
.tools/plugin-cli/bin/cpr-plugin.exe package --manifest plugin.json --binary target/aarch64-unknown-linux-gnu/release/cpr-data-toolkit --target aarch64-unknown-linux-gnu --resource-map web=web/dist --output-dir dist
```

锁文件已由 pnpm 12.6 生成并保留发布年龄策略；`allowBuilds` 只允许固定提交的 UI Git 依赖执行准备脚本。安装包已核对为 ARM64 ELF、glibc 2.17，后端执行权限和三份页面资源完整。

### 交付与版本递增

0.5 提供 Linux x86_64、Linux ARM64 和 macOS ARM64 的原生 `.tar.gz`、官方 CLI 生成的 `.sha256` 以及 ZIP。ZIP 内含对应原生包、校验文件和 `INSTALL.md`，解压后在 CPR 插件管理页上传 `.tar.gz`。平台选择见 `docs/installation.md`，中文发布说明见 `docs/releases/0.5.md`。

以后每次交付新包增加 `0.1`，下次为 `0.6`（内部 `0.6.0`）；打包前同步 `plugin.json`、`Cargo.toml`、`web/package.json` 和 Cargo 锁文件中的本项目版本。失败修复及同一次打包重试不重复递增，约定见 `AGENTS.md`。

多平台构建使用 `.github/workflows/package.yml`，在 GitHub Actions 手动运行「Build release packages」或使用 `gh workflow run package.yml --ref main`。工作流只构建产物，不自动发布；前端资源构建一次后供三个平台使用，Rust、SDK、CLI 和构建工具版本固定，Linux 使用 glibc 2.17 目标。下载对应构建产物后，通过 GitHub CLI 将安装包附到 Release。

0.5 已完成本机前端类型检查与生产构建、Linux ARM64 release 构建和包内文件检查；本次多平台发布进度见 `docs/implementation-plan.md`。
