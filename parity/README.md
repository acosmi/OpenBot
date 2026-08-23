# parity ledger —— schema、校验规则与用法

Phase 0 / Evidence Freeze 产物（v3 §19.3「Phase 0 必做产物」，v3 §24 的 **G0** 闸门）。

`parity/*.yaml` 是「上游有什么 → Rust 落在哪 → 谁负责 → 怎么验 → 现在做到哪」的
唯一台账。G0 的判据是 **「API/page/table/env/event/test parity ledger 未分类项 = 0」**；
DoD（v3 §25）另加一条：没有 parity ledger 100% 归类，不得宣称「全量完成」。

> 台账不是文档，是闸门的输入。`cargo xtask parity-check` 直接读它判红。

---

## 1. 文件清单

v3 §19.3 与 GUI 设计系统文档 §11 一共点名 9 份 ledger：

| 文件 | 覆盖面 | 真源章节 |
|---|---|---|
| `api.yaml` | HTTP API 面（server / supervisor / agent-computer 全部 handler） | v3 §15.1、§1.3 |
| `routes.yaml` | 前端 route 文件（页面 + layout） | v3 §3.1 |
| `tables.yaml` | PostgreSQL 表与 migration | v3 §14.2 |
| `env.yaml` | 环境变量三档 preserve / rename / remove | v3 §15.4 |
| `events.yaml` | AG-UI 与实时事件族 | v3 §7、§12 |
| `components.yaml` | Generative UI components 与外部扩展面 | v3 §3.3、§3.4 |
| `browser-operations.yaml` | Browser engine 操作面 | v3 §11.2 |
| `ui.yaml` | 21 原语 + 45 业务组件 + 47 图标 + 6 运行时库 + 27 页 | GUI 文档 §11 |
| `tests.yaml` | 上游测试 inventory（AST 级） | v3 §1.3、§21.1 |

`parity-check` 扫 `parity/` 顶层的**全部 `.yaml`**，不按名单白名单化：
名单外的文件照样被校验（只是会 warn 说 `schema` 不在已知名单里），
名单内缺席的文件不会被它发现——**缺文件由 G0 人工核对，不由本工具兜底**。
扩展名写成 `.yml` 的文件会被单独 warn 点名，避免「文件名写错所以没被校验」这种静默漏检。

---

## 2. Schema v1

顶层是一个 mapping，**恰好** 6 个键，不得自行增删：

```yaml
schema: api                 # 本 ledger 的名字，与文件名同源
schema_version: 1           # 整数 1
upstream_commit: 891df72f1827454d8b353d108fe5dd2313b7e30d
generated_by: manual        # manual | 脚本名
recount:                    # 至少一条
  - command: "grep -c '^  - id: ' parity/api.yaml"
    cwd: repo               # repo | upstream
    expect: 146
entries:
  - id: ...
```

### 2.1 `recount[]`

每条恰好三个键：`command` / `cwd` / `expect`。

- `cwd: upstream` = 上游只读克隆的根；`cwd: repo` = 本仓根。二选一，没有第三个值。
- `expect` 必须存在——**没有期望值的复算命令无法被重跑核对**，等于没有复算。
- 写进来之前必须**自己先跑一遍**并确认 `expect` 就是真实输出（CLAUDE.md §8）。

### 2.2 `entries[]`

9 个必填键 + 2 个可选键，别的键一律判红：

| 键 | 必填 | 含义 |
|---|:--:|---|
| `id` | ✅ | 文件内唯一的稳定标识，kebab-case |
| `upstream` | ✅ | 上游落点。**禁止裸行号**（不得以 `:<数字>` 结尾），用 `path::symbol` |
| `label` | ✅ | `parity` / `新增` / `替代` 三选一 |
| `target` | ✅ | Rust 侧确定落点（模块路径 / 类型 / 路由）。不许写 TBD |
| `owner` | ✅ | v3 §5.1 十个 crate 之一 |
| `test_id` | ✅ | `^T-[A-Z]+-[0-9]{4}$`，**全仓唯一** |
| `migration_rule` | ✅ | `preserve` / `rename` / `remove` / `n/a`，冒号后接说明 |
| `status` | ✅ | `todo` / `in_progress` / `done` |
| `evidence` | ✅ | 这条断言是**哪条命令**跑出来的 |
| `notes` | ⬜ | 补充说明 |
| `done_evidence` | ⬜ | **当且仅当** `status: done` 时存在且非空 |

`owner` 的十个合法值（`OWNERS` 常量）：
`openbot-contracts` · `openbot-domain` · `openbot-application` · `openbot-infra` ·
`openbot-agent` · `openbot-computer` · `openbot-server` · `openbot-ui` ·
`openbot-desktop` · `openbot-testkit`。

### 2.3 `label` 的三个值——这是 CLAUDE.md §4 的硬要求

- **`parity`**：上游已有这个行为，Rust 侧照做（语义可以被修正，见下）。
- **`新增`**：上游**没有**，是本次重写新加的。
- **`替代`**：上游有，但换了实现 / 换了协议 / 换了库。

> 把新增写成「当前行为」是 v2 审计里**最重的一类错误**（v3 §28.1 R1）。
> 所以 `LABELS` 是封闭三值域，中文原样，不接受大小写变体、英文译名或第四个值。

两条容易踩反的判据：

1. **v3 §2.4「不得照译」的上游缺陷仍然是 `parity`**——端点/行为存在于上游，
   只是语义被修正。这类条目必须在 `notes` 里写明「缺陷修正，不照译」。
2. **视觉不是 parity 对象**（CLAUDE.md §4a）。旅程 / route / 组件行为对上游 parity，
   外观是本项目自有设计系统，所以 `ui.yaml` 里大量条目天然是 `新增` 或 `替代`。

---

## 3. 八条校验规则

`cargo xtask parity-check` 强制的全部规则（真源 = `crates/openbot-testkit/src/bin/xtask.rs`
的 `RULES` 常量，违规时原样打印规则全文，不会只给一个编号）：

| # | 规则 | 为什么 |
|---|---|---|
| 1 | 除 `notes` / `done_evidence` 外每个键都必须存在且非空字符串（顶层键集合固定，不得自行增删） | 少一个键就是少一段契约；顶层键开放会让各 ledger 慢慢长成七种形状 |
| 2 | `label` 只能是 `parity` / `新增` / `替代` 三个值之一 | CLAUDE.md §4：把新增写成当前行为是最重的一类错误 |
| 3 | `owner` 必须是 v3 §5.1 十个 crate 之一 | 无主条目 = 没人会做；写错 crate 名 = 静默无主 |
| 4 | `status=done` **当且仅当** `done_evidence` 存在且非空（`status` 限 `todo` / `in_progress` / `done`） | v3 §19.3：CI 拒绝没有证据的 `done`。双向：非 done 带着 `done_evidence` 也判红 |
| 5 | `id` 在文件内唯一；`test_id` 在**全部** ledger 内唯一 | 重复 id 让「改哪一条」不可判定；重复 test_id 让两条不同契约共用一个验收点 |
| 6 | `test_id` 匹配 `^T-[A-Z]+-[0-9]{4}$` | 格式自由 = 无法机械关联测试与台账 |
| 7 | `upstream` 禁止裸行号（不得以 `:<数字>` 结尾） | CLAUDE.md §8：位置引用只用符号名。行号一次重构就全错 |
| 8 | `recount` 至少一条，且每条 `command` 非空（`cwd` 限 `upstream` / `repo`，`expect` 必填） | 计数必须能被一条命令复算 |

**规则 4 为什么要把 `status` 也封闭**：如果 `status` 可以是任意字符串，
「done ⟺ done_evidence」这条双向约束在 `status: blocked` 这种值上会**静默为真**，
于是一个既没证据又没做完的条目可以合法存在。封闭域是这条规则的承重结构，不是装饰。

### 3.1 不判红、只 warn 的四类

warn 不影响退出码，但会打印出来。它们不是硬规则，因为都属于「可能是有意为之」：

- `schema` 不在 v3 §19.3 + GUI §11 的 9 个 ledger 名单里；
- `upstream_commit` ≠ `891df72f1827454d8b353d108fe5dd2313b7e30d`；
- `target` 里出现 `TBD` / `未定`；
- `migration_rule` 的前缀不在 `preserve` / `rename` / `remove` / `n/a` 内；
- `recount` 条目里出现三个已定义键之外的键；
- `parity/` 下有 `.yml` 文件（只扫 `.yaml`，这份没被校验）。

---

## 4. 怎么跑

```bash
# 全量校验（退出码 0 = 通过，非 0 = 有违规）
cargo xtask parity-check

# 机器可读报告：每份 ledger 的条目数、status 分布、recount 条数、violations、warnings
cargo xtask parity-check --json
```

`cargo xtask` 是 `.cargo/config.toml` 里的 alias，展开为
`cargo run -p openbot-testkit --features xtask --bin xtask --`。
xtask 刻意**不是**第 11 个 crate（v3 §5.1 只允许四个理由建 crate，它一个都不满足），
落点是 `openbot-testkit` 的 bin target，`required-features = ["xtask"]`
让它对 `cargo build --workspace` 完全透明。

`parity/` 为空或不存在时 `parity-check` 优雅返回 0 并 warn「0 ledger」——
骨架 PR 自己跑 `cargo xtask ci` 时它就是空的，那不算失败。

**复算命令要自己跑。** `parity-check` 校验 `recount` 段的**结构**（有没有、
`command` 空不空、`cwd` 合不合法、`expect` 有没有），**不执行**那些命令：
它们的 `cwd: upstream` 指向一份不在本仓里的只读克隆，工具无从知道它在哪。
条目正确性由写台账的人现场跑 + 复核的人重跑双向保证。

---

## 5. 怎么加一条新条目

1. **先跑命令拿证据**，再写条目。顺序反了就会写出「应该 / 大概」。
2. 定 `label`。先问一句：**上游到底有没有这个东西？** 有 → `parity`；
   没有 → `新增`；有但换了实现/协议/库 → `替代`。拿不准就 grep 上游，
   grep 空要配正向对照（同一条命令在确实存在的地方能命中）。
3. 取 `test_id`：`T-<族>-<四位序号>`，族用大写字母（`T-API-0001` / `T-ROUTE-0032` /
   `T-UI-0107`）。**全仓唯一**，先在 `parity/` 里 grep 一遍确认没被占。
4. 写 `evidence`：这条断言是哪条命令跑出来的。写不出来说明还没有证据。
5. `status` 一律从 `todo` 起步。**只有拿到本轮实跑的通过证据才能改 `done`，
   并同时补 `done_evidence`。** 假 `done` 会被规则 4 与人工复核双向抓出来。
6. 如果这条改变了某个计数，**同 PR 更新 `recount` 段的 `expect`**，并把新命令跑一遍。
7. 跑 `cargo xtask parity-check`，绿了才提交。

### 5.1 三条最常见的错

- **把新增写成 parity**：最重的一类（v3 §28.1 R1）。判据 = 能不能在固定 commit 的
  上游克隆里 grep 到它。
- **`upstream` 写行号**：`server/src/app.ts:412` 判红。写 `server/src/app.ts::createApp`。
- **`status: done` 没有证据**：规则 4 直接红。没跑就写 `todo`，在 `notes` 里写清
  阻塞原因与解除条件——这比一个假 `done` 有用得多。

---

## 6. 与其它闸门的关系

- **G0**（v3 §24）：本目录未分类项 = 0 是硬闸门之一；另三条是固定
  source/provenance/SBOM/NOTICE（见 `provenance/sources.spdx.json` 与仓根 `NOTICE`）、
  CrabCode 每个拟复制文件有授权或明确转 clean-room、上游基线测试原始结果归档。
- **v3 §16.3 供应链**：`license/NOTICE/provenance verification` 与本目录同属 G0 证据面；
  构建期工具的钉版在 `tools/pins.toml`。
- **CLAUDE.md §10**：任何闸门失败只能修复后重跑，不能以「后续补齐」进入下一阶段。
