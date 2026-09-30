use crate::utils::is_nc_name;
use oxiri::Iri;
use quick_xml::Decoder;
use quick_xml::escape::unescape_with;
use quick_xml::name::{NamespaceBindingsIter, PrefixDeclaration};
use std::borrow::Cow;

/// Iterator on the file prefixes.
///
/// See [`ReaderRdfXmlParser::prefixes`](crate::ReaderRdfXmlParser::prefixes).
pub struct RdfXmlPrefixesIter<'a> {
    pub(super) inner: NamespaceBindingsIter<'a>,
    pub(super) decoder: Decoder,
    pub(super) lenient: bool,
}

impl<'a> Iterator for RdfXmlPrefixesIter<'a> {
    type Item = (&'a str, &'a str);

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        loop {
            let (key, value) = self.inner.next()?;
            return Some((
                match key {
                    PrefixDeclaration::Default => "",
                    PrefixDeclaration::Named(name) => {
                        let Ok(Cow::Borrowed(name)) = self.decoder.decode(name) else {
                            continue;
                        };
                        let Ok(Cow::Borrowed(name)) = unescape_with(name, |_| None) else {
                            continue;
                        };
                        if !self.lenient && !is_nc_name(name) {
                            continue; // We don't return invalid prefixes
                        }
                        name
                    }
                },
                {
                    let Ok(Cow::Borrowed(value)) = self.decoder.decode(value.0) else {
                        continue;
                    };
                    let Ok(Cow::Borrowed(value)) = unescape_with(value, |_| None) else {
                        continue;
                    };
                    if !self.lenient && Iri::parse(value).is_err() {
                        continue; // We don't return invalid prefixes
                    }
                    value
                },
            ));
        }
    }

    #[inline]
    fn size_hint(&self) -> (usize, Option<usize>) {
        self.inner.size_hint()
    }
}
