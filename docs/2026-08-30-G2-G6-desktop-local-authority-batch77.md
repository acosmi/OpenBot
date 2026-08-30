# Batch77：Desktop Local app-instance authority source

> 日期：2026-08-30（America/Los_Angeles）
>
> 分支：`feat/2026-08-30-G2-G6-desktop-local-authority`
>
> base：`e45eac78bf7915216622f3f5b9a88761a109cb7c`（PR #59 merge commit）
>
> implementation：`fcab8951ece371670307687672ccad698d6fb344`
>
> 第一真源：v4 §5.2–§5.3、§6.1、§13.1–§13.3、§15.3、§24 G2/G6、§28.1 R150/R151；GUI v2 §15.1。

## 1. 结论

本批补齐 Batch76 仍要求caller提供、但仓内不存在的 Desktop Local authority source：

- authority由`openbot-infra`认证适配器铸造，Tauri transport继续只消费结果，不越层调用`AuthContextBuilder`；
- current OS user不从`USER`/`USERNAME`环境变量猜，而由host断言路径来自当前用户的app-data API；
- app instance是一次选举后稳定持久的非秘密256-bit CSPRNG ID；
- deployment/tenant绑定该instance，actor固定`desktop-local-user`，auth generation=0，`single_user=true`，角色为Admin+User；
- Server `OPENBOT_SINGLE_USER`的兼容actor `dev-local-user`不复用；
- identity file为closed v1格式，corrupt/symlink/宽权限fail-closed；
- 双进程/多线程不靠覆盖rename，以完整写+fsync后hard-link noclobber原子选唯一winner；
- matching crash temporary hard link在后续load可安全回收。

本批没有把principal写入PostgreSQL，也没有做package channel membership或Tauri setup接线；因此不关闭完整Desktop Local启动journey。

## 2. 根因与ownership

第一真源把Desktop Local身份固定为“当前OS用户+本地app instance”。仓内已有Server单用户principal，但它是固定上游兼容键`dev-local-user`，服务共享Server部署；直接复用会把两种身份模型混为一谈。

`AuthContextBuilder`文档又明确：transport不能铸造authority。正确依赖方向是infra认证层读取可信host输入并构造`AuthContext`，未来binary只负责把Tauri的per-user app-data path传入，再把结果交给Batch76 lifecycle。

环境变量用户名不是OS peer，可被启动环境覆盖；instance若只在内存生成又会让重启后的thread/memory归属变化。因此本批把“当前用户”绑定在app-data namespace，把“app instance”绑定在持久closed文件。

## 3. Closed identity file

### 3.1 Host断言

`CurrentOsUserAppDataRoot::from_current_os_user_app_data`只接受绝对路径；命名要求调用点说明路径来自当前OS用户API。类型Debug固定`<redacted>`，错误不携路径/用户名/文件内容。

root若是symlink或非目录拒绝。Unix创建/收紧为0700；identity文件要求普通非symlink文件、≤128 bytes且group/other bits为0。

### 3.2 格式与身份

文件恰两行：

```text
openbot-desktop-instance-v1
instance=<64个小写hex>
```

uppercase、长度错误、额外行、错header、坏hex均拒绝。instance是identifier，不是credential；不进入错误或权限判断。

`desktop-local-<instance>`同时作为typed DeploymentId/TenantId；不同app-data instance不会碰撞。唯一actor在该scope内固定，角色由§6.1 single-user裁决直接给Admin+User。

### 3.3 Cross-process noclobber

每个candidate在同目录`create_new` temporary，写满、`sync_all`后才`hard_link(temp, final)`：

- 第一个link成功者成为winner；
- 后续link得到AlreadyExists，只删除自己的temp并读取完整winner；
- final从不被第二个candidate覆盖；
- Unix link前文件mode已是0600，目录link/unlink后fsync；
- winner在link后崩溃留下的同内容temp可由后续load识别并删除；不同candidate temp不被误删。

若文件系统不支持同卷hard link，source返回stable unavailable，不退回有覆盖竞态的rename。

## 4. 测试与负向证明

- 首次创建→重启读取：instance与AuthContext逐字段相等、文件bytes不变；
- 32个相互独立store在Barrier后并发同一root，全部返回同一instance，目录最终唯一identity文件；
- 手工制造与final同inode的crash temp，下一次load回收且authority不变；
- Unix实得root0700/file0600；改成0644后拒绝；
- symlink final、corrupt content、relative root、uppercase、extra line均拒绝；
- source guard确认production源码不读`std::env::var`；
- 完整infra 315条lib测试回归，OIDC/SAML/Server single-user既有语义不变。

## 5. 本轮亲跑证据

| 证据 | 结果 |
| --- | --- |
| `cargo fmt --all -- --check` + `git diff --check` | 通过 |
| 新增targeted首次 | `3/1/0`；唯一失败为source guard命中自身禁词，不记为通过 |
| 新增targeted最终 | `4/0/0` |
| Infra完整lib | `315/0/0` |
| Clippy | 首跑仅固定2-byte `chunks_exact` API建议失败；改`as_chunks::<2>`后all-feature lib `-D warnings`通过 |
| parity | 首跑因`auth/mod.rs`粗前缀红13；改归既有single-user owner、未加overlay后最终`813/881/1694`、0 violation、revalidate=0；fixtures`17/22/39`、overlay`1445/241/2/6` |
| strict recount | clean pinned upstream `891df72f…`，`159/0/0` |
| Grok | tree=`86f5a85f560f721677fa7e587a67ac0ffc036cb5`，diff0；inventory 2,110 files |
| invariants | Cargo.lock/workflow/dependency diff0；非Grok恰一个`package.json`；新增npm/package=0 |

Windows完整infra图按既有`openssl-sys`/`samael`限制未运行，不能把std API可编译推定为NTFS hard-link/DACL真机证据。本批无T-ID/UI/CSS/locale变化，没有重跑Trunk、Browser、Engine或golden。没有运行`cargo xtask ci`，没有派发GitHub Actions。

## 6. 未闭合边界

- Windows per-user app-data DACL、NTFS hard-link与crash行为没有真机证据；
- host尚未实际调用Tauri`app_data_dir()`构造`CurrentOsUserAppDataRoot`；构造函数名是trust assertion，不是路径来源证明；
- PostgreSQL users/user_roles、package channel memberships与canonical local profile尚未由该authority provision；
- authority尚未传给Batch76 `VerifiedDesktopWindowAuthority`，没有真实window load；
- Desktop Remote仍需Server session source；
- `tauri.conf`/capability/binary/reviewed identity、真实Wry/WebView2、golden与G6整关仍todo。

下一批应把该authority与PostgreSQL principal/package membership原子provisioning同批，再接Tauri setup；在真DB证据前不得把文件级AuthContext写成完整Desktop Local登录完成。
