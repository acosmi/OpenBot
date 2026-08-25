# G3 Intelligence Importer batch 5

> 第一真源修订：v3 §28.1 R68；R63 继续有效，只运行本机定向测试，不派发 Actions / 不跑
> `cargo xtask ci`。

## 1. 完成项

- [x] `openbot-intelligence-bundle-v1` strict envelope + payload DTO；
- [x] Ed25519 strict signature 覆盖 header/nonce/ciphertext/plaintext hash；
- [x] 32-byte migration master → HKDF-SHA256(payload hash + header) → Zeroizing AES-256 key；
- [x] AES-256-GCM AAD header、signature-before-decrypt、plaintext SHA-256 与 payload/envelope binding；
- [x] 512MiB outer cap、regular-file/same-inode 读取；Unix decryption key 0600；secret 不进 argv/report；
- [x] 独立 target mapping：deployment/tenant/user/bot/channel；foreign UUID 逐 thread claim；
- [x] terminal-only run schema；queued/running 无反序列化入口；
- [x] source checksum、target-normalized checksum、DB 重建 checksum 三腿；
- [x] thread/member/title/anchor/message/run/event/observable memory 全量 projection；
- [x] 每 thread aggregate + 四类 cursor 同事务；failed cursor 精确 resume；completed rerun 全量重验；
- [x] existing ID exact replay / different binding conflict；
- [x] only active/superseded observable memory + mandatory source + `verified_import`；
- [x] `openbot-migrate intelligence-import` 真子进程；
- [x] `openbot-migrate intelligence-validate-tool-run-fk` staged FK finalizer；
- [x] importer crypto/runtime-exclusion dependency guard；Server main 零 importer 调用。

## 2. 关键裁决

1. Bundle 只能证明“legacy exporter 签过这些 source facts”，不能决定 target authority。Target mapping
   是管理员独立文件；fingerprint 不匹配/命名前旧 UUID 必须逐项 claim，未 claim 时零 DB 写。
2. Maintenance 必须先 drain。Bundle run enum 只有 completed/failed/cancelled/reconciliation_required；
   queued/running 不是一个会被运行时 `if` 忘掉的分支，而是类型层不存在。
3. 每 thread 是最小 crash unit：thread/member/run/message/event/memory 与四类 cursor 同事务。
   Cursor 绑定 bundle plaintext hash，换 bundle/换 deployment 不得续旧 cursor。
4. Checksum 不是输入自证。Exporter/source hash 先验；mapping 后生成 target oracle；事务写完再从 PG
   重建 projection 复算；completed rerun 仍复算，DB 事后 tamper 必须判红。
5. AES master 不直接用于 GCM。Payload hash 作 HKDF salt、signed header 作 info，派生 key 随 bundle
   内容/identity 改变，降低跨 bundle nonce 重用风险；derived key 在 drop 时擦除。
6. Memory 只导入可观察 content + verifiable source。Hidden learning、无来源 memory、forbidden/deleted
   不可观察内容都不伪造；origin 固定 `verified_import`。
7. `tool_calls_run_id_fkey` 只有所有 cursor completed 且 orphan=0 才 VALIDATE。Finalizer 是显式
   maintenance 命令，不在 Server startup/request path 偷跑长锁表操作。

## 3. 本机证据

- contracts bundle closed wire：**2/0/0**；
- application importer：**7/0/0**；
- bundle crypto：**1/0/0**，header/signature/key/ciphertext tamper 全拒；
- `openbot-migrate` unit：**3/0/0**；既有 preflight CLI：**2/0/0**；
- PG17/SCRAM importer adapter：**3/0/0**；
- 真实 CLI + 独立 exporter 侧实现：**1/0/0**；
- contracts WASM check：通过；定向 Clippy `-D warnings`：通过；
- `bash tools/check-intelligence-import-dependencies.sh`：通过；
- Cargo.lock：新 package **0**，仍 **428** packages；Cargo Vet 仍 **15/403**；
- tests ledger 新闭合 **5**：**174 done / 873 todo / 1047**；
- API：**26/130/156**；总 parity：**293/1360/1653**，violations/warnings 0；
- fixtures：**10/22/32**；GitHub Actions 未派发。

## 4. Legacy exporter 必须实现的 v1 格式

- Outer JSON 字段固定为 `format/bundleId/sourceDeploymentId/nonce/ciphertext/payloadSha256/
  signingKeyId/signature`，camelCase，未知字段拒绝；二进制均 RFC4648 standard base64，hash 为
  64 字符 lowercase hex。
- `framed(x) = u64 big-endian byte length || raw bytes`。AAD/HKDF info = 依次 framed：
  `format, bundleId, sourceDeploymentId, signingKeyId, payloadSha256(raw 32 bytes)`。
- AES key = `HKDF-SHA256(master=32 bytes, salt=payloadSha256 raw, info=AAD, len=32)`；
  AES-256-GCM nonce 12 bytes，ciphertext 含 16-byte postfix tag，AAD 如上。
- Ed25519 signature input = `AAD || framed(nonce) || framed(ciphertext)`；必须先验证 signature，
  再 HKDF/decrypt，再比较 plaintext SHA-256，最后解析 payload。
- Payload schema 逐字段以 `openbot_contracts::intelligence` 为 canonical inventory；时间必须能无损
  进入 PostgreSQL microsecond precision。Run 只能 terminal；memory 必须 observable content+source。
- Per-thread checksum 使用 u64-BE length-framed canonical JSON records：messages 按 seq；events 按
  `(eventSeq,runId,seq)`；terminal states/runId、memory/id 排序；另算 thread projection 与 sample
  render。实现可独立，但结果必须逐字段等于 bundle checksum；Rust 测试的 exporter 侧实现不调用
  verifier/sealer 辅助函数，以避免同一实现自证。

## 5. 实际未完成边界

本机没有真实 Intelligence customer export/API，也没有第一真源要求的 actual legacy exporter 运行结果；
因此本批证明的是 Rust importer、neutral format 与独立 exporter 实现的互操作，不是 production 数据已迁。
G8 仍须取得合同/法务许可后，在 maintenance 对生产规模脱敏快照完成三次真实 export/import/restore，
记录 count/hash/terminal/sample render、RPO/RTO 与签名 key custody。真实 provider/Agent consumer、
`remember` tool 与 Memory GUI 同样未完成；G3/G8 整关均不勾。

## 6. Git 恢复点

- 实施提交：`31903eeb89d326800a63e72239c13f1aa52c4661`；
- 分支：`feat/2026-08-24-G3-intelligence-importer`；
- 堆叠 PR：**#22**，base=`feat/2026-08-24-G3-run-recovery-terminal`，创建后
  `OPEN/CLEAN/MERGEABLE`；
- 实施 head 的 GitHub Actions run 数：**0**。
