# G3 Native Thread/Realtime/Memory 数据地基 batch 1

> 第一真源修订：v3 §28.1 R62；状态勾选见 v3 §24.1。本文只闭合物理数据地基、纯领域
> 不变量与 repository，不把表存在冒充 ApplicationService/SSE/WS/memory user journey。
>
> 运行方式补充：R63 按用户额度指令关闭 GitHub Actions 自动触发且禁止未经授权派发；本批后续
> 复核只跑本机定向测试。W-7c 已取得的 Ubuntu 历史证据仍有效，不因额度控制被改写成未完成。

## 1. 完成项勾选

- [x] ThreadIdentity 固定上游 8 条：SHA-256 deployment fingerprint 前六字节、UUIDv8、跨实例
  owns、跨 deployment 拒认、v4/坏 UUID 拒认、随机尾部不被 fingerprint 吞掉；
- [x] native 0016 十表：threads、thread_memberships、messages、runs、run_events、thread_leases、
  outbox、memories、memory_events、intelligence_import_cursors；
- [x] 十个 repository 与十表同批，`IMPLEMENTED_REPOSITORIES` 从 30 收敛为规划全集 40；
- [x] thread 软删、single foreground、terminal exactly-once、fencing、双 cursor、outbox
  replay-safe、memory explicit origin/source/scope/delete 约束；
- [x] post-0016 PG17 fixture、双 replica migration、staged tool→run FK 与既有 tool 矩阵回归；
- [ ] transactional append、50ms/8KiB chunk、LISTEN/NOTIFY live、SSE/WS reconnect、memory
  API/GUI、Intelligence importer/checksum 仍留后续 G3。

## 2. R62 裁决摘要

- Thread 用户删除为软删；不擅自发明固定 retention 天数，G8 policy 才决定物理清理；
- `reconciliation_required` 继续占 foreground slot，禁止 unknown commit 后自动开启下一 run；
- terminal 属性由 event type 决定，DB partial unique 保证每 run 至多一条 terminal event；
- lease takeover 只在 expiry 后发生并推进 fencing token；到 `i64::MAX` fail-closed；
- 普通 outbox 只收 internal / idempotent external destination，non-idempotent effect 不可入表重放；
- memory 只有 preference/fact，入口只有 user action / remember tool / verified import；没有 background
  learning 值。Fact/import 必须带 message+thread source；forbid/delete 同写 `content=NULL`；
- tool call FK 以 `NOT VALID` 添加：新写必须有 run，历史 0013 行不被扫描；完成 importer/backfill
  后再独立 `VALIDATE CONSTRAINT`；
- Thread/Memory repository 不公开 hard delete；Row Debug 对 title/message/outbox/run event/memory/
  import cursor 等用户内容与授权状态统一脱敏。

## 3. 机器证据

- batch 1 当时 `cargo test -p openbot-contracts ids::thread::tests --locked`：固定上游 **8/0/0**；
  R64 后同 module 另加 UUID plausibility/ownership 分离 1 条，当前为 **9/0/0**；contracts WASM 通过；
- `cargo test -p openbot-domain --locked`：**336/0/0**；
- native 0016 PG17/SCRAM：**3/0/0**；
- 0015 + 0016 + repositories + tool application 组合：**15/0/0**；
- `all_forty_current_repositories_touch_their_real_tables`：**1/0/0**；
- infra production ThreadId CSPRNG issuer：**1/0/0**（1000 次唯一且全部 owns）；
- 完整 workspace：**1101 passed / 0 failed / 117 ignored**；
- 完整 PG17/SCRAM infra+server：**525 / 0 / 0**；
- post-0016 fixture：**41 表 / 351 列 / 268 NOT NULL / 181 约束 / 80 索引 / 4 触发器**；
- `fixtures/db/schema-0016.json`：4195 行，SHA-256
  `3a9ca0e2292e25171785c526047c279291c1671a357195b603efa2b998616877`；
- AST generator：105 文件 / 229 describe / 1047 test，ThreadIdentity 8 条 overlay 全保留；
- parity：**269 done / 1376 todo / 1645 total**；tables=**54/0**，tests=**161/886**；
  fixtures=**10/22/32**；
- Cargo.lock 新 package：**0**；contracts 只增加锁内既有 `sha2 0.10.9` 直接边；
- schema latest：**0016**，native ledger 四行。

## 4. 未冒充的边界

本批有 replay repository，但还没有“先 replay 再 live”的 transport；有 outbox claim/CAS，但还
没有 relay loop；有 memory 表/召回 SQL，但还没有两个显式写入口的 ApplicationService、API 或
GUI；有 import cursor，但没有 bundle signature/import/checksum 工具。上述项目完成前 G3 整关
保持未勾，`intelligence_channel_mappings` 继续只读 legacy provenance，不删除、不回退成 live truth。

## 5. Git 恢复点

- 实施提交：`21d25942dd170f12ecf4c050478139a64e77e987`；
- 分支：`feat/2026-08-24-G3-native-data-base`；
- PR：#18，base=`fix/2026-08-24-W7c-linux-ci`，head=上述分支；
- 创建后实查：OPEN / CLEAN / MERGEABLE，`statusCheckRollup=[]`；Actions 无新增 run。
