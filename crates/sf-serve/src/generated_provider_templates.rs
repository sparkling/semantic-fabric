//! Compiled-in C2 successor query templates, copied byte for byte from the
//! immutable Query consumer source (revision
//! `b676cf4f32a7148ba51273e8356869550ccea15f`,
//! `assets/semantic-product-mock/`). The pinned digests are checked against
//! these bytes in tests, never fetched or registered at runtime. Rendering
//! matches Query's `enumeration_query`/`root_query`: every placeholder
//! occurrence is replaced and nothing else changes, so no `FROM` is appended
//! and `LIMIT 65` / the 64-style page contract stay exactly as transferred.

pub(crate) const ENUMERATE_ID: &str = "c2-successor-enumerate-v1";
pub(crate) const ENUMERATE_DIGEST: &str =
    "sha256:3459cb4a268ffc242de3c3afcc032f8001792958537b1303901c76a75ae2e2ec";
pub(crate) const ROOT_ID: &str = "c2-successor-root-v1";
pub(crate) const ROOT_DIGEST: &str =
    "sha256:7fb994ca6015966d8be3eb823e61df6080926b0cbcd991b118e86f6f7a74b2a3";
pub(crate) const ENUMERATE_TEMPLATE: &str =
    include_str!("generated_provider_templates/c2-successor-enumerate.sparql");
pub(crate) const ROOT_TEMPLATE: &str =
    include_str!("generated_provider_templates/c2-successor-root.sparql");
pub(crate) const CURSOR_PLACEHOLDER: &str = "{{AFTER_STYLE_NUMBER}}";
pub(crate) const STYLE_PLACEHOLDER: &str = "{{STYLE_NUMBER}}";
pub(crate) const MAX_STYLE_NUMBER_BYTES: usize = 64;

/// The two transferred templates. Closed; no registration exists.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Template {
    Enumerate,
    Root,
}

impl Template {
    pub(crate) const fn id(self) -> &'static str {
        match self {
            Self::Enumerate => ENUMERATE_ID,
            Self::Root => ROOT_ID,
        }
    }

    pub(crate) const fn digest(self) -> &'static str {
        match self {
            Self::Enumerate => ENUMERATE_DIGEST,
            Self::Root => ROOT_DIGEST,
        }
    }

    const fn text(self) -> &'static str {
        match self {
            Self::Enumerate => ENUMERATE_TEMPLATE,
            Self::Root => ROOT_TEMPLATE,
        }
    }

    const fn placeholder(self) -> &'static str {
        match self {
            Self::Enumerate => CURSOR_PLACEHOLDER,
            Self::Root => STYLE_PLACEHOLDER,
        }
    }
}

/// Query's `valid_style_token`: a style identity safe to splice into a quoted
/// SPARQL literal.
pub(crate) fn valid_style_token(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= MAX_STYLE_NUMBER_BYTES
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
}

/// Exact rendered byte length, computed before any allocation.
pub(crate) fn rendered_len(template: Template, parameter: &str) -> Option<usize> {
    let text = template.text();
    let placeholder = template.placeholder();
    let count = text.matches(placeholder).count();
    let removed = count.checked_mul(placeholder.len())?;
    let added = count.checked_mul(parameter.len())?;
    text.len().checked_sub(removed)?.checked_add(added)
}

/// Replace every placeholder occurrence; `capacity` is [`rendered_len`].
pub(crate) fn render(template: Template, parameter: &str, capacity: usize) -> String {
    let mut pieces = template.text().split(template.placeholder());
    let mut out = String::with_capacity(capacity);
    out.push_str(pieces.next().unwrap_or_default());
    for piece in pieces {
        out.push_str(parameter);
        out.push_str(piece);
    }
    out
}
