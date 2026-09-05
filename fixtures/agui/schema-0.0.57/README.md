# AG-UI 0.0.57 官方schema来源

仅作为离线协议fixture数据，不参与TypeScript/Node构建。

固定来源：`https://github.com/ag-ui-protocol/ag-ui`，commit
`54f13419055b4d0f442c71e1efab18b310982ce1`。包版本由该commit的
`sdks/typescript/packages/core/package.json`核对为`@ag-ui/core@0.0.57`。
六份上游文件的Git blob、字节数及SHA-256在相邻provenance逐项登记；主控已从官方Git tree独立复核。
`LICENSE`是该commit根MIT原文；版权行逐字保留，不从package作者字段推测新增版权人。

本地`package-manifest.json`保留上游package.json的原始字节，只更名为惰性资料，
避免在OpenBot引入第二个package.json；它不参与依赖解析或执行。
本README由OpenBot编写，不是上游文件。

事件JSONL由本项目按官方schema确定性构造，是protocol corpus，不是vendor recorded或live trace。
provenance记录39行的三个scenario；合法解析走公共AguiDecoder，事件type集合与官方枚举及Rust常量三向相等。
字段和状态示例仅为测试数据，不含当前工作区路径、客户请求或凭据。

Registry/SLSA URL仅为来源参考；本次没有执行Sigstore签名验证，不能据URL存在宣称已验签。
本次主控未运行npm/npx；生产代码、Cargo依赖与锁文件没有变化。
