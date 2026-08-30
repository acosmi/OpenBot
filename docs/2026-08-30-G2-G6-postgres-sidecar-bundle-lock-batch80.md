# Batch80：PostgreSQL source pin、release bundle attestation 与 start lock

> 日期：2026-08-30（America/Los_Angeles）
>
> 分支：`feat/2026-08-30-G2-G6-postgres-sidecar-supervisor`
>
> base：`6292f1c8950bc66133728bfe1c4fdc6ddaeda1cb`（PR #62 merge commit）
>
> implementation：`c6a6fde8415e2e61694c1e69110a4f58638068cf`
>
> 第一真源：v4 §5.1、§13.1、§14.1、§16.2–§16.3、§23.4、§24 G2/G6、§28.1 R153/R154；GUI v2 §15.1。

## 1. 结论

本批关闭 sidecar supervisor 的两个供应链/并发前置，但刻意不声称进程 supervisor 已完成：

- PGDG PostgreSQL 17.11 官方 source archive 的 URL、精确字节数、官方 SHA-256 copy 与 tar 结构进入机器 pin；
- `cargo xtask postgres fetch-source|verify-source` 只在 release build 阶段下载/验证，应用首次运行零下载；
- Desktop runtime 只能接 signed outer core 给出的 manifest digest，随后校固定 release 字段和 bundle 全部普通文件；
- platform/arch、PG version/source SHA、release epoch、minimum core、reviewed signing identity 与三个 program 逐项固定；
- 任何额外文件、symlink、非普通文件、非 canonical lowercase SHA、文件 hash 或固定字段漂移均 fail-closed；
- per-instance start lock 用 `create_new`、CSPRNG nonce、0600、file+directory fsync 排他；Drop 只 bounded 比对自己的完整字节后删除，被替换或崩溃残留都保留。

本批没有构建或签名 macOS/Windows PostgreSQL binary，没有生成真实 release manifest，没有读取 OS key store，也没有执行 `initdb`、spawn、ready probe、shutdown 或 Tauri setup。

## 2. 根因与边界

Batch79 可以证明“这个活 PostgreSQL 的 data_directory 属于当前 app instance”，但仍由 caller 提供 pool。没有可信 binary 来源时，开发机 PATH/Homebrew 或任意替换的 `postgres` 都可能被启动；两个 Desktop 进程也可能同时在 attestation 之前拉起 sidecar。

三条不能混写：

1. source archive SHA 证明源码输入，不等于平台 binary SHA；binary 还取决于编译器、SDK、configure/Meson 选项、依赖与签名；
2. manifest 内自报文件 SHA 不构成真实性；攻击者能同时修改 manifest 和文件，所以 manifest SHA 必须由 signed outer core 独立提供；
3. PID 不是稳定进程身份；用 PID 猜 stale lock 会遇 PID reuse，并可能在旧进程仍活时双启动。

因此本批只建立 source truth、release-owned manifest contract 与 fail-closed lock。crash residue 暂不自动恢复；未来只有平台认证的 process identity/orphan recovery 才能解锁，当前需显式 repair。

## 3. PGDG 17.11 source pin

`tools/postgres-pins.toml` 固定：

| 字段 | 值 |
| --- | --- |
| version / major | `17.11` / `17` |
| archive | `postgresql-17.11.tar.gz` |
| URL | `https://ftp.postgresql.org/pub/source/v17.11/postgresql-17.11.tar.gz` |
| size | `28,397,423` bytes |
| SHA-256 | `5367f6fb2ec97efe1eb2e0c7926bb33438e51b0bd3a9733b88498056a7dc9a7e` |
| checksum copy | `tools/postgresql-17.11.tar.gz.sha256` |

xtask 除 size/SHA 外还机械检查：

- URL 只能在 PGDG `v17.11` exact HTTPS directory；
- 一个 `postgresql-17.11` source root；
- 只额外允许一个 exact `pax_global_header`，类型必须是 PAX global extension 且 ≤4 KiB；
- entry ≤20,000、展开总量 ≤512 MiB、无绝对路径/`..`/第二根/非 file-directory entry；
- `COPYRIGHT` 含 PGDG 与许可正文锚点；
- `configure`、`src/backend/tcop/postgres.c`、`src/bin/initdb/initdb.c`、`src/bin/pg_ctl/pg_ctl.c` 必须存在；
- pin、官方 checksum copy 与 Desktop runtime 的 version/source SHA 三向相等。

下载只落 ignored `target/postgres/source/`，后续 `fetch-source` 可复用已校 archive；它不是发行物，也不是 app 首启网络路径。

## 4. Release bundle attestation

显式 `postgres-sidecar` feature 才编译 manifest codec/getrandom/sha2，默认 typed in-process graph不变。`VerifiedPostgresBundle::open` 要求：

- bundle root 与 `manifest.json` 均为非 symlink 正确类型，manifest ≤1 MiB；
- caller 从 signed outer release metadata 提供 expected manifest SHA-256；digest 不符时先于 JSON shape 失败；
- host 以 `ReviewedPostgresSigningIdentity::from_reviewed_release` 断言已评审 signing identity；空值、控制字符、>256 bytes 与 §23.4 禁用 mark 拒绝；
- schema/version、当前platform、`aarch64|x86_64`、PG17.11、PGDG source SHA、`ENGINE_RELEASE_EPOCH=1`、当前 core version 与 signing identity exact；
- program path按平台固定为 `bin/{postgres,initdb,pg_ctl}`，Windows加`.exe`；Unix program必须有execute bit；
- manifest记录3–8192个文件，实际普通文件总量≤2 GiB；除manifest自身外，manifest key集合与实际文件集合双向相等；
- 每个路径只能含normal component，所有文件在hash时再次拒绝symlink/非普通文件，SHA必须lowercase64hex且逐文件一致。

manifest只“记录”signing identity；本批不冒充平台 codesign/Authenticode verification。真实binary build/sign与manifest生成仍属于后续 release assembly。

## 5. Single-instance start lock

`PostgresStartLock::acquire` 只接受 absolute、非 symlink、Unix group/other bits=0 的 app-data root与64位lowercase instance ID。lock closed bytes绑定：

```text
openbot-postgres-start-lock-v1
pid=<当前PID>
instance=<instance id>
manifest=<verified manifest sha256>
nonce=<128-bit CSPRNG lowercase hex>
```

文件 `create_new` 原子创建，Unix 0600，写满后 file fsync，再 directory fsync。第二个 owner 或 crash residue统一 `postgres_sidecar_start_lock_held`，不自动删。Drop先按expected长度做有界读取，文件必须仍是非symlink普通文件且完整字节与自身nonce一致才删除；replacement/symlink/不同内容留在原位，避免旧guard删除新owner或攻击者文件。

lock不是secret；nonce只用于ownership fencing。OS key store里的随机SCRAM secret是下一批独立边界。

## 6. 首跑失败与修正

- 第一次 `--locked` test在编译前拒绝：Desktop新增现有direct dependency edge需要更新lock metadata；offline更新后确认Cargo package `823→823`，没有下载新crate；
- xtask pin单测首跑因误用`toml::Value::from_str`解析完整文档失败，并报unused import/多余mut；改`toml::from_str`并清警告；
- sandbox source fetch因DNS拒绝失败，不计通过；宿主联网下载后size/SHA先通过；
- tar closed-root首跑暴露普通list隐藏的`pax_global_header`，收成exact单一bounded PAX规则；
- 下一次暴露我猜错`postgres.c`路径，亲读archive后改为真实`src/backend/tcop/postgres.c`；
- bounded lock compare加入后，最终矩阵首编译暴露`File::by_ref`在Read/Write间歧义；显式`std::io::Read::by_ref`后通过；
- 一次fmt check指出单行机械换行，执行`cargo fmt --all`后最终check通过。

上述均不计成功；最终证据如下。

## 7. 本轮亲跑证据

| 证据 | 结果 |
| --- | --- |
| Desktop all-feature | `113/0/0`；postgres bundle/lock子集`4/0/0`；doc-test `0/0/1 ignored` |
| xtask完整bin | `95/0/0`；postgres pin子集`2/0/0` |
| `cargo xtask postgres fetch-source` | sandbox DNS失败不计；宿主下载后最终reuse+verify通过 |
| `cargo xtask postgres verify-source` | `17.11 / 28,397,423 / 5367f6fb…9a7e`通过 |
| Clippy | Desktop all-target/all-feature、xtask bin均`-D warnings`通过 |
| cross target | Windows MSVC all-feature check通过；Linux GNU feature-only check通过，但打印既有budget/transport dead-code 2 warning，不冒充Clippy/runtime |
| Tauri dependency guard | 通过；Linux host graph absent，既有MPL/UNIC/Vet blocker不变 |
| parity | `813/881/1694`、0 violation、required revalidate=0；fixtures `17/22/39`、overlay `1444/242/2/6` |
| strict recount | clean pinned upstream `891df72f…`，`159/0/0` |
| Grok | Git tree=`86f5a85f560f721677fa7e587a67ac0ffc036cb5`，diff0；inventory 2,110 files |
| dependency/package | Cargo.lock只增既有getrandom/sha2 direct edge，package `823→823`；非Grok恰一个`package.json`；npm=0 |

本批无API/UI/T-ID/CSS/locale变化，没有重跑Trunk、Browser、Engine/golden。没有运行 `cargo xtask ci`，没有派发GitHub Actions。

## 8. 未闭合边界

- macOS arm64/x64、Windows x64 PostgreSQL binary尚未从source reproducibly build、逐文件hash、codesign/Authenticode并生成真实manifest；
- actual reviewed signing identity/产品identity尚无值，测试identity只用于closed contract；
- macOS Keychain、Windows Credential Manager、Linux Secret Service adapter尚未实现；
- SCRAM secret生成/读取/rotation尚未接 `SecretBytes`，不允许退回env/文件；
- `initdb`/postgres/pg_ctl version execution、process identity、ready deadline、graceful shutdown、crash/orphan recovery、backup/upgrade尚未实现；
- crash residue当前fail-closed，不以PID猜测自动清理；
- Batch79 installation/bootstrap尚未与本批bundle/lock组合，更未进入真实Tauri setup/window；
- 本批不修改`grok-bot/`，不新增Grok产品能力或npm。

下一批应先落OS key-store secret port+macOS/Windows adapter，或在已评审平台binary build产物上实现`VerifiedPostgresBundle`→lock→version→initdb/spawn→ready→Batch79 bootstrap→graceful shutdown；任一缺失时不得创建native window。
