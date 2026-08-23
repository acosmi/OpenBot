# 术语表（`zh-CN` 首版）

本文件是术语表的**真源**（GUI 设计系统方案 §8.5）。
`docs/2026-08-22-OpenBot-GUI设计系统与视觉规格-方案.md` §8.5 的那张表是本文件的人类快照；
两者不一致时以本文件为准，并在同一个 PR 里把文档改回来。

标签：整张表 = **新增**。上游 `891df72f1827454d8b353d108fe5dd2313b7e30d` 的 `app/src`
零 i18n 框架（实跑 `grep -rlE 'useTranslation|i18next|react-intl|next-intl|<Trans\b' --include=*.tsx --include=*.ts . | wc -l` → `0`；
同形状命令换成 `useQuery` 在同一棵树上 → `38`，证明命令本身能命中），
英文文案以字面量散落在 JSX 里，没有可以 parity 的翻译目录。因此本表与
`en.json` / `zh-CN.json` 的全部键都是本项目新建，不是对上游目录的移植。

---

## 22 条术语

| en | zh-CN | 说明 |
| --- | --- | --- |
| Bot | Bot（不译） | 产品自身的名字，任何位置都不翻译 |
| Coworker | 同事 | 用户创建、可配技能的智能体；界面上一律「同事」，不用「代理」「智能体」 |
| Channel | 频道 | 一个协作空间 |
| Thread | 会话 | 频道内的一次完整对话 |
| Run | 运行 | 一次工具 / 技能的执行 |
| Tool | 工具 | 可被调用的能力单元 |
| Skill | 技能 | 用户可编辑、可分配给同事的工具封装 |
| Plugin | 插件 | 第三方扩展包 |
| Connector | 连接器 | 连接外部服务（如 Google Drive）的组件 |
| Computer | 计算机 | 同事可操作的机器；不用「电脑」「桌面」 |
| Deployment | 部署 | |
| Boundary | 边界 | 工具调用的权限 / 数据边界 |
| Credential | 凭据 | 不用「证书」「密钥」（密钥另有其词） |
| Identity provider | 身份提供方 | 不用「身份供应商」 |
| People | 成员 | admin 里的人员列表；不用「人员」「用户」 |
| Grant | 授权 | 名词与动词同形 |
| Approval | 审批 | 不用「批准」（那是动作 Approve） |
| Audit | 审计 | |
| Component | 组件 | 同事渲染出的 UI 组件 |
| Gallery | 组件库 | 组件的浏览页；不用「画廊」 |
| Memory | 记忆 | 不用「内存」 |
| Tenant package | 租户包 | |

---

## 改动走 PR

- 本表任何一行的改动都必须走 PR，并在同一个 PR 里同步：
  1. 本文件；
  2. `zh-CN.json` 里所有用到该词的值；
  3. GUI 方案 §8.5 的快照表。
- 新增术语的判据：**同一个英文名词在两个以上界面位置出现，且译法不唯一**。
  只出现一次的词不进表，直接在 `zh-CN.json` 里定。
- 与本表冲突的译法在 review 阶段驳回，不做「这里例外一下」。

## 与 locale 文件的关系

- `en.json` 是源语言真源；`zh-CN.json` 的键集合必须与它**逐字相等**，占位符集合逐键相等。
  闸门 = `xtask i18n-check`（GUI 方案 §8.3；`leptos_i18n_build` 对缺键只发 cargo warning，
  且 `-D warnings` 管不到 build script 的 warning，所以必须自建这道闸门）。
- 键 `snake_case`，句子级而非单词级：`channels.composer_send` 而不是 `common.send`，
  除非真是通用词。
- 禁止字符串拼接组句，用占位符 `{name}`。
- 首版没有使用 ICU 复数形式（`plurals` 特性已在 §8.1 打开，但本版所有带 `{count}`
  的键都是单一形态）。引入复数键时要同 PR 扩 `xtask i18n-check` 的占位符比对规则。

## 不翻译的东西（§8.7）

模型可见文本（tool description、系统提示）、审计记录正文、日志、API 错误 `code`、
租户包 YAML 键、组件参数 schema。它们是协议不是界面，不进 locale 文件。
`errors.code_label` 翻译的是「错误码」这个**标签**，`code` 值本身原样显示。
