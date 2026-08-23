# Cargo Vet 基线（R45）

本目录由 `cargo-vet 0.10.0` 生成并由 CI 以 `cargo vet --locked` 消费。建立时的
`Cargo.lock` SHA-256 为
`0a6639f094d099c19aa9f2b3a13871ac408149c942c62e0fb16260764b51ab81`。

## 读口径

- `imports.lock` 锁定 Google 公开的 exact/delta audits；当前机械覆盖 **14** 个依赖。
- `config.toml` 的 **350** 条 exemptions 是“引入 Cargo Vet 时已存在的精确版本快照”，
  **不是安全审计结论**。
- CI 不运行 `init` / `regenerate`。Cargo.lock 新增或升级一个未覆盖版本时，
  `cargo vet --locked` 直接判红，不会自动添加 exemption。

复算：

```bash
cargo vet --version                                                   # cargo-vet 0.10.0
cargo vet --locked                                                    # 14 fully audited, 350 exempted
python3 -c 'import tomllib;d=tomllib.load(open("supply-chain/config.toml","rb"));print(sum(len(v) for v in d.get("exemptions",{}).values()))'  # 350
```

## 直接信任源裁决

本轮只导入 Cargo Vet 官方 registry 登记的 Google 审计集。它在 0.10.0 下可完整
解析，且对当前图实际提供 14 条 `safe-to-deploy` exact/delta 证据。

本轮不导入 Mozilla / Bytecode Alliance：前者的当前集合混合 publisher/wildcard 动态
信任，后者在 0.10.0 下实得 80 条无效审计告警。在工具和映射边界未单独审查
之前，不把“registry 里有名字”冒充为本项目已直接信任。

## 更新程序

1. 审查 Cargo.lock 变化与新依赖的用途。
2. 显式运行 `cargo vet regenerate imports`，逐行 review `imports.lock` 的新审计与来源。
3. 优先完成/导入审计；只有记录 owner 和理由后才能运行
   `cargo vet regenerate exemptions`。
4. 运行 `cargo vet fmt` 和 `cargo vet --locked`，并把 audit/exemption 数字与本文同 PR 更新。

负向对照：本轮在临时基线中只删除 `aead 0.5.2` exemption 后重跑 `--locked`，
实得 `Vetting Failed` 且精确报 `aead:0.5.2 missing ["safe-to-deploy"]`，证明该闸门会说“不”。
