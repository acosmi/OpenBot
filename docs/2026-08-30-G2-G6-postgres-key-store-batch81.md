# Batch81：PostgreSQL SCRAM OS key-store boundary

> 日期：2026-08-30（America/Los_Angeles）
>
> 分支：`feat/2026-08-30-G2-G6-postgres-key-store`
>
> base：`bb27acfa974d9a906640ddbaf38662066dbe8677`（PR #63 merge commit）
>
> implementation：`b11f7b3d179275d5c680cbfdd1ec965f6e3927bb`
>
> 第一真源：v4 §5.1、§6.4、§13.1、§14.1、§16.2–§16.3、§23.4、§24 G2/G6、§28.1 R154/R155；GUI v2 §15.1。

## 1. 结论

本批关闭 Desktop PostgreSQL 随机 SCRAM secret 的 OS key-store 边界：

- 只有持有 Batch80 per-instance start lock 的 owner 可以 load/create；
- key-store service 必须由 host 断言已评审，且是跨平台 canonical lowercase ASCII、无禁用来源 mark；
- account 固定 `postgresql-17-<instance-id>`；renderer、环境变量和文件配置没有自报入口；
- existing secret 必须恰为 64 lowercase hex，corrupt 时 fail-closed 且写入0；
- none 时用 OS CSPRNG 生成32字节，转64hex，写入后立即回读并常数时间比较；missing/different 返回 reconciliation，不继续启动；
- 所有 Rust-owned read result 从构造时即由 `SecretBytes`/zeroizing owner 接管，Debug redacted，不实现 Clone/Serialize；
- macOS 使用 `security-framework 3.7.0` generic password，并用独立 private Keychain 真跑；
- Windows 继续通过仓内唯一 unsafe crate封装 Credential Manager，绝不在 Desktop crate 写 Win32 unsafe。

本批没有启动 PostgreSQL。平台 binary build/sign、`initdb`/spawn/ready/graceful、Windows Credential Manager真机、Linux Secret Service、真实 Tauri setup仍未闭合；也不把一个SCRAM项写成Desktop全Vault/KMS完成。

## 2. Secret contract

### 2.1 Reviewed service + instance account

`ReviewedPostgresKeyStoreService::from_reviewed_release`只接受3–128字节：

```text
[a-z0-9._:-]+
```

值必须已经lowercase，并拒绝 `openbot` / `copilotkit` / `codex` / `openai` / `grok` / `xai`。真实service值仍等待reviewed外部产品identity；测试使用中性fixture，不把fixture名带进发行物。

Windows generic TargetName大小写不敏感，macOS service/account构成generic-password复合键；强制lowercase避免两平台对同一输入产生碰撞/分叉。

### 2.2 Shape 与 ownership

`PostgresScramSecret` 恰为256-bit random的64位lowercase hex ASCII；它内部是 domain `SecretBytes`，只提供显式 `expose()` 给后续 `initdb`/process framing。

key-store port不返回裸 `Vec<u8>`，而返回私有字段 `PostgresStoredSecret(SecretBytes)`：平台adapter一取得owned bytes就交出唯一allocation；caller若不用也会zeroize。`PostgresScramSecret`/`PostgresStoredSecret` Debug都只打印 `[REDACTED]`，不实现Clone/Serialize/Display/PartialEq。

这些保证仍服从 `SecretBytes` 已披露边界：系统框架内部buffer与进入Rust owner之前的系统副本不在Rust可达范围。macOS `security-framework` 负责释放其OS-owned Keychain buffer；本仓不伪称可覆盖Security.framework内部内存。

### 2.3 Load/create transaction

入口是 live `PostgresStartLock::load_or_create_scram_secret`，不是store上的自由helper：

1. 由lock内不可变instance构造account；
2. read existing；若shape合法直接返回，非法报corrupt，不overwrite；
3. none时生成32-byte CSPRNG raw，raw先进入临时`SecretBytes`，编码后raw owner drop清理；
4. write 64hex；
5. 立即read back并再次shape-check；
6. `SecretBytes::ct_eq` 比generated/persisted；missing或different报`postgres_key_store_reconciliation_required`；
7. 返回persisted owner，generated owner drop。

共同测试证明restart第二次不写、首次writes=1；corrupt writes=0；模拟store在write后替换首byte时稳定reconciliation。

## 3. macOS Keychain

生产adapter `MacOsKeychainPostgresSecretStore::current_user_default()` 打开当前OS用户default Keychain；read/write使用指定Keychain的generic password service+account。`errSecItemNotFound`映射None，其它仅记录安全OSStatus数字并返回stable unavailable，不记录service/account/secret。

本轮正向不污染default Keychain：

1. `CreateOptions` 在唯一临时目录创建private `.keychain-db`，test-only password、prompt=false；
2. 绑定该Keychain的production adapter；
3. production start lock + reviewed test service首次read none→write→readback；
4. 第二次load逐字相同；
5. exact item delete；
6. drop secret/lock/store并删除private Keychain与lock目录；最终目录0。

Keychain API是同步阻塞API；port文档固定未来supervisor必须在Tauri UI线程之外调用。

## 4. Windows Credential Manager

`windows-sys 0.61.2`只新增 `Win32_Security_Credentials` feature；所有 `Cred*` raw pointer继续只在 `openbot-windows-sandbox`：

- `CredReadW` / `CredWriteW` / `CredDeleteW` / `CredFree`；
- `CRED_TYPE_GENERIC`；当前token的用户credential set；
- `CRED_PERSIST_LOCAL_MACHINE`，同一用户在本机后续logon可见、不漫游；
- target UTF-16 NUL-terminated，空/NUL/>512拒绝；
- blob只收1–128 bytes，低于Win32 2,560-byte上限；attributes/username/alias均0/null；
- read后只复制一次；OS返回blob在`CredFree`前用volatile逐byte清零；
- `WindowsCredentialSecret` Debug redacted、Drop zeroize，只有显式`into_bytes()`把唯一Vec移交Desktop `PostgresStoredSecret`。

Windows target完整typecheck与Clippy通过，Windows-only test也编译。真实round-trip test已落但 `ignored`，本轮没有Windows主机，不能宣称logon-session/persistence真行为通过。

## 5. Dependency delta

新增且只有两个registry package：

| package | version | checksum | license/build |
| --- | --- | --- | --- |
| `security-framework` | `3.7.0` | `b7f4bc…cd1d` | MIT OR Apache-2.0；无build.rs |
| `security-framework-sys` | `2.17.0` | `6ce269…20e3` | MIT OR Apache-2.0；无build.rs |

Cargo package `823→825`；Windows只增加既有`zeroize` direct edge，新增package0。

`check-postgres-key-store-dependencies.sh`机械锁定：

- exact version/checksum与zero build script；
- macOS Desktop graph必须含两包；Windows/Linux/Server graph必须为0；
- Windows Desktop必须经唯一`openbot-windows-sandbox`，该crate不得泄漏到其它图；
- `security_framework*` Rust consumer恰一个 Desktop sidecar文件；Cred* consumer恰一个Windows unsafe文件；
- secret路径禁止 `std::env::{var,...}` 或子进程CLI fallback。

六target cargo-deny bans/sources/licenses绿。Cargo Vet按既有同命令复算：macOS `272`（Batch16 270 + 新2），Windows `269`；仍红且 `supply-chain` config/exemption 0改。union `369`只用于诊断，不替代target计数。

## 6. 首跑失败与修正

- offline首次因新crate未缓存失败；获准后只下载exact `security-framework 3.7.0`/sys2.17.0；
- private Keychain测试首编译发现3.7.0 `SecKeychainItem::delete()`返回unit而非Result；按真实签名去掉unwrap；
- common trait由裸Vec再收紧为`PostgresStoredSecret`时漏改macOS impl签名，offline编译判红后修正；
- Windows Clippy首跑给出`ptr::read`/`CredWriteW` const引用与测试array建议三条；不加allow，逐项修后绿；
- dependency guard首跑误把`std::env::consts::ARCH`当运行期env fallback；收窄为`var/var_os/vars/vars_os`后绿；
- cargo-vet sandbox首跑因用户cache EPERM尚未执行；宿主重跑得到union369，再按既有target口径实得272/269；
- parity第一次调用因Batch80清理后`target-xtask`不存在exit127；重建xtask后正式parity绿；
- 最终fmt check曾指出Windows测试一处机械换行；执行rustfmt后最终check绿。

上述均不计成功，以下为最终证据。

## 7. 本轮亲跑证据

| 证据 | 结果 |
| --- | --- |
| Desktop all-feature | `115/0/0`；sidecar bundle/lock/key-store子集`6/0/0`；doc-test`0/0/1 ignored` |
| macOS Keychain | private Keychain真实create/none/write/read/restart same/delete，目录0，包含在上述6条 |
| macOS Clippy | Desktop all-target/all-feature `-D warnings`通过 |
| Windows compile/Clippy | Desktop +唯一unsafe crate all-target/all-feature MSVC target `-D warnings`通过；Windows tests typecheck通过 |
| Windows runtime | Credential Manager round-trip `ignored`，未跑 |
| Linux | `postgres-key-store` feature-only compile通过；仍有既有budget/transport dead-code 2 warning，不冒充Clippy/runtime；Secret Service未实现 |
| dependency guards | key-store guard、Tauri guard、六target cargo-deny全部通过 |
| Cargo Vet | union369红；target macOS272 / Windows269红；config/exemption diff0 |
| parity | `813/881/1694`、0 violation、required revalidate=0；fixtures `17/22/39`、overlay `1444/242/2/6` |
| strict recount | clean pinned upstream `891df72f…`，`159/0/0` |
| Grok | Git tree=`86f5a85f560f721677fa7e587a67ac0ffc036cb5`，diff0；inventory2,110 files |
| package/npm | Cargo823→825（exact新2）；非Grok恰一个`package.json`；npm=0 |

本批无API/UI/T-ID/CSS/locale变化，没有重跑Trunk、Browser、Engine/golden。没有运行 `cargo xtask ci`，没有派发GitHub Actions。

## 8. 未闭合边界

- Windows Credential Manager真机read/write/delete/persistence未跑；
- Linux tier-2 Secret Service adapter未实现；
- actual reviewed key-store service/product identity尚无值；fixture值不进发行；
- macOS production default Keychain constructor已编译，正向使用隔离private Keychain；未做签名entitlement发行验证；
- security-framework内部OS buffer由依赖释放，本仓只承诺Rust-owned allocation的SecretBytes zeroize；
- 平台PostgreSQL binary build/sign/manifest、`initdb`/spawn/version/ready/graceful/orphan/backup仍未实现；
- Batch79 installation/bootstrap与Batch80 bundle/lock/本批secret尚未组合成supervisor；
- 真实Tauri `app_data_dir()` setup/window仍未接；
- 本批只闭PostgreSQL SCRAM secret，不关闭Desktop全Vault/KMS、G2或G6整关；
- 不修改`grok-bot/`，不新增Grok产品能力或npm。

下一批应实现`VerifiedPostgresBundle`→start lock→OS secret→verified binary version→initdb/spawn→ready probe→Batch79 bootstrap→graceful shutdown的单一state machine；在Windows真机补Credential Manager证据前继续标红Windows release。
