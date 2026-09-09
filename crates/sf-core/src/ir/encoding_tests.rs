use super::*;

#[test]
fn iri_substitution_escapes_non_ucschar_as_utf8_not_codepoints() {
    let recipe = Template::parse("http://ex/{v}").unwrap();
    for (input, encoded) in [
        ("\u{80}", "%C2%80"),
        ("\u{9f}", "%C2%9F"),
        ("\u{a0}", "\u{a0}"),
        ("\u{d7ff}", "\u{d7ff}"),
        ("\u{e000}", "%EE%80%80"),
        ("\u{f8ff}", "%EF%A3%BF"),
        ("\u{f900}", "\u{f900}"),
        ("\u{fdcf}", "\u{fdcf}"),
        ("\u{fdd0}", "%EF%B7%90"),
        ("\u{fdef}", "%EF%B7%AF"),
        ("\u{fdf0}", "\u{fdf0}"),
        ("\u{ffef}", "\u{ffef}"),
        ("\u{fff0}", "%EF%BF%B0"),
        ("\u{ffff}", "%EF%BF%BF"),
        ("\u{10000}", "\u{10000}"),
        ("\u{1fffd}", "\u{1fffd}"),
        ("\u{1fffe}", "%F0%9F%BF%BE"),
        ("\u{e0000}", "%F3%A0%80%80"),
        ("\u{e1000}", "\u{e1000}"),
        ("\u{efffd}", "\u{efffd}"),
        ("\u{f0000}", "%F3%B0%80%80"),
        ("\u{10ffff}", "%F4%8F%BF%BF"),
        ("你好/%EE%80%80", "你好%2F%25EE%2580%2580"),
        ("a\0b", "a%00b"),
    ] {
        let mut out = String::new();
        assert!(recipe.expand(&[("v", Some(input))][..], true, &mut out));
        assert_eq!(out, format!("http://ex/{encoded}"), "{input:?}");
        assert!(recipe.expand(&[("v", Some(input))][..], false, &mut out));
        assert_eq!(
            out,
            format!("http://ex/{input}"),
            "literal templates never encode"
        );
    }
}
