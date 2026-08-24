//! 固定上游 JavaScript 文本边界的纯语义。
//!
//! 只收那些必须逐字节对齐 ECMAScript、且会被多个 crate 消费的规则。它不是通用 Unicode
//! 工具箱；当前唯一规则是 `String.prototype.trim()` 使用的 `TrimString` 空白集合。

/// ECMAScript `TrimString` 的 `WhiteSpace ∪ LineTerminator` 封闭集合。
///
/// 不能换成 [`char::is_whitespace`]：Rust 少认 U+FEFF，却多认 ECMAScript 不认的 U+0085。
/// 两个方向都会改变配置、email 与旧 JavaScript 数据的身份语义。
#[must_use]
pub const fn is_ecmascript_whitespace(value: char) -> bool {
    matches!(
        value,
        '\u{0009}'..='\u{000D}'
            | '\u{0020}'
            | '\u{00A0}'
            | '\u{1680}'
            | '\u{2000}'..='\u{200A}'
            | '\u{2028}'
            | '\u{2029}'
            | '\u{202F}'
            | '\u{205F}'
            | '\u{3000}'
            | '\u{FEFF}'
    )
}

/// 与 ECMAScript `String.prototype.trim()` 相同的首尾裁剪。
#[must_use]
pub fn trim_ecmascript(value: &str) -> &str {
    value.trim_matches(is_ecmascript_whitespace)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 两个语言集合不相等的两端都必须被行使，避免退回 `str::trim()` 仍然全绿。
    #[test]
    fn trim_string_includes_bom_and_excludes_next_line() {
        assert_eq!(trim_ecmascript("\u{FEFF}\u{3000}v\u{00A0}"), "v");
        assert_eq!(trim_ecmascript("\u{0085}v\u{0085}"), "\u{0085}v\u{0085}");
        assert!(!'\u{FEFF}'.is_whitespace(), "Rust 的负向对照");
        assert!('\u{0085}'.is_whitespace(), "Rust 的另一方向正向对照");
    }
}
