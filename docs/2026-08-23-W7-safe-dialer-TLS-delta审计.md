# W-7 HTTP safe dialer / TLS delta 审计

> **形态**：R29 要求的独立 delta audit；不是“加一个库”的附注。
> **日期**：2026-08-23（America/Los_Angeles）。
> **影响面**：v3 §6.2、§7.5、§10.5、§16.3；`openbot-infra::net::safe_http`；OIDC
> discovery/JWKS/token 出网；后续 remote Agent/MCP 共用同一 dialer。
> **修订登记**：v3 §28.1 R48；OIDC/session 竖切另见 R49。G2 仍因动态 IdP/SAML/外审未闭合。

---

## 1. 旧边界与本次必须回答的问题

旧图把 `openidconnect 4.0.1` 设为 `default-features = false`，OIDC discovery/JWKS 只有注入的
GET port；真实 socket、DNS、TLS 与 token POST 都不存在。这是诚实的边界，但不能满足第一真源：

- 每跳最多 3 次 redirect，逐跳重做 IP policy；
- 禁 metadata/loopback/link-local/private/reserved，除非管理员给精确 CIDR；
- 连接必须绑定已验证 DNS 结果，同时 TLS SNI/HTTP Host 仍用原 hostname；
- `Authorization` 只在同 origin redirect 保留；
- body/总墙钟有硬上限；
- discovery/JWKS/token 与未来 remote Agent/MCP 必须是**同一个** dialer。

R29 已证明 TLS 后端无免费选项：rustls 的 first-party provider 都带 C/汇编，native-tls 在 Linux
落回 OpenSSL。因而本次问题不是“要不要有 C”，而是“选哪一块最窄、如何把它明示并持续守住”。

## 2. 候选与裁决

| 候选 | 实测/官方性质 | 裁决 |
| --- | --- | --- |
| reqwest 0.13.4 + rustls | 可禁自动 redirect、覆写 DNS；但仍多一层 proxy/retry/resolver/client policy，连接绑定要靠多项 builder 配置共同成立 | 不选；本项目只需 HTTP/1 framing，不需要高层客户端的第二套策略面 |
| Hyper 1.11 + 显式 `TcpStream` | Hyper/http-body 已在 W-4 锁图；调用方可直接把已验证 `SocketAddr` 交给 `TcpStream::connect`，随后才包 TLS | **选**；DNS rebinding 防线由数据流证明，不由 client 配置约定 |
| native-tls | macOS/Windows 用系统栈，Linux 要 OpenSSL；三平台构建答案不同 | 不选 |
| rustls + aws-lc-rs | rustls 默认且功能完整/含 PQ；官方也说明构建面更重，需 cmake 等工具 | 本轮不选 |
| rustls 0.23.43 + ring 0.17.14 | rustls 官方称其 first-party、跨平台更易构建；功能面较窄、无 PQ；ring 仍含 C/汇编 build.rs | **选**；满足当前 TLS1.2/1.3 IdP/HTTP，构建面小于 aws-lc-rs，但明确接受非纯 Rust代价 |
| 第三方 RustCrypto provider | rustls 官方列表标为 experimental | 不把实验密码 provider 放进生产认证链 |

证书根选择 lockfile 固定的 `webpki-roots 1.0.9`，不读取代理环境或让系统根的机器差异静默改变
认证答案。它牺牲“自动信任企业私有 CA”；私有 CA 只能作为未来**显式配置输入**进入
`SafeDialer::with_extra_roots`，不能靠隐藏宿主状态生效。

## 3. 精确依赖差集

以 `HEAD:Cargo.lock` 对当前 lock 做集合差，本轮新增 **20** 个精确版本、删除 **0**：

```text
ipnet 2.12.1
ring 0.17.14
rustls 0.23.43
rustls-pki-types 1.15.1
rustls-webpki 0.103.15
tokio-rustls 0.26.4
try-lock 0.2.5
untrusted 0.9.0
want 0.3.1
webpki-roots 1.0.9
windows-sys 0.52.0
windows-targets 0.52.6
windows_aarch64_gnullvm 0.52.6
windows_aarch64_msvc 0.52.6
windows_i686_gnu 0.52.6
windows_i686_gnullvm 0.52.6
windows_i686_msvc 0.52.6
windows_x86_64_gnu 0.52.6
windows_x86_64_gnullvm 0.52.6
windows_x86_64_msvc 0.52.6
```

`bytes 1.12.1` / `http-body-util 0.1.5` / `hyper 1.11.0` / `hyper-util 0.1.20` 已在 W-4
Axum 图，本轮只是上收为真实直接依赖，没有新增版本。tokio 只追加 `io-util` feature。

正常生产反向链只有：

```text
ring 0.17.14 <- rustls 0.23.43 <- openbot-infra <- openbot-server
                         ^       <- tokio-rustls 0.26.4
ring 0.17.14 <- rustls-webpki 0.103.15 <- rustls 0.23.43
```

## 4. build.rs / C / 汇编 / 预生成对象

在白名单修改前，`cargo deny check bans` 精确判红两项：

```text
build-script-not-allowed ring 0.17.14
build-script-not-allowed rustls 0.23.43
```

逐份审计：

1. `rustls 0.23.43/build.rs`：13 行；默认图下 `main()` 为空。只有另开本仓未开的
   `read_buf` 且为 nightly 才 emit 一条 cfg。零文件、零进程、零网络。SHA-256：
   `380b9a051325baa7d4957bd9a4f1a637c27a663610b1b502f9524530f6995f4d`。
2. `ring 0.17.14/build.rs`：1,044 行；SHA-256：
   `9d1928ffb1d8e15766c1c1b9ead73e4b81a21703dd25f7c27b87842a2e6e9cee`。
   它通过 `cc::Build` 调目标 C compiler/ar，把 crate 内 C 与预生成汇编编成静态库；源码无
   socket/HTTP/download。crates.io 解包目录没有 `.git`，正常 Cargo 构建固定走
   `pregenerated/`，不执行 Perl/NASM。
3. 必须单列的代价：Windows NASM 目标直接链接 crate 包内 **17 个 `.o`**；这些对象不是本次
   构建从汇编源码重建的。lock checksum 能固定收到的字节，不能冒充“对象可由当前工具链逐字节
   复现”。这也是为什么本文不得把 rustls/ring 写成“纯 Rust TLS”。

`deny.toml` 只在上述事实记录完整后放行 `ring` / `rustls` 两个 build.rs；随后全局
`interpreted=deny` 又精确报出 **38** 份 Perl，`include-archives=true` 让 17 个 `.o` 也进入扫描。
最终 bypass 只匹配 `ring@0.17.14` 的 `crypto/**/*.pl` 与 `pregenerated/*.o`；新路径/新格式/
新版本仍判红。版本、feature 或 build.rs hash 变化均要求 guard 判红并新建 delta audit。

## 5. 许可证差异

- ring：`Apache-2.0 AND ISC`，两项均已在仓库许可证白名单；
- rustls：`Apache-2.0 OR ISC OR MIT`；
- Hyper/tokio-rustls/ipnet 等：MIT/Apache 族；
- `webpki-roots 1.0.9`：**CDLA-Permissive-2.0**，是本轮唯一新增许可证标识。它覆盖 Mozilla
  根证书数据，不是第一方代码许可。该许可允许使用、修改和分发数据，要求保留 notices；因此
  与仓库已接受的宽松许可同族，但必须在 `deny.toml` 明示并进入 NOTICE，不能靠 exception 绕过。

LICENSE 原件全文已进 `NOTICE` §6A，SPDX package/relationship 同步；加白名单前
`cargo deny check licenses` 精确 rejected，接线后 licenses/bans 全绿。

## 6. 运行时协议边界

`openbot-infra::net::safe_http` 当前构造保证：

- HTTP/1 only；无 proxy、自动 retry、自动 redirect、自动 decompression；
- DNS 每跳调用一次；resolver 回来的端口被丢弃，始终用 URL 权威端口；
- 过滤后直接 `TcpStream::connect(SocketAddr)`，并以 `peer_addr == chosen` 做正向对照；
- TLS SNI 与 Host 用原 hostname；TLS ALPN 固定 `http/1.1`，0-RTT 关闭；
- 301/302 对 secret POST 拒绝；303 清空 body 改 GET；307/308 只许同 origin；
- 跨 origin 永远删除 Authorization；第 4 次 redirect 拒绝；
- response body 按 frame 累加，越界前拒绝；一个总 timeout 覆盖 DNS/connect/TLS/header/body。

地址分类以当前 IANA IPv4/IPv6 Special-Purpose registry 为依据。Rust 1.98 的
`IpAddr::is_global` 编译探针仍得 E0658（unstable），所以代码显式编码非 global 段以及
192.0.0.0/24、2001::/23 内更具体的 global 例外；IANA registry 更新时必须同批更新表驱动测试。

## 7. 本轮已实跑的正负对照

截至环境 OIDC 竖切验收：

```text
cargo test -p openbot-infra -p openbot-server --all-features --locked -- --include-ignored
→ 435 passed / 0 failed / 0 ignored（infra 300 + server 135）
cargo vet --locked
→ 14 fully audited / 370 exempted（W-7 新 20 条均明示非 full audit）
```

网络层新增覆盖：IANA global/non-global 表、CIDR 唯一覆盖、loopback 默认拒绝/显式放行、逐跳
重解析与 private rebinding 拒绝、跨 origin Authorization 剥离、4th redirect、读取中 size limit、
总 timeout、CA→leaf SNI 正负；以及 OAuth method/endpoint/header/body 封闭形态与 secret Debug。
PG17 另有跨 replica state/限速、group/generation/session/audit rollback 五条；真实本机 TLS IdP
恰三次请求完成 discovery→PKCE token→JWKS→RS256 claims→session，state 重放未再触达 IdP。

## 8. 复算命令

```bash
# 依赖差集
python3 -c 'import tomllib,subprocess;old=tomllib.loads(subprocess.check_output(["git","show","HEAD:Cargo.lock"],text=True));new=tomllib.load(open("Cargo.lock","rb"));k=lambda p:(p["name"],p["version"]);a={k(p) for p in old["package"]};b={k(p) for p in new["package"]};print(len(b-a),sorted(b-a),len(a-b),sorted(a-b))'

cargo tree -i ring@0.17.14 -e normal --locked
cargo tree -p openbot-infra -e features --locked

# build.rs
RING_DIR=$(find "$HOME/.cargo/registry/src" -maxdepth 2 -type d -name ring-0.17.14 | head -n1)
RUSTLS_DIR=$(find "$HOME/.cargo/registry/src" -maxdepth 2 -type d -name rustls-0.23.43 | head -n1)
wc -l "$RING_DIR/build.rs" "$RUSTLS_DIR/build.rs"                         # 1044 / 13
shasum -a 256 "$RING_DIR/build.rs" "$RUSTLS_DIR/build.rs"
find "$RING_DIR/pregenerated" -type f -name '*.o' | wc -l                 # 17

# 安全与回归
bash tools/check-safe-dialer-dependencies.sh
cargo test -p openbot-infra --all-features --locked --lib
cargo deny check
cargo vet --locked
```

## 9. 未解除边界

1. 本机只实跑 macOS arm64；Linux x64/arm64、macOS x64、Windows MSVC 的 ring C/汇编构建尚未由
   本仓 CI runner 执行。三平台 CI 未绿前不得把“可跨平台构建”写成完成事实。
2. Windows 17 个预生成对象的 source-to-object reproducibility 尚未独立复核；外部安全审计必须把
   它列为供应链观察项。
3. 本文件只解除 safe dialer/TLS 的 R29 前置，不解除 SAML `samael`/OpenSSL/libxml2/xmlsec 的
   **另一份** delta audit，也不替代 G2 第一次外部安全审计。
4. 静态 Mozilla roots 需要显式升级节奏；`webpki-roots` 版本变化必须跟 lock/delta audit/NOTICE 同批。
5. 长寿命环境/runtime 只持有 zeroize 且不可 Clone 的 `SecretBytes`；`openidconnect/oauth2`
   的接口仍要求在单次 callback 内临时物化 `ClientSecret(String)`，库内部临时副本不在
   `SecretBytes` 可擦除范围。它们不落库、不进 Debug/日志，函数返回即 drop；不得把这写成
   “进程历史内存全部已擦除”（R46 的同一边界）。
