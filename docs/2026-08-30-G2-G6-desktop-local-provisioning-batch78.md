# Batch78：Desktop Local PostgreSQL principal 与 package membership

> 日期：2026-08-30（America/Los_Angeles）
>
> 分支：`feat/2026-08-30-G2-G6-desktop-local-provisioning`
>
> base：`018e959861aa12e340143e3b5e87424baa52c88a`（PR #60 merge commit）
>
> implementation：`e47932debc254a50700f644b7b88e6162b7a578c`
>
> 第一真源：v4 §5.2–§5.3、§6.1、§6.5、§13.1–§13.3、§14.1、§15.3、§24 G2/G6、§28.1 R151/R152；GUI v2 §15.1。

## 1. 结论

本批把 Batch77 的 Desktop Local app-instance authority 接入既有 PostgreSQL 身份与 Tenant Package 机制：

- 复用并参数化 Server single-user 已有的 canonical principal 事务内核，不复制第二套 SQL，也不改变 Server 固定 actor/profile 与 disabled 零连接语义；
- Desktop Local 使用独立固定 actor `desktop-local-user`、non-routable canonical email 与中性 display name；
- 同一事务原子 upsert user、保留既有 `auth_generation`，并把 role 集合收敛为唯一 `admin`；
- authority 生成恰指向自身 actor 的 single-user `TenantPackageAudienceContext`，再由既有 package synchronizer 物化 channel membership；
- 真实 PostgreSQL 17.11 纵向测试覆盖 authority file → canonical principal → repair → package sync → membership。

本批仍没有 Tauri setup、真实 native window load、app-data 与 sidecar data-dir 同根约束或可发布 binary，因此不关闭完整 Desktop Local 启动 journey，也不勾 G2/G6 整关。

## 2. 根因与 ownership

Batch77 只在内存中产出 `AuthContext`。若不把同一 authority 物化为 PostgreSQL `users` / `user_roles`，后续 package synchronization 无法形成 materialized channel membership；若直接复用 Server 的 `dev-local-user`，又会混淆固定上游兼容身份与每用户、每 app-instance 的 Desktop Local deployment。

既有 `single_user::initialize_single_user` 已经拥有正确的事务语义：canonical profile 可修复、generation 不回退、roles 删除后收敛到唯一 admin、冲突时整事务回滚。本批把该事务内核提升为认证模块内的共享私有函数：Server wrapper 继续固定传入原有键，Desktop authority 只传自己的 canonical profile。transport 与 window lifecycle 都不直接操作用户表。

Package audience 仍由 application 层既有 `TenantPackageAudienceContext::single_user` 裁决；infra authority 只提供精确 actor，不重写 §6.5 的 `all` / 具名组 / single-user 规则。

## 3. 实现

### 3.1 Canonical principal 事务

`initialize_canonical_principal(pool, actor_id, email, name)` 保持在 `auth::single_user` 父模块私有：

- 连接后开启单一事务；
- `INSERT ... ON CONFLICT(id) DO UPDATE` 只恢复 canonical email/name，不覆盖 `auth_generation`；
- 复用 domain `plan_set_role(Admin)` 删除旧 role 后写唯一 admin；
- 任一步失败都不提交。

Server `initialize_single_user(pool, enabled)` 在 `enabled=false` 时仍于取连接前返回；开启时固定传 `dev-local-user` 原值。Desktop 没有获得修改 Server compatibility key 的入口。

### 3.2 Desktop Local profile 与 audience

Desktop authority 固定：

- actor：`desktop-local-user`；
- email：`desktop-local@localhost.invalid`；
- name：`Desktop Local User`。

email 使用 `.invalid` 保留域，不能被当成真实可投递地址；display name 不猜测 OS account name。`provision_postgres` 与 `tenant_package_audience_context` 都是既有 authority 的方法，因而 principal、package audience 与 Batch77 持久 instance 的 deployment/tenant/actor 保持同源。

### 3.3 预期启动顺序

本批把后续 setup 的依赖顺序固定为：

1. 从当前 OS 用户 app-data root load/create Desktop authority；
2. 应用 PostgreSQL baseline/native schema；
3. provision/repair canonical Desktop principal；
4. 用同一 authority 的 single-user audience 同步 Tenant Package；
5. 最后才把 verified authority 绑定给 native window lifecycle。

第 1、3、4 步已有模块级真实纵向证据；第 2、5 步尚未在 Tauri binary setup 中接线，不能据此宣称完整启动已完成。

## 4. 真实 PostgreSQL 纵向证据

本轮使用本机 Homebrew PostgreSQL 17.11，在显式临时目录创建专用 cluster，并只监听 `127.0.0.1:55478`：

- sandbox 内首次 `initdb` 因 SysV shared-memory `EPERM` 失败；随后在宿主权限下以 POSIX shared memory 初始化成功，失败首跑不计为通过；
- local trust socket 仅用于给专用测试账号设置一次性口令；两组 Rust integration tests 均经 TCP SCRAM 连接；
- Desktop 纵向测试首次创建 canonical user/sole admin，随后故意改坏 email/name、把 generation 改为 7 并添加 `user` role；再次 provision 后 canonical profile 与 sole admin 恢复、generation 仍为 7；
- 同一 authority 同步一个 `allowed_groups: [all]` 的 Tenant Package，实得 membership grant=1，数据库存在该 actor 的 channel membership，报告标记 single-user groups ignored；
- 既有 Server `dev_actor` 三条真实 PG 测试全部通过，证明 disabled no-connect、canonical repair 与 email collision 23505/rollback 语义没有回归；
- cluster 最后 `fast` stop，显式临时目录已删除，不留测试数据库或构建外数据。

## 5. 本轮亲跑证据

| 证据 | 结果 |
| --- | --- |
| Desktop PG vertical：`cargo test -p openbot-infra --test desktop_local_authority -- --include-ignored` | `1/0/0` |
| Server compatibility PG：`cargo test -p openbot-infra --test dev_actor -- --include-ignored` | `3/0/0` |
| Infra完整lib（sandbox首跑） | `300/15/0`；15项均为既有loopback bind `EPERM`，不记为通过 |
| Infra完整lib（宿主重跑） | `315/0/0` |
| Clippy：`cargo clippy -p openbot-infra --all-targets --all-features --locked -- -D warnings` | 通过 |
| parity | `813/881/1694`、0 violation、required revalidate=0；fixtures `17/22/39`、overlay `1445/241/2/6` |
| strict recount | clean pinned upstream `891df72f…`，`159/0/0` |
| Grok | tree=`86f5a85f560f721677fa7e587a67ac0ffc036cb5`，diff0；inventory 2,110 files |
| invariants | Cargo.lock/workflow/dependency diff0；非Grok恰一个`package.json`；新增npm/package=0 |

Windows完整infra图按既有 `openssl-sys` / `samael` 限制未运行，不能从macOS结果推定Windows PostgreSQL或filesystem行为。本批无T-ID/UI/CSS/locale变化，没有重跑Trunk、Browser、Engine或golden。没有运行 `cargo xtask ci`，没有派发GitHub Actions。

## 6. 未闭合边界

- Tauri binary尚未从真实 `app_data_dir()` 构造 authority，也没有执行上述 setup 顺序；
- app-data identity与sidecar PostgreSQL data-dir尚未强制同一 per-user/per-instance root；
- authority尚未交给 Batch76 `VerifiedDesktopWindowAuthority` 做真实Wry/WebView2 window load；
- Windows per-user DACL、NTFS与PostgreSQL sidecar没有真机证据；
- Desktop Remote仍需Server session authority，不得复用Local profile；
- 可发布binary、reviewed identity、sidecar生命周期、golden与G6整关仍todo；
- 本批没有新增产品能力、API、UI、T-ID、npm、package或dependency，也没有修改 `grok-bot/`。

下一批应接 Tauri setup 与 app-data/sidecar data-dir 同根约束；只有真实 binary setup 能按既定顺序启动并把同一 authority 绑定到 window 后，才可继续关闭 Desktop Local 正式 journey。
