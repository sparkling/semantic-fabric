pub(super) enum Prologue<'input> {
    Base { iri: &'input str },
    Prefix { name: &'input str, iri: &'input str },
    Version,
}

pub(super) enum BodyToken<'input> {
    IriRef(&'input str),
    PrefixedName {
        prefix: &'input str,
        local: &'input str,
    },
}

pub(super) struct Scanner<'input> {
    input: &'input str,
    cursor: usize,
    prologue_done: bool,
}

impl<'input> Scanner<'input> {
    pub(super) fn new(input: &'input str) -> Self {
        Self {
            input,
            cursor: 0,
            prologue_done: false,
        }
    }

    pub(super) fn next_prologue(&mut self) -> Option<Prologue<'input>> {
        if self.prologue_done {
            return None;
        }
        self.skip_layout();
        let start = self.cursor;

        if self.take_keyword("BASE") {
            self.skip_layout();
            if let Some(iri) = self.take_iriref() {
                return Some(Prologue::Base { iri });
            }
        }
        self.cursor = start;

        if self.take_keyword("PREFIX") {
            self.skip_layout();
            if let Some((name, end)) = scan_pname_ns(self.input, self.cursor) {
                self.cursor = end;
                self.skip_layout();
                if let Some(iri) = self.take_iriref() {
                    return Some(Prologue::Prefix { name, iri });
                }
            }
        }
        self.cursor = start;

        if self.take_keyword("VERSION") {
            self.skip_layout();
            if self.take_short_string() {
                return Some(Prologue::Version);
            }
        }

        self.cursor = start;
        self.prologue_done = true;
        None
    }

    pub(super) fn next_body(&mut self) -> Option<BodyToken<'input>> {
        self.prologue_done = true;
        while self.cursor < self.input.len() {
            self.skip_layout();
            if self.cursor >= self.input.len() {
                return None;
            }
            match self.byte(0) {
                Some(b'\'' | b'"') => {
                    self.skip_string();
                    continue;
                }
                Some(b'<') => {
                    let start = self.cursor;
                    if let Some(iri) = self.take_iriref() {
                        return Some(BodyToken::IriRef(iri));
                    }
                    self.cursor = next_char(self.input, start);
                    continue;
                }
                Some(b'?' | b'$') => {
                    self.skip_variable();
                    continue;
                }
                Some(b'_') if self.byte(1) == Some(b':') => {
                    self.skip_blank_node_label();
                    continue;
                }
                _ => {}
            }

            if let Some(name) = scan_prefixed_name(self.input, self.cursor) {
                self.cursor = name.end;
                return Some(BodyToken::PrefixedName {
                    prefix: name.prefix,
                    local: name.local,
                });
            }
            self.cursor = next_char(self.input, self.cursor);
        }
        None
    }

    fn take_keyword(&mut self, keyword: &str) -> bool {
        let Some(end) = self.cursor.checked_add(keyword.len()) else {
            return false;
        };
        if self
            .input
            .get(self.cursor..end)
            .is_some_and(|word| word.eq_ignore_ascii_case(keyword))
        {
            self.cursor = end;
            true
        } else {
            false
        }
    }

    fn take_iriref(&mut self) -> Option<&'input str> {
        if self.byte(0) != Some(b'<') {
            return None;
        }
        let content = self.cursor + 1;
        let relative_end = self.input.as_bytes()[content..]
            .iter()
            .position(|byte| *byte == b'>')?;
        let end = content + relative_end;
        self.cursor = end + 1;
        self.input.get(content..end)
    }

    fn take_short_string(&mut self) -> bool {
        let Some(quote @ (b'\'' | b'"')) = self.byte(0) else {
            return false;
        };
        self.cursor += 1;
        while let Some(byte) = self.byte(0) {
            match byte {
                b'\\' => {
                    self.cursor += 1;
                    if self.cursor < self.input.len() {
                        self.cursor = next_char(self.input, self.cursor);
                    }
                }
                byte if byte == quote => {
                    self.cursor += 1;
                    return true;
                }
                b'\n' | b'\r' => return false,
                _ => self.cursor = next_char(self.input, self.cursor),
            }
        }
        false
    }

    fn skip_layout(&mut self) {
        loop {
            while self.byte(0).is_some_and(is_sparql_whitespace) {
                self.cursor += 1;
            }
            if self.byte(0) != Some(b'#') {
                return;
            }
            while let Some(byte) = self.byte(0) {
                if matches!(byte, b'\n' | b'\r') {
                    break;
                }
                self.cursor = next_char(self.input, self.cursor);
            }
        }
    }

    fn skip_string(&mut self) {
        let quote = self.byte(0).expect("caller checked quote");
        let long = self.byte(1) == Some(quote) && self.byte(2) == Some(quote);
        self.cursor += if long { 3 } else { 1 };
        while let Some(byte) = self.byte(0) {
            if byte == b'\\' {
                self.cursor += 1;
                if self.cursor < self.input.len() {
                    self.cursor = next_char(self.input, self.cursor);
                }
            } else if long
                && byte == quote
                && self.byte(1) == Some(quote)
                && self.byte(2) == Some(quote)
            {
                self.cursor += 3;
                return;
            } else if !long && byte == quote {
                self.cursor += 1;
                return;
            } else if !long && matches!(byte, b'\n' | b'\r') {
                return;
            } else {
                self.cursor = next_char(self.input, self.cursor);
            }
        }
    }

    fn skip_variable(&mut self) {
        self.cursor += 1;
        while let Some(character) = self.input[self.cursor..].chars().next() {
            if !is_varname_char(character) {
                return;
            }
            self.cursor += character.len_utf8();
        }
    }

    fn skip_blank_node_label(&mut self) {
        self.cursor += 2;
        while let Some(character) = self.input[self.cursor..].chars().next() {
            if !(is_pn_chars(character) || character == '.') {
                return;
            }
            self.cursor += character.len_utf8();
        }
    }

    fn byte(&self, offset: usize) -> Option<u8> {
        self.input.as_bytes().get(self.cursor + offset).copied()
    }
}

struct PrefixedName<'input> {
    prefix: &'input str,
    local: &'input str,
    end: usize,
}

fn scan_prefixed_name(input: &str, start: usize) -> Option<PrefixedName<'_>> {
    let (prefix, local_start) = scan_pname_ns(input, start)?;
    let end = scan_pname_local(input, local_start);
    Some(PrefixedName {
        prefix,
        local: &input[local_start..end],
        end,
    })
}

fn scan_pname_ns(input: &str, start: usize) -> Option<(&str, usize)> {
    if input.as_bytes().get(start) == Some(&b':') {
        return Some((&input[start..start], start + 1));
    }
    let mut cursor = start;
    let first = input.get(cursor..)?.chars().next()?;
    if !is_pn_chars_base(first) {
        return None;
    }
    cursor += first.len_utf8();
    let mut trailing_dot = false;
    while let Some(character) = input.get(cursor..)?.chars().next() {
        if is_pn_chars(character) {
            trailing_dot = false;
            cursor += character.len_utf8();
        } else if character == '.' {
            trailing_dot = true;
            cursor += 1;
        } else {
            break;
        }
    }
    if trailing_dot || input.as_bytes().get(cursor) != Some(&b':') {
        return None;
    }
    Some((&input[start..cursor], cursor + 1))
}

fn scan_pname_local(input: &str, start: usize) -> usize {
    let mut cursor = start;
    let mut consumed = false;
    let mut last_non_dot = start;
    while cursor < input.len() {
        if let Some(width) = plx_width(input, cursor) {
            cursor += width;
            last_non_dot = cursor;
            consumed = true;
            continue;
        }
        let character = input[cursor..].chars().next().expect("valid cursor");
        let allowed = if consumed {
            is_pn_chars(character) || character == ':' || character == '.'
        } else {
            is_pn_chars_u(character) || character == ':' || character.is_ascii_digit()
        };
        if !allowed {
            break;
        }
        cursor += character.len_utf8();
        consumed = true;
        if character != '.' {
            last_non_dot = cursor;
        }
    }
    if consumed {
        last_non_dot
    } else {
        start
    }
}

fn plx_width(input: &str, cursor: usize) -> Option<usize> {
    match input.as_bytes().get(cursor).copied()? {
        b'%' if input
            .as_bytes()
            .get(cursor + 1..cursor + 3)
            .is_some_and(|digits| digits.iter().all(u8::is_ascii_hexdigit)) =>
        {
            Some(3)
        }
        b'\\'
            if input
                .as_bytes()
                .get(cursor + 1)
                .is_some_and(|byte| b"_~.-!$&'()*+,;=/?#@%".contains(byte)) =>
        {
            Some(2)
        }
        _ => None,
    }
}

fn is_sparql_whitespace(byte: u8) -> bool {
    matches!(byte, b' ' | b'\t' | b'\n' | b'\r')
}

fn is_varname_char(character: char) -> bool {
    character.is_ascii_digit()
        || is_pn_chars_u(character)
        || character == '\u{00B7}'
        || ('\u{0300}'..='\u{036F}').contains(&character)
        || ('\u{203F}'..='\u{2040}').contains(&character)
}

fn is_pn_chars(character: char) -> bool {
    is_pn_chars_u(character)
        || character == '-'
        || character.is_ascii_digit()
        || character == '\u{00B7}'
        || ('\u{0300}'..='\u{036F}').contains(&character)
        || ('\u{203F}'..='\u{2040}').contains(&character)
}

fn is_pn_chars_u(character: char) -> bool {
    character == '_' || is_pn_chars_base(character)
}

fn is_pn_chars_base(character: char) -> bool {
    character.is_ascii_alphabetic()
        || ('\u{00C0}'..='\u{00D6}').contains(&character)
        || ('\u{00D8}'..='\u{00F6}').contains(&character)
        || ('\u{00F8}'..='\u{02FF}').contains(&character)
        || ('\u{0370}'..='\u{037D}').contains(&character)
        || ('\u{037F}'..='\u{1FFF}').contains(&character)
        || ('\u{200C}'..='\u{200D}').contains(&character)
        || ('\u{2070}'..='\u{218F}').contains(&character)
        || ('\u{2C00}'..='\u{2FEF}').contains(&character)
        || ('\u{3001}'..='\u{D7FF}').contains(&character)
        || ('\u{F900}'..='\u{FDCF}').contains(&character)
        || ('\u{FDF0}'..='\u{FFFD}').contains(&character)
}

fn next_char(input: &str, cursor: usize) -> usize {
    cursor
        + input[cursor..]
            .chars()
            .next()
            .expect("cursor must point within input")
            .len_utf8()
}

pub(super) fn unescape_iriref(mut input: &str) -> Option<String> {
    let mut output = String::with_capacity(input.len());
    while let Some((before, after)) = input.split_once('\\') {
        output.push_str(before);
        let mut characters = after.chars();
        let marker = characters.next()?;
        let width = match marker {
            'u' => 4,
            'U' => 8,
            _ => return None,
        };
        let remainder = characters.as_str();
        let digits = remainder.get(..width)?;
        let decoded = u32::from_str_radix(digits, 16)
            .ok()
            .and_then(char::from_u32)?;
        output.push(decoded);
        input = &remainder[width..];
    }
    output.push_str(input);
    Some(output)
}
