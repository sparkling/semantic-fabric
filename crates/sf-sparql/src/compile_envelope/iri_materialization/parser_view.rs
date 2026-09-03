use std::borrow::Cow;
use std::str::Chars;

/// Mirrors spargebra 0.4.6's `standard-unicode-escaping` preprocessing.
///
/// Invalid or incomplete escapes are replayed unchanged, including the
/// leading slash. A valid escape can introduce grammar-significant syntax;
/// callers must therefore scan this view rather than the submitted bytes.
pub(super) fn parser_view(input: &str) -> Cow<'_, str> {
    if needs_unicode_decode(input) {
        UnicodeDecoder::new(input).collect()
    } else {
        Cow::Borrowed(input)
    }
}

fn needs_unicode_decode(input: &str) -> bool {
    input
        .as_bytes()
        .windows(2)
        .any(|pair| pair[0] == b'\\' && matches!(pair[1], b'u' | b'U'))
}

struct UnicodeDecoder<'input> {
    chars: Chars<'input>,
    replay: String,
}

impl<'input> UnicodeDecoder<'input> {
    fn new(input: &'input str) -> Self {
        Self {
            chars: input.chars(),
            replay: String::with_capacity(9),
        }
    }

    fn decode<const WIDTH: usize>(&mut self, marker: char) -> char {
        self.replay.push(marker);
        for _ in 0..WIDTH {
            let Some(next) = self.chars.next() else {
                return '\\';
            };
            self.replay.push(next);
        }
        let decoded = u32::from_str_radix(&self.replay[1..], 16)
            .ok()
            .and_then(char::from_u32);
        if decoded.is_some() {
            self.replay.clear();
        }
        decoded.unwrap_or('\\')
    }
}

impl Iterator for UnicodeDecoder<'_> {
    type Item = char;

    fn next(&mut self) -> Option<Self::Item> {
        let next = if self.replay.is_empty() {
            self.chars.next()?
        } else {
            self.replay.remove(0)
        };
        if next != '\\' {
            return Some(next);
        }
        match self.chars.next() {
            Some('u') => Some(self.decode::<4>('u')),
            Some('U') => Some(self.decode::<8>('U')),
            Some(other) => {
                self.replay.push(other);
                Some('\\')
            }
            None => Some('\\'),
        }
    }
}
