use std::io::{self, Read};

#[derive(Debug, Default, Clone, Copy)]
pub struct Counts {
    pub lines: u64,
    pub words: u64,
    pub bytes: u64,
    pub chars: u64,
    pub max_line_len: u64,
}

impl Counts {
    pub fn add(&mut self, other: &Counts) {
        self.lines += other.lines;
        self.words += other.words;
        self.bytes += other.bytes;
        self.chars += other.chars;
        if other.max_line_len > self.max_line_len {
            self.max_line_len = other.max_line_len;
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct CountFlags {
    pub lines: bool,
    pub words: bool,
    pub bytes: bool,
    pub chars: bool,
    pub max_line_len: bool,
}

impl CountFlags {
    pub fn default_if_none(mut self) -> Self {
        if !self.lines && !self.words && !self.bytes && !self.chars && !self.max_line_len {
            self.lines = true;
            self.words = true;
            self.bytes = true;
        }
        self
    }

    pub fn needs_words(&self) -> bool {
        self.words
    }

    pub fn needs_chars(&self) -> bool {
        self.chars
    }

    pub fn needs_max_line_len(&self) -> bool {
        self.max_line_len
    }
}

pub fn count_reader<R: Read>(reader: R, flags: CountFlags) -> io::Result<Counts> {
    let flags = flags.default_if_none();

    if flags.needs_chars() || flags.needs_max_line_len() {
        count_full(reader, flags)
    } else if flags.needs_words() {
        count_lwb(reader)
    } else {
        count_lines_bytes(reader)
    }
}

/// Full counting: lines, words, bytes, chars, max_line_len
fn count_full<R: Read>(mut reader: R, flags: CountFlags) -> io::Result<Counts> {
    let mut counts = Counts::default();
    let mut buf = [0u8; 64 * 1024];
    let mut in_word = false;
    let mut current_line_len: u64 = 0;
    let need_words = flags.needs_words();
    let need_chars = flags.needs_chars();
    let need_max_line = flags.needs_max_line_len();

    loop {
        let n = reader.read(&mut buf)?;
        if n == 0 {
            break;
        }
        counts.bytes += n as u64;

        for &b in &buf[..n] {
            if b == b'\n' {
                counts.lines += 1;
                if need_max_line {
                    if current_line_len > counts.max_line_len {
                        counts.max_line_len = current_line_len;
                    }
                    current_line_len = 0;
                }
            } else if need_max_line {
                // Count display width: only count bytes that start a character
                if b < 0x80 {
                    if b == b'\t' {
                        // GNU wc counts tab as advancing to next 8-col stop
                        current_line_len = (current_line_len + 8) & !7;
                    } else if b == b'\r' {
                        current_line_len = 0;
                    } else {
                        current_line_len += 1;
                    }
                } else if (b & 0xC0) != 0x80 {
                    // Start of a multibyte UTF-8 char
                    current_line_len += 1;
                }
                // continuation bytes (10xxxxxx) don't advance the column
            }

            if need_words {
                update_word_state(b, &mut in_word, &mut counts.words);
            }

            if need_chars {
                if (b & 0xC0) != 0x80 {
                    counts.chars += 1;
                }
            }
        }
    }

    if need_max_line && current_line_len > counts.max_line_len {
        counts.max_line_len = current_line_len;
    }

    Ok(counts)
}

/// Classify byte for word-state machine (matches GNU wc's locale-aware behavior)
#[inline(always)]
fn update_word_state(b: u8, in_word: &mut bool, word_count: &mut u64) {
    let is_ws = b == b' ' || b == b'\t' || b == b'\n' || b == b'\r'
        || b == b'\x0b' || b == b'\x0c';
    if is_ws {
        *in_word = false;
    } else if is_word_byte(b) && !*in_word {
        *in_word = true;
        *word_count += 1;
    }
    // Control chars, continuation bytes, invalid bytes: no state change
}

#[inline(always)]
fn is_word_byte(b: u8) -> bool {
    // Printable ASCII (0x21-0x7E) or UTF-8 leading bytes (0xC2-0xF4)
    (b > 0x20 && b < 0x7F) || (b >= 0xC2 && b <= 0xF4)
}

/// Count lines, words, bytes only (no char/max-line-len overhead)
fn count_lwb<R: Read>(mut reader: R) -> io::Result<Counts> {
    let mut counts = Counts::default();
    let mut buf = [0u8; 64 * 1024];
    let mut in_word = false;

    loop {
        let n = reader.read(&mut buf)?;
        if n == 0 {
            break;
        }
        counts.bytes += n as u64;

        for &b in &buf[..n] {
            if b == b'\n' {
                counts.lines += 1;
            }
            update_word_state(b, &mut in_word, &mut counts.words);
        }
    }

    Ok(counts)
}

/// Count only lines and bytes (fastest non-trivial path)
fn count_lines_bytes<R: Read>(mut reader: R) -> io::Result<Counts> {
    let mut counts = Counts::default();
    let mut buf = [0u8; 64 * 1024];

    loop {
        let n = reader.read(&mut buf)?;
        if n == 0 {
            break;
        }
        counts.bytes += n as u64;
        counts.lines += memchr::memchr_iter(b'\n', &buf[..n]).count() as u64;
    }

    Ok(counts)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn count_str(s: &str, flags: CountFlags) -> Counts {
        count_reader(s.as_bytes(), flags).unwrap()
    }

    fn all_flags() -> CountFlags {
        CountFlags {
            lines: true,
            words: true,
            bytes: true,
            chars: true,
            max_line_len: true,
        }
    }

    #[test]
    fn empty() {
        let c = count_str("", all_flags());
        assert_eq!(c.lines, 0);
        assert_eq!(c.words, 0);
        assert_eq!(c.bytes, 0);
        assert_eq!(c.chars, 0);
        assert_eq!(c.max_line_len, 0);
    }

    #[test]
    fn single_line_no_newline() {
        let c = count_str("hello world", all_flags());
        assert_eq!(c.lines, 0);
        assert_eq!(c.words, 2);
        assert_eq!(c.bytes, 11);
        assert_eq!(c.chars, 11);
        assert_eq!(c.max_line_len, 11);
    }

    #[test]
    fn single_line_with_newline() {
        let c = count_str("hello world\n", all_flags());
        assert_eq!(c.lines, 1);
        assert_eq!(c.words, 2);
        assert_eq!(c.bytes, 12);
        assert_eq!(c.chars, 12);
        assert_eq!(c.max_line_len, 11);
    }

    #[test]
    fn multiple_lines() {
        let c = count_str("one\ntwo three\nfour\n", all_flags());
        assert_eq!(c.lines, 3);
        assert_eq!(c.words, 4);
        assert_eq!(c.bytes, 19);
    }

    #[test]
    fn utf8_chars() {
        let c = count_str("cafe\u{0301}\n", all_flags()); // "café" with combining accent
        assert_eq!(c.lines, 1);
        assert_eq!(c.words, 1);
        assert_eq!(c.bytes, 7); // c(1) a(1) f(1) e(1) \u{0301}(2) \n(1)
        assert_eq!(c.chars, 6); // 5 codepoints + newline
    }

    #[test]
    fn whitespace_variants() {
        let c = count_str("a\tb\x0bc\x0cd\re\n", all_flags());
        assert_eq!(c.words, 5);
    }

    #[test]
    fn control_chars_not_words() {
        let c = count_str("\x01\x02\x03", all_flags());
        assert_eq!(c.words, 0);
    }

    #[test]
    fn control_chars_dont_split_words() {
        let c = count_str("\x01hello\x02world\x01", all_flags());
        assert_eq!(c.words, 1);
    }
}
