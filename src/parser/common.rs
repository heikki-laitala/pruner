//! Shared helpers used by more than one language extractor.

pub(super) fn node_text<'a>(node: tree_sitter::Node<'a>, src: &'a [u8]) -> &'a str {
    node.utf8_text(src).unwrap_or("")
}

/// Normalize a type name to its simple base name for call resolution.
/// e.g. `Box<String>` -> `Box`, `com.foo.User` -> `User`, `List<Map<K,V>>` -> `List`
pub(super) fn normalize_type_name(raw: &str) -> String {
    let without_generics = raw.split('<').next().unwrap_or(raw);
    let simple = without_generics
        .rsplit('.')
        .next()
        .unwrap_or(without_generics);
    simple.to_string()
}
