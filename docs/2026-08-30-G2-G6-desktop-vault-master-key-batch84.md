# Batch84：Desktop Vault master key OS key-store

> 日期：2026-08-30（America/Los_Angeles）
>
> 分支：`feat/2026-08-30-G2-G6-desktop-vault-master-key`
>
> base：`bd5ae924020a5d2556cadf47d40fd1c5c3217e9a`（PR #66 merge commit）
>
> implementation：`fc2085cffd8ceeb096ba576f228e41a88e5172aa`
>
> 第一真源：v4 §5.1–§5.2、§6.1、§6.4、§8.6、§9.2、§13.1–§13.3、§16.2–§16.4、§24 G2/G6、§28.1 R155–R158。

## 1. 结论

Batch83 已有权威 data plane，但 production `ApplicationService` 仍需要 credential Vault、audit checkpoint、run assertion 与 MCP OAuth state 四类密钥输入。第一真源 §6.4 明确要求 Desktop master key 来自 Keychain / Credential Manager / Secret Service；复用 Server `KEY_ENCRYPTION_KEY`、环境变量或 app-data 文件都不合法。

本批闭合 supported Desktop 的 master-key前置：

1. 把 Batch81 的平台调用上收到唯一 `OsSecretStore`；
2. PostgreSQL SCRAM 与 application Vault 分别拥有自己的service/account/格式/reconciliation；
3. 只有仍持 sidecar start lock 的 `RunningDesktopLocalDataPlane` 可 load/create application key material；
4. master item按per-instance account存version 1 + 32-byte CSPRNG key；
5. existing格式错误不覆盖，new写后立即回读并常数时间相等；
6. master只在startup `SecretBytes`中存在，随后构造credential Vault、audit key、run assertion signer与MCP OAuth state key并擦除；
7. 没有环境/file fallback，窗口仍未创建。

## 2. 唯一平台边界

`os_secret_store.rs` 是唯一 macOS `security-framework` consumer：

- default/private Keychain generic password；
- not-found与unavailable分型；
- read立即进入`SecretBytes`；
- 日志只记platform code，不记service/account/value。

Windows仍只由第11个`openbot-windows-sandbox`封装 `CredReadW/WriteW/DeleteW/Free`；共享adapter只拼已reviewed target并消费safe wrapper。PG模块不再直接命名Keychain/Credential Manager API，通过blanket adapter保留原`PostgresSecretStore`格式边界。guard逐文件截掉tests后要求production Keychain consumer恰一、Win32 FFI consumer恰一。

## 3. Master key格式与并发

service ID由release输入，要求3–128位canonical lowercase ASCII `[a-z0-9._:-]`并拒source marks。account固定：

```text
desktop-vault-master-v1-<64 lowercase hex instance>
```

value固定33 bytes：首字节format version=`1`，后32 bytes是OS CSPRNG master。读取只收exact shape；corrupt写0。首次生成时frame本身也进入`SecretBytes`，write后readback→decode→CT equality；missing/different为reconciliation required。

调用入口挂在`RunningDesktopLocalDataPlane`上，所以平台操作发生时Batch80 start lock仍由exact child owner持有；第二个Desktop进程无法同时生成不同master。该同步API必须由后续Tauri background startup调用，不能在UI thread执行。

## 4. Application material

`DesktopApplicationKeyMaterial`持有：

- tenant-bound `CredentialRecordVault`，key version 1；
- domain-separated audit checkpoint `SecretBytes`；
- 只持derived key的`RemoteRunAssertionSigner`；
- domain-separated MCP OAuth state `SecretBytes`。

audit/MCP HMAC labels从Server main上收到`openbot-domain::vault::derivation`，自由label不可表示。固定旧Server向量机械锁住；Server继续从raw `KEY_ENCRYPTION_KEY`配置字节导出，兼容通常44-byte base64文本，不错误强制AES key长度。Server生产图因此移除hmac直接边，sha2只留integration dev dependency。

## 5. 真实证据

macOS private Keychain测试：create private keychain→none→write versioned master→read→derive→seal credential→restart read same master→另一个material open逐字节相同→delete；临时keychain目录0。既有PostgreSQL private Keychain测试同批回归。

Batch83 host PG17.11纵向进一步在running data plane上：首次Vault write=1并seal canary，clean shutdown；sidecar/data plane restart后Vault write仍1、重新derive后open同一密文逐字节相同，再verified shutdown。最终=`1/0/0`。

Windows新增同格式Credential Manager ignored纵向，target tests/Clippy编译通过；未在Windows真机运行，不记PASS。Linux tier-2 `desktop-vault` feature compile通过，但没有Secret Service adapter，runtime仍fail-closed/不可构造。

## 6. 首跑失败与修正

- Desktop test首编译发现workspace UUID未开v4；测试改固定`Uuid::from_u128`，不扩feature；
- Windows Clippy发现macOS-only test sequence static未cfg；收窄target后绿；
- shared derivation最初错误要求16/24/32-byte AES形状；复核Server历史发现audit/MCP使用raw配置字节，改为任意非空HMAC master并钉旧向量；
- Server移除sha2生产边后integration test缺direct dev edge；只移入dev-dependencies；
- Desktop最小infra Vault图的通用repo宏未使用，lint只在`!server-runtime`模块局部allow，Server完整图仍`-D warnings`。

以上均不计成功；最终证据如下。

## 7. 本轮亲跑证据

| 证据 | 结果 |
| --- | --- |
| Domain完整 | `371/0/0` |
| Desktop完整all-feature | `123/0/2`；两ignored为PG host fixtures，macOS两条private Keychain均实跑 |
| Server lib / production assembly | `216/0/0` + `7/0/0` |
| PG+bootstrap+Vault host | `1/0/0` |
| macOS Clippy | Desktop最小图与Domain/infra/Server完整图 all-target/all-feature绿 |
| Windows | Desktop+唯一unsafe crate all-target/all-feature Clippy绿；Vault/Credential Manager runtime ignored未跑 |
| Linux | `desktop-vault` feature compile绿；既有budget/transport dead-code 2 warning |
| dependency guard | 单Keychain consumer、单Cred FFI consumer、PG/Vault格式隔离、Server图保持，绿 |
| Cargo | package `825→825`；Desktop dev新增既有uuid direct edge，Server生产移除既有hmac direct edge |
| parity / strict | `813/881/1694`、0 violation、overlay=`1289/397/2/6`；strict=`159/0/0` |
| Grok/package/npm | tree `86f5a85f…`、inventory 2,110；非Grok一个package.json；npm=0 |

本批无API/UI/T-ID/CSS/locale变化，没有重跑Trunk/Browser/Engine/golden，没有运行`cargo xtask ci`，没有派发Actions。Cargo Vet既有红线与exemption均未改。

## 8. 未闭合边界

- Windows Credential Manager真机format/derive/seal/open未跑；Linux Secret Service未实现；
- key rotation/key ring、recovery/export与旧Desktop master迁移尚无；当前fresh Desktop固定version1；
- production `ApplicationService` adapters仍未由这些material组装；
- 真实Tauri app-data/background setup、window-last、tauri.conf/capability/identity仍未接；
- release PostgreSQL binary、backup/upgrade、真实Wry/golden仍未闭合；
- 不关闭Desktop全Vault/KMS、G2/G6整关，不修改grok-bot，不新增Grok产品能力/npm。

下一批可在不读环境密钥的前提下抽取 Server/Desktop共享production application assembly；Desktop必须只消费本批material与Batch83 pool/auth，所有adapter成功后才允许创建window。
