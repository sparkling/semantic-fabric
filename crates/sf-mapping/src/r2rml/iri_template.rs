//! Compile-time IRI grammar proof; unresolved recipes retain their processor base.
use sf_core::ir::{Segment, Template};

pub(super) fn resolve_iri_template(template: Template, base: &str) -> (Template, bool) {
    if absolute_for_all_rows(&template) {
        return (template, false);
    }
    // Encoded substitutions cannot introduce raw ':', '/', '?' or '#'. Only
    // a literal colon before the first component delimiter can form a scheme.
    let fixed: String = template
        .segments()
        .iter()
        .filter_map(|s| match s {
            Segment::Literal(s) => Some(s.as_ref()),
            Segment::Column(_) => None,
        })
        .collect();
    if !template
        .segments()
        .iter()
        .any(|s| matches!(s, Segment::Column(_)))
        || !fixed
            .split(['/', '?', '#'])
            .next()
            .unwrap_or("")
            .contains(':')
    {
        let mut segments = vec![Segment::Literal(base.into())];
        segments.extend_from_slice(template.segments());
        if let Ok(prefixed) = Template::from_segments(segments) {
            if absolute_for_all_rows(&prefixed) {
                return (prefixed, false);
            }
        }
    }
    (template, true)
}

fn absolute_for_all_rows(template: &Template) -> bool {
    let mut fixed = String::new();
    let mut leading = String::new();
    let mut has_slot = false;
    for segment in template.segments() {
        match segment {
            Segment::Column(_) => has_slot = true,
            Segment::Literal(text) => {
                // No substitution may complete a '%' escape started in fixed
                // text: empty slots alone cannot prove that boundary safe.
                let mut bytes = text.bytes();
                while let Some(byte) = bytes.next() {
                    if byte == b'%'
                        && !(bytes.next().is_some_and(|c| c.is_ascii_hexdigit())
                            && bytes.next().is_some_and(|c| c.is_ascii_hexdigit()))
                    {
                        return false;
                    }
                }
                fixed.push_str(text);
                if !has_slot {
                    leading.push_str(text);
                }
            }
        }
    }
    if sf_core::NamedNode::new(fixed.as_str()).is_err() {
        return false;
    }
    if !has_slot {
        return true;
    }
    let Some((scheme, tail)) = leading.split_once(':') else {
        return false;
    };
    let mut chars = scheme.chars();
    if !chars.next().is_some_and(|c| c.is_ascii_alphabetic())
        || !chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'))
    {
        return false;
    }
    // Require a committed path/query/fragment before the first slot. Neither
    // a scheme nor any part of an authority may depend on a substitution.
    if let Some(authority) = tail.strip_prefix("//") {
        authority.contains(['/', '?', '#'])
    } else {
        !tail.is_empty() && tail != "/"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preserves_static_fast_paths_but_defers_grammar_sensitive_slots() {
        for recipe in [
            "http://e/{id}",
            "urn:fixed-{id}",
            "../{id}",
            "{id}",
            "http://e/?{id}",
            "1:x",
        ] {
            assert!(
                !resolve_iri_template(Template::parse(recipe).unwrap(), "http://base/").1,
                "{recipe}"
            );
        }
        for recipe in [
            "{scheme}:x",
            "ur{suffix}:x",
            "http://host:{port}/x",
            "http://{host}",
            "http:{path}",
            "http:/{path}/x",
            "http://e/%{hex}",
            "http://e/bad value/{x}",
            "http://[v1./]{x}",
            "http://[v1.?]{x}",
            "http://[v1.#]{x}",
        ] {
            let input = Template::parse(recipe).unwrap();
            let (output, late) = resolve_iri_template(input.clone(), "http://base/");
            assert!(late, "{recipe}");
            assert_eq!(output, input);
        }
    }

    #[test]
    fn parser_accepts_encoder_ucschar_boundaries_in_committed_components() {
        for recipe in ["http://e/{x}", "http://e/?{x}", "http://e/#{x}"] {
            let (template, late) =
                resolve_iri_template(Template::parse(recipe).unwrap(), "http://base/");
            assert!(!late);
            for &(start, end) in sf_core::ir::encoding::UCSCHAR_RANGES {
                for codepoint in [start, end] {
                    let value = char::from_u32(codepoint).unwrap().to_string();
                    let mut expanded = String::new();
                    assert!(template.expand(
                        &[("x", Some(value.as_str()))][..],
                        true,
                        &mut expanded
                    ));
                    assert!(
                        sf_core::NamedNode::new(&expanded).is_ok(),
                        "{codepoint:X} in {recipe}"
                    );
                }
            }
        }
    }
}
