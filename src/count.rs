use std::io::{self, Read};

const IS_WHITESPACE: u8 = 1;
const IS_WORD: u8 = 2;
const IS_NEWLINE: u8 = 4;

static BYTE_CLASS: [u8; 256] = {
    let mut table = [0u8; 256];
    // Whitespace: \t(9), \n(10), \v(11), \f(12), \r(13), space(32)
    table[9] = IS_WHITESPACE;
    table[10] = IS_WHITESPACE | IS_NEWLINE;
    table[11] = IS_WHITESPACE;
    table[12] = IS_WHITESPACE;
    table[13] = IS_WHITESPACE;
    table[32] = IS_WHITESPACE;
    // Printable ASCII: 0x21-0x7E
    let mut i = 0x21;
    while i <= 0x7E {
        table[i] = IS_WORD;
        i += 1;
    }
    // UTF-8 leading bytes: 0xC2-0xF4
    let mut i = 0xC2;
    while i <= 0xF4 {
        table[i] = IS_WORD;
        i += 1;
    }
    table
};

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

pub fn count_bytes(data: &[u8], flags: CountFlags) -> Counts {
    let flags = flags.default_if_none();

    if flags.needs_chars() || flags.needs_max_line_len() {
        count_bytes_full(data, flags)
    } else if flags.needs_words() {
        count_bytes_lwb(data)
    } else {
        count_bytes_lines(data)
    }
}

fn count_bytes_lines(data: &[u8]) -> Counts {
    Counts {
        lines: memchr::memchr_iter(b'\n', data).count() as u64,
        bytes: data.len() as u64,
        ..Counts::default()
    }
}

#[cfg(target_arch = "x86_64")]
fn count_words_lines_simd(data: &[u8]) -> (u64, u64) {
    if is_x86_feature_detected!("avx2") {
        unsafe { count_words_lines_avx2(data) }
    } else {
        let words = unsafe { count_words_sse2(data) };
        let lines = memchr::memchr_iter(b'\n', data).count() as u64;
        (words, lines)
    }
}

#[cfg(not(target_arch = "x86_64"))]
fn count_words_lines_simd(data: &[u8]) -> (u64, u64) {
    let words = count_words_scalar(data);
    let lines = memchr::memchr_iter(b'\n', data).count() as u64;
    (words, lines)
}

#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2")]
unsafe fn count_words_lines_avx2(data: &[u8]) -> (u64, u64) {
    use std::arch::x86_64::*;

    let mut words: u64 = 0;
    let mut lines: u64 = 0;
    let mut prev_not_word_carry: u32 = 1;

    let x20 = _mm256_set1_epi8(0x20i8);
    let xc1 = _mm256_set1_epi8(0xC1u8 as i8);
    let xf5 = _mm256_set1_epi8(0xF5u8 as i8);
    let newline = _mm256_set1_epi8(b'\n' as i8);

    let chunks = data.chunks_exact(32);
    let remainder = chunks.remainder();

    for chunk in chunks {
        let v = _mm256_loadu_si256(chunk.as_ptr() as *const __m256i);

        // Line counting
        let nl_vec = _mm256_cmpeq_epi8(v, newline);
        let nl_bits = _mm256_movemask_epi8(nl_vec) as u32;
        lines += nl_bits.count_ones() as u64;

        // Word byte detection
        let gt_x20 = _mm256_cmpgt_epi8(v, x20);
        let gt_xc1 = _mm256_cmpgt_epi8(v, xc1);
        let lt_xf5 = _mm256_cmpgt_epi8(xf5, v);
        let utf8_lead = _mm256_and_si256(gt_xc1, lt_xf5);
        let word_vec = _mm256_or_si256(gt_x20, utf8_lead);
        let word_bits = _mm256_movemask_epi8(word_vec) as u32;

        let prev_not_word = ((!word_bits) << 1) | prev_not_word_carry;
        let word_starts = prev_not_word & word_bits;
        words += word_starts.count_ones() as u64;
        prev_not_word_carry = if word_bits & (1 << 31) != 0 { 0 } else { 1 };
    }

    let mut prev_was_space = prev_not_word_carry != 0;
    for &b in remainder {
        if b == b'\n' { lines += 1; }
        let class = BYTE_CLASS[b as usize];
        if class & IS_WHITESPACE != 0 {
            prev_was_space = true;
        } else if class & IS_WORD != 0 {
            if prev_was_space { words += 1; }
            prev_was_space = false;
        }
    }

    (words, lines)
}

#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "sse2")]
unsafe fn count_words_sse2(data: &[u8]) -> u64 {
    use std::arch::x86_64::*;

    let mut words: u64 = 0;
    let mut prev_not_word_carry: u32 = 1;

    let x20 = _mm_set1_epi8(0x20i8);
    let xc1 = _mm_set1_epi8(0xC1u8 as i8);
    let xf5 = _mm_set1_epi8(0xF5u8 as i8);

    let chunks = data.chunks_exact(16);
    let remainder = chunks.remainder();

    for chunk in chunks {
        let v = _mm_loadu_si128(chunk.as_ptr() as *const __m128i);

        let gt_x20 = _mm_cmpgt_epi8(v, x20);
        let gt_xc1 = _mm_cmpgt_epi8(v, xc1);
        let lt_xf5 = _mm_cmplt_epi8(v, xf5);
        let utf8_lead = _mm_and_si128(gt_xc1, lt_xf5);
        let word_vec = _mm_or_si128(gt_x20, utf8_lead);
        let word_bits = _mm_movemask_epi8(word_vec) as u32;

        let prev_not_word = ((!word_bits) << 1) | prev_not_word_carry;
        let word_starts = prev_not_word & word_bits;
        words += word_starts.count_ones() as u64;
        prev_not_word_carry = if word_bits & (1 << 15) != 0 { 0 } else { 1 };
    }

    let mut prev_was_space = prev_not_word_carry != 0;
    for &b in remainder {
        let class = BYTE_CLASS[b as usize];
        if class & IS_WHITESPACE != 0 {
            prev_was_space = true;
        } else if class & IS_WORD != 0 {
            if prev_was_space { words += 1; }
            prev_was_space = false;
        }
    }

    words
}

#[cfg(not(target_arch = "x86_64"))]
fn count_words_simd(data: &[u8]) -> u64 {
    count_words_scalar(data)
}

#[allow(dead_code)]
fn count_words_scalar(data: &[u8]) -> u64 {
    let mut words: u64 = 0;
    let mut in_word = false;
    for &b in data {
        let class = BYTE_CLASS[b as usize];
        if class & IS_WHITESPACE != 0 {
            in_word = false;
        } else if class & IS_WORD != 0 && !in_word {
            in_word = true;
            words += 1;
        }
    }
    words
}

fn count_bytes_lwb(data: &[u8]) -> Counts {
    let (words, lines) = count_words_lines_simd(data);
    Counts {
        lines,
        words,
        bytes: data.len() as u64,
        ..Counts::default()
    }
}

fn count_bytes_full(data: &[u8], flags: CountFlags) -> Counts {
    let mut counts = Counts {
        bytes: data.len() as u64,
        ..Counts::default()
    };
    let mut in_word = false;
    let mut current_line_len: u64 = 0;
    let need_words = flags.needs_words();
    let need_chars = flags.needs_chars();
    let need_max_line = flags.needs_max_line_len();

    for &b in data {
        if b == b'\n' {
            counts.lines += 1;
            if need_max_line {
                if current_line_len > counts.max_line_len {
                    counts.max_line_len = current_line_len;
                }
                current_line_len = 0;
            }
        } else if need_max_line {
            if b < 0x80 {
                if b == b'\t' {
                    current_line_len = (current_line_len + 8) & !7;
                } else if b == b'\r' {
                    current_line_len = 0;
                } else {
                    current_line_len += 1;
                }
            } else if (b & 0xC0) != 0x80 {
                current_line_len += 1;
            }
        }
        if need_words {
            update_word_state(b, &mut in_word, &mut counts.words);
        }
        if need_chars && (b & 0xC0) != 0x80 {
            counts.chars += 1;
        }
    }
    if need_max_line && current_line_len > counts.max_line_len {
        counts.max_line_len = current_line_len;
    }
    counts
}

/// Full counting: lines, words, bytes, chars, max_line_len
fn count_full<R: Read>(mut reader: R, flags: CountFlags) -> io::Result<Counts> {
    let mut counts = Counts::default();
    let mut buf = vec![0u8; 256 * 1024];
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

#[inline(always)]
fn update_word_state(b: u8, in_word: &mut bool, word_count: &mut u64) {
    let class = BYTE_CLASS[b as usize];
    if class & IS_WHITESPACE != 0 {
        *in_word = false;
    } else if class & IS_WORD != 0 && !*in_word {
        *in_word = true;
        *word_count += 1;
    }
}

/// Count lines, words, bytes only (no char/max-line-len overhead)
fn count_lwb<R: Read>(mut reader: R) -> io::Result<Counts> {
    let mut counts = Counts::default();
    let mut buf = vec![0u8; 256 * 1024];
    let mut in_word = false;

    loop {
        let n = reader.read(&mut buf)?;
        if n == 0 {
            break;
        }
        let chunk = &buf[..n];
        counts.bytes += n as u64;

        // Use SIMD memchr for newline counting
        counts.lines += memchr::memchr_iter(b'\n', chunk).count() as u64;

        // Word counting still needs byte-by-byte state machine
        for &b in chunk {
            let class = BYTE_CLASS[b as usize];
            if class & IS_WHITESPACE != 0 {
                in_word = false;
            } else if class & IS_WORD != 0 && !in_word {
                in_word = true;
                counts.words += 1;
            }
        }
    }

    Ok(counts)
}

/// Count only lines and bytes (fastest non-trivial path)
fn count_lines_bytes<R: Read>(mut reader: R) -> io::Result<Counts> {
    let mut counts = Counts::default();
    let mut buf = vec![0u8; 256 * 1024];

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
