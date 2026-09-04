# fixtures/

Phase 0（Evidence Freeze）的 fixture 目录。真源：

- `docs/2026-08-21-OpenBot全量Rust重写终版研究与实施方案.md` §19.3（Phase 0 必做产物）、§8.3（CEL）、§21（测试与量化验收）、§24 G0；
- `docs/2026-08-22-OpenBot-GUI设计系统与视觉规格-方案.md` §10（视觉 oracle 与闸门）、§11（Phase 0 产物新增）。

上游固定基线 commit：`891df72f1827454d8b353d108fe5dd2313b7e30d`（CopilotKit/openbot）。本目录里任何"上游是这样的"的断言都必须能在这个 commit 的干净克隆上复现。

---

## 1. 目录

| 路径 | 内容 | 状态 |
| --- | --- | --- |
| `MANIFEST.yaml` | 本目录的 parity ledger。每条 fixture 一行，`test_id` 前缀 `T-FIX`。 | 50 条：29 done / 21 todo |
| `policy/cel-corpus.json` | CEL corpus。69 条表达式 × context，结果类别由 `cel-js@0.8.2` 实跑测出。 | done |
| `ui/seed.json` | GUI golden 的确定性数据（§10.2）。25 张实体表 95 行 + native 段 + runtime views，覆盖 27 页。 | done |
| `ui/golden/MANIFEST.toml` | golden 清单：镜像 digest 位、字体、视口、阈值、平台矩阵、mask 规则、bundle 预算。 | done（digest 为 TBD 占位） |
| `ui/golden/{web,macos-arm64,windows-x64}/*.png` | golden PNG 本体。 | todo（T-FIX-0004..0007） |
| `agui/*.jsonl` | AG-UI 事件族与畸形事件录像。 | todo（T-FIX-0010..0012） |
| `provider/*.jsonl` | provider 流录像。 | todo（T-FIX-0013..0017） |
| `mcp/*.json` | MCP conformance、catalog 漂移、OAuth、恶意载荷。 | todo（T-FIX-0018..0021） |
| `browser/*.json` | Browser engine 操作、陈旧 ref、被拒副作用。 | todo（T-FIX-0022..0024） |
| `computer/*.json` | Browser residency、CDP输入、protocol-v3 screencast与背压边界。 | done（T-FIX-0025/0048..0050；viewer/跨平台仍按各自todo） |
| `upstream-baseline/` | 上游基线测试原始输出归档（§24 G0）。 | todo（T-FIX-0026） |

> `todo` 是 v3 §19.3 允许的状态。G0 的判据是"未归类项 = 0"，不是"全部 done"。
> 但 `todo` 必须写清**需要录什么 / 录制前提 / 验收判据 / 阻塞原因 / 解除条件**五项，否则它只是"待后续"，等于没有归类。
> `MANIFEST.yaml` 里每条 todo 的 `notes` 都逐项写了这五项。

---

## 2. `MANIFEST.yaml` 的 schema

顶层键集合与 parity ledger 统一 schema v1 逐字一致，只有六个：
`schema` / `schema_version` / `upstream_commit` / `generated_by` / `recount` / `entries`。

`schema` 的取值是 `fixtures`。它不属于 parity ledger 的九个名字
（`api` `routes` `tables` `env` `events` `components` `browser-operations` `ui` `tests`），
因为这份台账管的是 fixture 而不是某一个 parity 维度。八条校验规则对它逐条适用，
所以 CI 可以用同一个校验器一视同仁地拒绝"无证据的 done"。

校验器强制的八条：

1. 除 `notes` / `done_evidence` 外每个键都存在且为非空字符串；
2. `label` ∈ {`parity`, `新增`, `替代`}；
3. `owner` ∈ §5.1 的十个 crate；
4. `status = done` 当且仅当 `done_evidence` 存在且非空；
5. `id` 文件内唯一，`test_id` 全部 ledger 内唯一；
6. `test_id` 匹配 `^T-[A-Z]+-[0-9]{4}$`；
7. `upstream` 禁止裸行号（不得以 `:<数字>` 结尾）；
8. `recount` 至少一条且每条 `command` 非空。

本机复算（六条 recount 全部跑过，见 `MANIFEST.yaml::recount`）：

```bash
python3 -c "import yaml,io;d=yaml.safe_load(io.open('fixtures/MANIFEST.yaml',encoding='utf-8'));print(len(d['entries']))"          # 50
python3 -c "import yaml,io;d=yaml.safe_load(io.open('fixtures/MANIFEST.yaml',encoding='utf-8'));print(sum(1 for e in d['entries'] if e['status']=='todo'))"  # 21
python3 -c "import yaml,io;d=yaml.safe_load(io.open('fixtures/MANIFEST.yaml',encoding='utf-8'));print(sum(1 for e in d['entries'] if e['status']=='done'))"  # 29
```

---

## 3. `policy/cel-corpus.json`

### 3.1 它是什么

v3 §8.3 要求 Phase 0「从现有默认、测试和生产脱敏 policy 构建 corpus」，Rust 对每条 expression、context、结果和错误语义做 golden 对照，**oracle 固定为 `cel-js@0.8.2`**。

本文件的 69 条 entry 覆盖十一组：`engine-divergence` 8 · `default-policy` 2 · `contains-semantics` 5 · `non-boolean` 4 · `broken-syntax` 2 · `field-rules` 12 · `key-and-shortcircuit` 9 · `intent` 7 · `command` 3 · `shipped-presets` 9 · `engine-probe` 8（合计 69，复算命令见 §3.4）。

来源（全部是固定 commit 上读实的，不是推断）：

| 来源 | 符号 |
| --- | --- |
| 两个注入的全局函数 | `server/src/computer/policy.ts::POLICY_FUNCTIONS` |
| policy 层求值与 fail-closed 语义 | `server/src/computer/policy.ts::evaluateActionPolicy` / `matches` |
| 默认 policy | `server/src/computer/policy-store.ts::DEFAULT_ACTION_POLICY` = `{mode:"enforce", deny:[], allow:["true"]}` |
| 上游测试里的全部表达式 | `server/tests/computer-policy.test.ts` |
| 随包预设 3 条 | `app/src/routes/_authed/admin/boundaries.tsx::PRESETS` |
| `.env.example` 的示例 deny | `.env.example::AGENT_COMPUTER_POLICY`（与预设 1 逐字相同） |

### 3.2 `result_class` 的词表

v3 §8.3 规定的三类是 `true` / `false` / `error`。上游 `policy.ts::matches` 把"抛错"与"返回非布尔"归到**同一条** fail-closed 路径，所以本文件把非布尔结果记为 `error`，同时在 `engine_raw` 保留引擎的真实返回：

- `engine_raw.kind = "boolean"` —— 真布尔；
- `engine_raw.kind = "non-boolean"` —— 表达式求值成功但答的不是布尔（带 `type` 与 `value`）；
- `engine_raw.kind = "throw"` —— 引擎抛错（带 `message` 原文）。

两种失效模式不被压成同一个字符串，否则 Rust 侧照着 `error` 复现时无法知道该抛错还是该返回字符串。

### 3.3 本轮实测出的六条分歧（`measured_findings`）

| ID | 结论 | 证据条目 | 正向对照 |
| --- | --- | --- | --- |
| F-CEL-1 | `cel-js@0.8.2` 无任何字符串方法，`contains`/`startsWith`/`endsWith`/`matches` 的**方法形式**抛 `Unknown method: <名>` | `method-form-*` 4 条 | `global-form-contains` / `global-form-matches` 正常求值 |
| F-CEL-2 | 两个全局函数都大小写不敏感（`contains` 双向 `toLowerCase`，`matches` 强制正则 `i` 标志） | `contains-case-insensitive-*`、`global-matches-case-insensitive` | `contains-miss` 答 `false` |
| F-CEL-3 | `&&` **只**从左向右短路；`||` **完全不短路** | `and-shortcircuit-left-false`、`and-no-reverse-shortcircuit`、`or-no-shortcircuit-left-true`、`or-no-shortcircuit-right-true` | `key-guarded-on-navigation` 答 `false` |
| F-CEL-4 | 读不存在的标识符或不存在的嵌套字段一律**抛错**，不返回 `null` | `contains-missing-element`、`key-unguarded-on-navigation`、`intent-missing-on-plain-click`、`probe-absent-nested-field-read` | `contains-empty-element-name`（存在但为空串）答 `false`；`probe-has-absent-field` 答 `false` |
| F-CEL-5 | 上游测试把 `repeat.count` 注释为"a number"，实测它抛 `Identifier "repeat" not found` | `non-boolean-unknown-root` | `non-boolean-bare-field` 才是真正的非布尔 |
| F-CEL-6 | `Identifier not found` 的错误消息把**整份 context 的 JSON** 拼进消息体，而上游 `matches` 用 `console.error` 原样打日志 | `contains-missing-element`、`key-unguarded-on-navigation` 的 `engine_raw.message` | `method-form-contains` 的消息不含 context |

F-CEL-1 与 F-CEL-2 就是 v3 §8.3 点名要求进 corpus 的两条。
F-CEL-3 / F-CEL-4 / F-CEL-5 是本轮跑出来的**新**分歧，§8.3 原文未列。
F-CEL-6 不是 parity 项而是**明确不照译**的上游缺陷（v3 §2.4 类）：它与 §8.6 的 payload 字段 allowlist 冲突，Rust 侧的错误消息只带表达式与失败原因。

### 3.4 复算

```bash
jq '.entries | length' fixtures/policy/cel-corpus.json                                                     # 69
jq -c '[.entries[].result_class] | group_by(.) | map({(.[0]): length}) | add' fixtures/policy/cel-corpus.json  # {"error":19,"false":15,"true":35}
jq '.contexts | length' fixtures/policy/cel-corpus.json                                                    # 21
jq '[.entries[] | select(.group == "engine-divergence")] | length' fixtures/policy/cel-corpus.json          # 8
```

### 3.5 怎么重新生成

`result_class` 与 `engine_raw` **不是手写的**，由脚本对 `cel-js@0.8.2` 实跑得到。重跑方式：

```bash
mkdir -p <workdir> && cd <workdir>
npm install cel-js@0.8.2      # 实测 9 个包，35s
node gen-corpus.mjs           # 直接写出 fixtures/policy/cel-corpus.json
```

`gen-corpus.mjs` 里的 `POLICY_FUNCTIONS` 与上游 `server/src/computer/policy.ts::POLICY_FUNCTIONS` 逐字对齐；对齐关系改了就必须同 PR 改两边。

**它现在不在本仓里。** 生成脚本 `celprobe/gen-corpus.mjs`（含 21 个 context 与 69 条 entry 的定义）
本轮只写在生成它的那次会话的临时工作目录里，随会话消失，**不入库**。绝对路径刻意不写进本文件：
本仓是 public，机器名 / 用户名 / 会话目录都不该出现在这里。

本仓是零 Node 项目（v3 §0.1 / GUI §12.1：构建链不引入 Node），把一个 `.mjs` 连同 `npm install` 的依赖收进仓库是一次范围变更，需要主控裁决：
① 收进 `tools/cel-oracle/`，并把 `cel-js@0.8.2` 的 tarball sha256 写进 `tools/pins.toml`（可离线复跑，但仓里多一条 Node 工具链）；
② 不收，corpus 视为一次性冻结产物，Rust 侧只做只读对照（改 corpus 必须重走一次本流程）。

在裁决前，`fixtures/policy/cel-corpus.json` 的 `result_class` / `engine_raw` 两列**在仓内不可复算** ——
仓里只有结果，没有产生结果的脚本。同族的还有 `tools/pins.toml` 里两条标了
`repo_reproducible = false` 的 recount（`tw-musl-present` 与 `binaryen-sha-vs-upstream`，
其取证文件同样只在会话临时目录里），所以"证据不在仓内"一共是这三处，不是一处。复算：

```bash
python3 -c "import tomllib;d=tomllib.load(open('tools/pins.toml','rb'));print(sum(1 for r in d['recount'] if r.get('repo_reproducible') is False))"   # 2，加本条 = 3
```

Rust 侧的对照值缺位，由 `T-FIX-0009` 在 `openbot-domain` 落地后补。

---

## 4. `ui/seed.json`

GUI §10.2 的确定性数据。要点：

- 时钟钉在 `2026-01-01T00:00:00Z`，所有时间戳都不晚于它；相对时间文案由 `now` 减字段值得出，所以 `now` 变了 golden 必须全量重截。
- `pages_covered` 27 条 = 26 个上游页面 route + `/memory`（v3 §4.3 的 native memory 面，上游无对应物，标 `新增`）。26 这个数字可复算：
  ```bash
  # cwd = 上游克隆
  grep -rhoE 'createFileRoute\(\s*$|createFileRoute\("[^"]*"' app/src/routes | wc -l   # 30
  # 减去 4 个纯布局 route：/_authed、/_authed/_app、/_authed/admin、/_authed/settings
  ```
- `entities` 是 25 张表共 95 行，键名与上游 `server/src/db/schema/**` 的表名一致；`native` 段是 v3 §4.3 新增或接管的表，整体标 `新增`；`runtime_views` 不是表，是 GUI 直接消费的投影（形状取自 `app/src/lib/computers/queries.ts::ComputerFleet` 与 `server/src/computer/schema.ts::ComputerStatus`）。
- 覆盖面刻意包含**空态与坏态**：`ch-empty` 没有任何消息；`tickets` MCP server 带 `last_error` 且凭据已 revoke；`usr-eli` 在 `revoked_access` 里；两个 component 未发布。只测"有数据"的分支等于没测。
- 中文覆盖：用户 `陈墨`、channel `客户支持`、skill `中文语气规范`、三条中文消息，用于 zh-CN golden 与 §8.6 CJK 排版。
- `msg-orders-0005` 的正文逐字取自 `server/src/computer/policy.ts::describeRefusal` 的输出形状，同时覆盖行内代码渲染与超长规则换行。

复算：

```bash
jq '[.entities | to_entries[] | .value | length] | add' fixtures/ui/seed.json   # 95
jq '.entities | length' fixtures/ui/seed.json                                   # 25
jq '.pages_covered | length' fixtures/ui/seed.json                              # 27
jq '.native.messages | length' fixtures/ui/seed.json                            # 10
```

---

## 5. `ui/golden/MANIFEST.toml`

清单，不是台账。它规定 golden 怎么截、怎么比、允许多大；某一张 PNG 到底有没有产出由 `MANIFEST.yaml` 的 `T-FIX-0004..0008` 负责。**清单里的 `TBD` 占位不构成台账里的 `done`。**

- 平台矩阵四条，合计 245 张：Web/en 110（`27×2×2 + 1×2`）、Web/zh-CN 27、macOS arm64 54、Windows x64 54。
- zh-CN 的 27 来自 Phase 0 任务书；§10.1 的矩阵表本身没有 locale 维度，所以清单把 zh-CN 腿的主题与视口各钉死为一个值（light / 1440×900）来让 27 成立，改任一维度就要同 PR 改 §10.1。
- 比对：任一通道差 `> 16/255` 记为差异像素；**失败判据 = 差异像素 > 0.1% 或存在任一 8×8 全差异块**。
- mask 清单首版为**空**，并写死"新增 mask 需评审"的规则。空不是遗漏：seed + Clock + 确定性头像已覆盖 27 页的全部动态源，实时 screencast 帧不在这 27 页里。
- bundle 预算三行：`app.wasm` gzip ≤ 3.5 MiB、`app.css` ≤ 128 KiB（120 KiB warning）、两份 woff2 ≤ 800 KiB（实测 740,216 B = 352,240 + 387,976）。

复算：

```bash
python3 -c "import tomllib,pathlib;d=tomllib.loads(pathlib.Path('fixtures/ui/golden/MANIFEST.toml').read_text(encoding='utf-8'));print(sum(m['count'] for m in d['matrix']))"     # 245
python3 -c "import tomllib,pathlib;d=tomllib.loads(pathlib.Path('fixtures/ui/golden/MANIFEST.toml').read_text(encoding='utf-8'));print(sum(f['bytes'] for f in d['fonts']['bundled']))"  # 740216
```

---

## 6. 四类“必须实录”fixture 的当前边界

本节最初的“crates 为空、四类全部 todo”已被后续批次推翻，当前状态只认
`fixtures/MANIFEST.yaml`与v4最新R行。AG-UI十一事件族已有production PostgreSQL纵向；OpenAI recorded
trace为provider `1/3`；RMCP已有产品协议纵向但完整官方conformance仍todo；Browser/CDP/Screen真实矩阵仍缺。

尚存的共同外部边界是：Anthropic/Google官方recorded或live credential证据、固定官方RMCP conformance
runner、Ubuntu runsc/Xvfb与Windows真机，以及Web golden的固定Linux镜像/fonts和Desktop正式发行窗口。
这些缺口不得用手写事件、compile-only、单元测试或本机另一平台结果替代。Batch107已经提供PNG-only
比较/diff/manifest gate；容器digest或245张基线缺失时`cargo xtask golden verify`必须判红。

依赖顺序（先做上面的才能做下面的）：

```text
T-FIX-0013 provider 基线流  ──> 0014 partial JSON/UTF-8 切分
                            └─> 0015 交错 tool call
                            └─> 0016 429/401/timeout/cancel race
T-FIX-0010 AG-UI 官方事件族 ──> 0011 畸形与漂移
T-FIX-0018 RMCP conformance ──> 0021 恶意载荷
T-FIX-0022 BrowserOperation ──> 0023 陈旧 ref / 0024 被拒副作用 / 0025 screencast 帧
T-FIX-0004 Web golden       ──> 0008 镜像 digest 固化（同一次 CI 绿，同 PR 写回）
```

---

## 7. 写新 fixture 时必须遵守的三条

1. **每个计数配一条可直接执行的复算命令**，并写进对应文件的 `recount` 段；`expect` 必须是你亲自跑出来的值。
2. **位置引用只用符号名**（`path::symbol`），禁止裸行号——校验器第 7 条会拒绝以 `:<数字>` 结尾的 `upstream`。
3. **"期望 0 / 不存在 / 被拒绝"的断言必须配正向对照。**
   例：`T-FIX-0023` 要求旧 ref 100% 被拒绝，就必须同时证明当前 ref 会成功；
   `T-FIX-0022` 声称上游没有下载/上传路径 —— 本轮已复算并配了正向对照，见下面 §7.1；
   `T-FIX-0018` 要求 conformance 100% 通过且**无 expected-failure baseline**，就必须故意打坏一条 case 验证它真的会判红。
   否则这些断言在"该功能压根没实现"的世界里同样成立。

### 7.1 一个实测到的弱证据（`page.on`）

v3 §11.2 断言上游 29 条手写浏览器路径里没有下载 / 上传 / 文件选择 / 对话框处理，并逐字点名 ``page.on``、``setInputFiles``、``filechooser`` 全仓零命中。本轮在固定 commit 上复算：

```bash
# cwd = 上游克隆；范围 agent-computer/src server/src app/src worker supervisor shared
setInputFiles / filechooser / fileChooser / waitForEvent
  / suggestedFilename / saveAs / acceptDownloads / 'download' / "download"   # 全部 = 0
# 正向对照（cwd = 上游克隆，grep -F -c 求和，范围 agent-computer/src）
.goto(  = 1     .click( = 3     .screenshot( = 1     .on( = 6
```

结论分两半：

- **按 API 名的七项零命中是可信的**，正向对照证明同一条 grep 能找到这类代码（`.on(` 有 6 处命中）。"上游没有下载 / 上传路径"这个结论成立。
- **``page.on`` 这一项是弱证据。** 上游的 Playwright Page 变量叫 `target` 不叫 `page`（`agent-computer/src/index.ts` 里是 `target.goto(...)`），所以 `page.on = 0` 在"确实有 `page.on` 但换了变量名"的世界里同样为真——它测的是变量命名习惯，不是有没有事件监听。建议同 PR 把 §11.2 的复算命令换成按 API 名的版本。
