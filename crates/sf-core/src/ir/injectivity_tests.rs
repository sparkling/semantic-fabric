use super::*;

#[test]
fn unreserved_and_unicode_separators_do_not_prove_injectivity() {
    for separator in ["x", "-", ".", "_", "~", "é", "中", "%2F"] {
        let template = Template::parse(&format!("http://ex/{{a}}{separator}{{b}}")).unwrap();
        assert!(!template.is_injective(), "{separator}");
    }
    let template = Template::parse("http://ex/{a}-{b}").unwrap();
    let mut first = String::new();
    let mut second = String::new();
    assert!(template.expand(&[("a", Some("x-")), ("b", Some("y"))][..], true, &mut first));
    assert!(template.expand(
        &[("a", Some("x")), ("b", Some("-y"))][..],
        true,
        &mut second
    ));
    assert_eq!(first, second);
}

#[test]
fn encoded_delimiters_prove_injectivity_even_with_split_literal_segments() {
    for separator in ["/", ":", "=", "!", "#", "?", "-/-", "é/中"] {
        let template = Template::parse(&format!("http://ex/{{a}}{separator}{{b}}")).unwrap();
        assert!(template.is_injective(), "{separator}");
    }
    let template = Template::from_segments(vec![
        Segment::Column("a".into()),
        Segment::Literal("-".into()),
        Segment::Literal("/".into()),
        Segment::Literal("".into()),
        Segment::Column("b".into()),
    ])
    .unwrap();
    assert!(template.is_injective());
}

#[test]
fn percent_triplets_and_multiple_non_delimiters_cannot_prove_separation() {
    let template = Template::parse("{a}%{b}").unwrap();
    assert!(!template.is_injective());
    let mut first = String::new();
    let mut second = String::new();
    assert!(template.expand(
        &[("a", Some("0")), ("b", Some("20 "))][..],
        true,
        &mut first
    ));
    assert!(template.expand(
        &[("a", Some("0 ")), ("b", Some("20"))][..],
        true,
        &mut second
    ));
    assert_eq!(first, "0%20%20");
    assert_eq!(first, second);
    let template = Template::from_segments(vec![
        Segment::Column("a".into()),
        Segment::Literal("-".into()),
        Segment::Literal("".into()),
        Segment::Literal("x".into()),
        Segment::Column("b".into()),
    ])
    .unwrap();
    assert!(!template.is_injective());
    assert!(Template::parse("{a}/{a}").unwrap().is_injective());
}
