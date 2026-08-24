# W-7b SAML / xmlsec / OpenSSL / libxml2 FFI delta 审计

> 日期：2026-08-23（America/Los_Angeles）。
> 修订登记：第一真源 §28.1 R50。
> 边界：这是 `samael 0.0.22` + `xmlsec` 的依赖/FFI/协议实现证据，不是外部安全审计。

## 1. 为什么不能关闭 xmlsec

第一真源 §6.2 固定 `samael 0.0.22`，同时要求签名覆盖、Destination、Audience、Recipient、
`InResponseTo`、时间窗、assertion replay，并拒绝 SHA-1、外部实体与 unsigned response/assertion。
`samael` 的默认 feature 是 `xmlsec`；关闭它只剩序列化/反序列化，无法验证 XMLDSig。故本仓精确写：

```toml
samael = { version = "=0.0.22", default-features = false, features = ["xmlsec"] }
```

`default-features = false` 不是关验签，而是把启用面写成显式单项；guard 要求 `xmlsec` 真在 feature 图。

## 2. 负向构建与本机闭包

安装 xmlsec 前实跑：

```text
Failed to get --cflags from xmlsec1-config. Is xmlsec1 installed?
```

本机原本已有 OpenSSL 3.6.3、系统 libxml2 2.9.13 与 Xcode `libclang.dylib`，缺口精确是
`xmlsec1-config`。安装 `libxmlsec1 1.3.12` 后，Homebrew 同时提供 libxml2 2.15.3；无额外
Cargo 环境变量即可构建。

第一次真实 XMLDSig 测试却 `SIGSEGV`。`otool -L`/`DYLD_PRINT_LIBRARIES=1` 证明同一进程加载：

```text
/opt/homebrew/.../libxml2.16.dylib   # xmlsec1 的 2.15.3
/usr/lib/libxml2.2.dylib             # Rust libxml crate 经 Xcode pkg-config 的 2.9.x
```

两份 libxml 全局状态跨 ABI 混用是崩溃根因。`.cargo/config.toml` 因而按 Apple 架构把 Rust
link search 固定到 Homebrew 标准前缀；清理 `libxml`/`samael` 后重建，测试二进制只加载
`/opt/homebrew/opt/libxml2/lib/libxml2.16.dylib`，同一真实 XMLDSig 正负矩阵通过。

这不是“本机装上就行”的偶然：重复 libxml 动态库是机器可复核的拒绝条件。

## 3. Cargo.lock 精确差集

相对 W-7a 提交 `44fc82c`，新增 **31** 个精确版本、删除 0：

```text
adler2 2.0.1
bindgen 0.72.1
cexpr 0.6.0
clang-sys 1.9.1
crc32fast 1.5.1
darling 0.20.11
darling_core 0.20.11
darling_macro 0.20.11
data-encoding 2.11.1
derive_builder 0.20.2
derive_builder_core 0.20.2
derive_builder_macro 0.20.2
flate2 1.1.9
fnv 1.0.7
foreign-types 0.3.2
foreign-types-shared 0.1.1
glob 0.3.4
libloading 0.8.9
libxml 0.3.3
miniz_oxide 0.8.9
openssl 0.10.81
openssl-macros 0.1.1
openssl-probe 0.1.6
openssl-sys 0.9.117
pkg-config 0.3.34
prettyplease 0.2.37
quick-xml 0.41.0
samael 0.0.22
shlex 1.3.0
simd-adler32 0.3.10
vcpkg 0.2.15
```

Cargo Vet 刷新 Google imports 后 `openssl-macros 0.1.1` 由 exact+delta audit 覆盖；其余从
31/31 unvetted 降到 **30/31**。30 条逐版本 exemption 均写
`owner=security`、`not a full source audit` 与“外部 SAML/XSW 审计仍必需”。当前结果为
`15 fully audited / 400 exempted`；不能把 400 写成“已审”。

## 4. 八份 build script

加白名单前 `cargo deny check` 精确报 8 个 `build-script-not-allowed`：

| crate | 行 | SHA-256 | 实际行为 |
| --- | ---: | --- | --- |
| bindgen 0.72.1 | 29 | `f7a10af0a21662e104e0058da7e3471a20be328eef6c7c41988525be90fdfe92` | 只在 OUT_DIR 写 host-target.txt，登记 libclang 相关 env |
| clang-sys 1.9.1 | 53 | `1d3a13cba52050a62c1d420431d2c8dd2f96919e1b4a6cc7faf13a84d807838b` | 定位/链接 libclang；可调用 llvm-config |
| crc32fast 1.5.1 | 35 | `deb6052c4a586e8875ef677fe2d9e9dcfafebfa949803455ea8eff6e7dbde436` | 执行 `$RUSTC --version` 后 emit cfg |
| libxml 0.3.3 | 60 | `8541d3886b77064f5ea766010c7b1db3603a4b0d9488d49d53b0d9c3f7daaed1` | LIBXML2/pkg-config/vcpkg 定位 libxml2 |
| openssl 0.10.81 | 167 | `ee2656bba4668b5850a6ff638a56910dbc555c0f1e574c3b4210a45a3ea98382` | 将 openssl-sys 的版本/config 映射为 cfg |
| openssl-sys 0.9.117 | 551 | `3a7f63b3c446451801ac03c34e03051b8d3ebcceea28b8fb2688922255629d27` | pkg-config/vcpkg/环境定位；cc 预处理 expando.c 校验 header。本图未开 vendored/bindgen |
| prettyplease 0.2.37 | 21 | `79a5b2d260aa97aeac7105fbfa00774982f825cd708c100ea96d01c39974bb88` | 只读 package version 后 emit |
| samael 0.0.22 | 88 | `f83b4cd2151cdee8356812427dd27f279ee5fd9e36bddab2c97f6bbd85ebb8cc` | 执行 xmlsec1-config 两次，bindgen 读 headers，只写 OUT_DIR |

samael build 在本机生成 `xmlsec_bindings.rs` **51,495 行**；这说明构建答案依赖原生 headers，
不是纯 Rust 可复现。脚本本身无 HTTP/download。crate 唯一 executable 是
`test_vectors/multi_saml_response.sh`（SHA-256
`ab091e9a22bc13290acfc03ad2b7ff372465d8e8926330cfa1e56d4d57ccfd2f`），仅用于维护者生成
双签名测试向量，build/production 源码零引用；deny bypass 只放这一条路径。

## 5. 协议收口

本仓没有把 `ServiceProvider::parse_xml_response` 当万能黑盒，而是在其前后加硬闸门：

- 输入/base64/解码 XML 均有 512 KiB 上限，XML 深度 64、元素 20,000、单元素属性 64；
- OIDC issuer 与实际 SSO/ACS endpoint 最长 4 KiB；SAML EntityID 是最长 1024 字节的
  `urn/http/https` 绝对标识，不误套 OIDC“必须 HTTPS host”规则，真正导航 URL 仍只许 HTTPS；
- quick-xml 先做严格良构检查；拒 DTD、ENTITY、PI、NUL、多 root、错 namespace；所以 libxml
  默认 parser 没机会解析外部实体；
- response 必须有且只有一个根级 XMLDSig，Reference 必须唯一且逐字指向 Response ID；
- `ValidateAndMarkNoAncestors` 输出的根必须仍是 SAML Response；只签 assertion 的外层
  Destination 不受保护，明确拒绝；
- SignedInfo/Reference 各恰一个；canonicalization 只收 exclusive-c14n，transform 顺序固定
  enveloped-signature → exclusive-c14n；DigestMethod 只收 SHA-256/384/512；
- SignatureMethod 只收 RSA/ECDSA SHA-256/384/512；SHA-1、SHA-224、DSA 不在类型 allowlist；
- signed Response 与 Assertion 分别精确校验 issuer、Version、IssueInstant；
- Destination/Recipient/`InResponseTo` 逐字等于配置 ACS/request；
- `AudienceRestriction` 非空，多个 restriction 按 SAML 的 **AND** 语义全部命中 SP entity；
- Conditions/SubjectConfirmation 时间窗取两者最早 expiry；有效期最多 10 分钟，另显式验证
  AuthnInstant 不在未来、AuthnContext 存在、SessionNotOnOrAfter 未过期；多年有效 assertion 拒绝；
- bearer confirmation 若携带本仓无法权威核对的 Address/扩展 content 则拒绝；email claim value
  最多 16 个、group value 最多 256 个且单值最多 4 KiB，防签名 IdP 响应把 session 投影撑爆；
- assertion ID 在 HMAC(provider, issuer, ID) 作用域写 PostgreSQL 一次性 replay 行；过期时间覆盖
  assertion 的完整有效窗与 2 分钟 clock-skew 尾窗，不再用固定 10 分钟提前截短；第二个合法
  request 重用同 assertion ID 也被拒；
- SP-initiated only；不实现 Single Logout，因而没有未签名 LogoutRequest 或 metadata form
  action 的 XSS 面。

真实测试不是“解析一个 struct”：本机动态生成 RSA-SHA256 XMLDSig，验证成功；随后逐项对照
unsigned、SHA-1、错 Destination/Audience/Recipient/`InResponseTo`/时间窗；另有 PG17 三条
动态闭环（SAML replay+session、动态 OIDC 跨 replica、legacy plaintext→v2）和真 Axum admin
注册/更新/兼容删除/列表/admin 删除。负向另含超长 assertion lifetime、未来 AuthnInstant、历史库
动态 provider 抢占环境 `google` ID、organization-scoped 行被错误放大为 deployment-owned；旧域名
` Legacy.Example ` 同事务规范成 `legacy.example` 后才可路由。HTTP 矩阵逐条走 register/update/delete 三路的未登录 401、
普通用户 403、fresh admin 200，并以畸形 delete body 抓到过一次 400-before-auth 后修成 guard 先行；
另钉 stale admin 401+稳定 code、匿名 SSO start 202，以及 dynamic-only 装配的登录 Origin/cookie
策略不再错误依附环境 OIDC coordinator；内部 route cookie 固定 SameSite=Strict。

最终机械汇总：workspace `991 passed / 0 failed / 64 ignored`；PostgreSQL 17/SCRAM + 本机
TLS/XMLDSig 的 infra+server `451/0/0`（314+137）；严格 recount `147/147`。W-7b 对应
`parity/tests.yaml` 11 条与 API 2 条转 done；`test-inventory` 重生成器按稳定 ID 保留非 todo
证据，已用固定上游克隆真重生成一次，11 条未被抹回 todo。

## 6. 2026-06 SSO 接管公告的四条反例

实施时复核了 Better Auth 官方高危公告 `GHSA-prpr-5gj3-qqhg`，不是照译固定上游旧代码：

1. domain 只收 1–16 个 bare canonical domain；URL/path/query/fragment/空项/重复项拒绝；注册写面
   仍要求 live fresh admin + trusted Origin。
2. provider ID 与环境 Google/Microsoft/Okta、credential/email-password/anonymous/sso 保留名隔离。
3. 更新/删除 provider 时，在同一 people+SSO 锁事务中推进所有关联 user generation、删除 session，
   并在释放 provider ID 前删除 `accounts` anchors；重注册不能继承旧受害者链接。
4. SAML 三个 SP 绑定值全部验证；Single Logout 未开放。

来源：

- https://github.com/better-auth/better-auth/security/advisories/GHSA-prpr-5gj3-qqhg
- https://better-auth.com/docs/plugins/sso
- https://github.com/njaremko/samael
- https://github.com/lsh123/xmlsec
- https://openssl-library.org/news/vulnerabilities/index.html

## 7. OpenSSL 当前 advisory 边界

macOS 实测 OpenSSL 3.6.3。官方在 2026-08-05/13 发布两条尚待 3.6.4 的低危公告：

- CVE-2026-54876：显式启用 OCSP response checking 的 TLS client 泄漏；
- CVE-2026-14456：QUIC server pending channel 无上限。

本仓 safe dialer TLS 仍是 rustls；OpenSSL 只在 `auth/sso/saml.rs` 做 X509 DER 结构校验及
xmlsec XMLDSig crypto backend，全仓无 `openssl::ssl/ocsp/pkcs12/quic` 调用，xmlsec 也不建
TLS/QUIC listener。故两条触发面不可达；guard 锁调用面和 macOS 3.6.3。**这不是漏洞不存在**：
OpenSSL 3.6.4 可获得时立即删本裁决、升级 native pin 并重跑动态链接/签名矩阵。

## 8. 尚未解除

1. samael 为 0.0.x，且含大量 libxml/xmlsec unsafe FFI；第一次独立 SAML/XML signature
   wrapping 外审尚未发生，所以 §24 G2 仍不得标绿。
2. 本机只真跑 macOS arm64；Linux CI 从未得到额度执行，Windows 的 `xmlsec1-config` 构建路径
   尚未兑现。Server OCI 的 Linux 原生包版本/镜像 digest 与 bindgen 输出仍需可复现发行批次钉死。
3. Server v2 KEK 此刻由既有 `KEY_ENCRYPTION_KEY` 提供；KMS/HSM adapter 与多版本 key ring 尚未
   落地。v1/plaintext→v2 的同事务写/回读已闭合，但这不等于 §6.4 全 vault 交付。
4. GUI 设计系统与 identity-provider 表单要到 G6 才实施；本批只交付生产 HTTP/API 语义。
5. 当前不接收 SP private key，metadata 若声明 `WantAuthnRequestsSigned=true` 会启动/注册拒绝；固定上游
   GUI 的真实注册体也没有 privateKey。上游加密单测里 synthetic `privateKey` 已在 parity 账本按替代
   口径说明，不能把“整个已接受 SAML config 都加密”写成“已支持 signed AuthnRequest”。若 G8 的
   Better Auth wildcard 兼容最终要求该能力，必须与 Server KMS/HSM 私钥托管同批实现和外审。
