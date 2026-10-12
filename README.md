# CPR 数据工具箱

项目标识：`cpr-data-toolkit`。拟为 Codex Proxy RS 开发的历史数据补入与维护插件，开发目录为 `D:\Code\cpr-data-toolkit`，导入来源可扩展。

当前已完成参考项目的文档与关键源码阅读，尚处于功能讨论阶段。这里还没有业务代码、依赖安装或可安装插件包。

阅读入口：

- `AGENTS.md`：已核对的接入合同、数据边界和验证约定
- `docs/requirements-discussion.md`：当前四项需求与尚待决定的口径
- `docs/initial-research.md`：固定源码基线与初次调研；其中候选功能以最新需求讨论为准

当前功能范围：

- API 导入：从 CCS 导入站点连接信息至 CPR 账号管理，来源适配可扩展
- 用量导入：导入去重后的 CCS 历史用量，补足迁移到 CPR 前的统计
- 手动调整：填写 token 和请求次数的增减量，提交前提醒派生指标可能不一致
- 模型定价：同步并批量维护模型价目，覆盖 OpenAI 兼容接口上的其他模型

管理页面采用上述四个标签，需求说明以 `docs/requirements-discussion.md` 为准。定价作用域和宿主接入方式仍待讨论。
