use std::io::{self, Read};
use unicode_width::UnicodeWidthChar;

const IS_WHITESPACE: u8 = 1;
const IS_WORD: u8 = 2;
const IS_NEWLINE: u8 = 4;

static BYTE_CLASS: [u8; 256] = {
    let mut table = [0u8; 256];
    table[9] = IS_WHITESPACE;
    table[10] = IS_WHITESPACE | IS_NEWLINE;
    table[11] = IS_WHITESPACE;
    table[12] = IS_WHITESPACE;
    table[13] = IS_WHITESPACE;
    table[32] = IS_WHITESPACE;
    let mut i = 0x21;
    while i <= 0x7E {
        table[i] = IS_WORD;
        i += 1;
    }
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

// --- Public entry points ---

pub fn count_reader<R: Read>(reader: R, flags: CountFlags) -> io::Result<Counts> {
    let flags = flags.default_if_none();
    if flags.needs_max_line_len() {
        count_full_reader(reader, flags)
    } else if flags.needs_chars() || flags.needs_words() {
        count_lwbc_reader(reader, flags.needs_words(), flags.needs_chars())
    } else {
        count_lines_bytes_reader(reader)
    }
}

pub fn count_bytes(data: &[u8], flags: CountFlags) -> Counts {
    let flags = flags.default_if_none();
    if flags.needs_max_line_len() {
        count_bytes_full(data, flags)
    } else if flags.needs_words() {
        let (words, lines) = count_words_lines_simd(data);
        let chars = if flags.needs_chars() { count_chars_simd(data) } else { 0 };
        Counts { lines, words, chars, bytes: data.len() as u64, ..Counts::default() }
    } else if flags.needs_chars() {
        Counts {
            lines: memchr::memchr_iter(b'\n', data).count() as u64,
            chars: count_chars_simd(data),
            bytes: data.len() as u64,
            ..Counts::default()
        }
    } else {
        Counts {
            lines: memchr::memchr_iter(b'\n', data).count() as u64,
            bytes: data.len() as u64,
            ..Counts::default()
        }
    }
}

// --- UTF-8 decoding for -L unicode width ---

fn decode_char_at(data: &[u8], pos: usize) -> (Option<char>, usize) {
    let b = data[pos];
    if b < 0x80 {
        return (Some(b as char), 1);
    }
    let (len, mut cp) = match b {
        0xC2..=0xDF => (2, (b as u32) & 0x1F),
        0xE0..=0xEF => (3, (b as u32) & 0x0F),
        0xF0..=0xF4 => (4, (b as u32) & 0x07),
        _ => return (None, 1),
    };
    if pos + len > data.len() {
        return (None, 1);
    }
    for i in 1..len {
        let cont = data[pos + i];
        if (cont & 0xC0) != 0x80 {
            return (None, i);
        }
        cp = (cp << 6) | ((cont as u32) & 0x3F);
    }
    match char::from_u32(cp) {
        Some(c) => (Some(c), len),
        None => (None, len),
    }
}

fn char_display_width(c: char) -> u64 {
    UnicodeWidthChar::width(c).unwrap_or(0) as u64
}

fn max_line_width_of(data: &[u8]) -> u64 {
    let mut max_w: u64 = 0;
    let mut cur_w: u64 = 0;
    let mut i = 0;
    while i < data.len() {
        let b = data[i];
        if b == b'\n' {
            if cur_w > max_w { max_w = cur_w; }
            cur_w = 0;
            i += 1;
        } else if b == b'\t' {
            cur_w = (cur_w + 8) & !7;
            i += 1;
        } else if b == b'\r' {
            cur_w = 0;
            i += 1;
        } else if b < 0x20 {
            i += 1;
        } else if b < 0x80 {
            cur_w += 1;
            i += 1;
        } else {
            let (ch, len) = decode_char_at(data, i);
            if let Some(c) = ch {
                cur_w += char_display_width(c);
            }
            i += len;
        }
    }
    if cur_w > max_w { max_w = cur_w; }
    max_w
}

// --- SIMD character counting ---

#[cfg(target_arch = "x86_64")]
fn count_chars_simd(data: &[u8]) -> u64 {
    if is_x86_feature_detected!("avx2") {
        unsafe { count_chars_avx2(data) }
    } else {
        count_chars_scalar(data)
    }
}

#[cfg(not(target_arch = "x86_64"))]
fn count_chars_simd(data: &[u8]) -> u64 {
    count_chars_scalar(data)
}

fn count_chars_scalar(data: &[u8]) -> u64 {
    data.iter().filter(|&&b| (b & 0xC0) != 0x80).count() as u64
}

#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2")]
unsafe fn count_chars_avx2(data: &[u8]) -> u64 {
    use std::arch::x86_64::*;
    let mut chars: u64 = 0;
    let threshold = _mm256_set1_epi8(-65i8);
    let chunks = data.chunks_exact(32);
    let remainder = chunks.remainder();
    for chunk in chunks {
        let v = _mm256_loadu_si256(chunk.as_ptr() as *const __m256i);
        let non_cont = _mm256_cmpgt_epi8(v, threshold);
        chars += (_mm256_movemask_epi8(non_cont) as u32).count_ones() as u64;
    }
    for &b in remainder {
        if (b & 0xC0) != 0x80 { chars += 1; }
    }
    chars
}

// --- SIMD word+line counting ---

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
        let nl_vec = _mm256_cmpeq_epi8(v, newline);
        lines += (_mm256_movemask_epi8(nl_vec) as u32).count_ones() as u64;
        let gt_x20 = _mm256_cmpgt_epi8(v, x20);
        let gt_xc1 = _mm256_cmpgt_epi8(v, xc1);
        let lt_xf5 = _mm256_cmpgt_epi8(xf5, v);
        let utf8_lead = _mm256_and_si256(gt_xc1, lt_xf5);
        let word_vec = _mm256_or_si256(gt_x20, utf8_lead);
        let word_bits = _mm256_movemask_epi8(word_vec) as u32;
        let prev_not_word = ((!word_bits) << 1) | prev_not_word_carry;
        words += (prev_not_word & word_bits).count_ones() as u64;
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
        words += (prev_not_word & word_bits).count_ones() as u64;
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
fn count_words_lines_simd_fallback(data: &[u8]) -> (u64, u64) {
    let words = count_words_scalar(data);
    let lines = memchr::memchr_iter(b'\n', data).count() as u64;
    (words, lines)
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

// --- mmap path: full counting with -L using unicode-width ---

fn count_bytes_full(data: &[u8], flags: CountFlags) -> Counts {
    let (words, lines) = if flags.needs_words() {
        count_words_lines_simd(data)
    } else {
        (0, memchr::memchr_iter(b'\n', data).count() as u64)
    };
    let chars = if flags.needs_chars() { count_chars_simd(data) } else { 0 };
    let max_line_len = if flags.needs_max_line_len() { max_line_width_of(data) } else { 0 };
    Counts { lines, words, bytes: data.len() as u64, chars, max_line_len }
}

// --- Reader paths (stdin, pipes, non-regular files) ---

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

fn count_lwbc_reader<R: Read>(mut reader: R, need_words: bool, need_chars: bool) -> io::Result<Counts> {
    let mut counts = Counts::default();
    let mut buf = vec![0u8; 256 * 1024];
    let mut in_word = false;

    loop {
        let n = reader.read(&mut buf)?;
        if n == 0 { break; }
        let chunk = &buf[..n];
        counts.bytes += n as u64;
        counts.lines += memchr::memchr_iter(b'\n', chunk).count() as u64;
        for &b in chunk {
            if need_words {
                update_word_state(b, &mut in_word, &mut counts.words);
            }
            if need_chars && (b & 0xC0) != 0x80 {
                counts.chars += 1;
            }
        }
    }
    Ok(counts)
}

fn count_full_reader<R: Read>(mut reader: R, flags: CountFlags) -> io::Result<Counts> {
    let mut counts = Counts::default();
    let mut buf = vec![0u8; 256 * 1024];
    let mut in_word = false;
    let mut current_line_len: u64 = 0;
    let need_words = flags.needs_words();
    let need_chars = flags.needs_chars();
    let need_max_line = flags.needs_max_line_len();

    loop {
        let n = reader.read(&mut buf)?;
        if n == 0 { break; }
        counts.bytes += n as u64;
        let chunk = &buf[..n];
        let mut i = 0;
        while i < chunk.len() {
            let b = chunk[i];
            if b == b'\n' {
                counts.lines += 1;
                if need_max_line {
                    if current_line_len > counts.max_line_len {
                        counts.max_line_len = current_line_len;
                    }
                    current_line_len = 0;
                }
                if need_words { update_word_state(b, &mut in_word, &mut counts.words); }
                if need_chars { counts.chars += 1; }
                i += 1;
            } else if b == b'\t' {
                if need_max_line { current_line_len = (current_line_len + 8) & !7; }
                if need_words { update_word_state(b, &mut in_word, &mut counts.words); }
                i += 1;
            } else if b == b'\r' {
                if need_max_line { current_line_len = 0; }
                if need_words { update_word_state(b, &mut in_word, &mut counts.words); }
                i += 1;
            } else if b < 0x20 {
                // control chars: no display width, but still count for words/chars
                if need_words { update_word_state(b, &mut in_word, &mut counts.words); }
                if need_chars { counts.chars += 1; }
                i += 1;
            } else if b < 0x80 {
                if need_max_line { current_line_len += 1; }
                if need_words { update_word_state(b, &mut in_word, &mut counts.words); }
                if need_chars { counts.chars += 1; }
                i += 1;
            } else {
                // multibyte: decode for display width (best-effort across buffer boundaries)
                let remaining = &chunk[i..];
                let (ch, len) = decode_char_at(remaining, 0);
                if let Some(c) = ch {
                    if need_max_line { current_line_len += char_display_width(c); }
                    if need_chars { counts.chars += 1; }
                } else if need_chars && (b & 0xC0) != 0x80 {
                    // invalid lead byte still counts as a char position
                    // (matches non-continuation-byte heuristic)
                }
                // word state for each byte in the sequence
                for j in 0..len {
                    if need_words && i + j < chunk.len() {
                        update_word_state(chunk[i + j], &mut in_word, &mut counts.words);
                    }
                }
                i += len;
            }
        }
    }

    if need_max_line && current_line_len > counts.max_line_len {
        counts.max_line_len = current_line_len;
    }
    Ok(counts)
}

fn count_lines_bytes_reader<R: Read>(mut reader: R) -> io::Result<Counts> {
    let mut counts = Counts::default();
    let mut buf = vec![0u8; 256 * 1024];
    loop {
        let n = reader.read(&mut buf)?;
        if n == 0 { break; }
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
        CountFlags { lines: true, words: true, bytes: true, chars: true, max_line_len: true }
    }

    fn chars_only() -> CountFlags {
        CountFlags { lines: false, words: false, bytes: false, chars: true, max_line_len: false }
    }

    #[test]
    fn empty() {
        let c = count_str("", all_flags());
        assert_eq!((c.lines, c.words, c.bytes, c.chars, c.max_line_len), (0, 0, 0, 0, 0));
    }

    #[test]
    fn single_line_no_newline() {
        let c = count_str("hello world", all_flags());
        assert_eq!((c.lines, c.words, c.bytes, c.chars, c.max_line_len), (0, 2, 11, 11, 11));
    }

    #[test]
    fn single_line_with_newline() {
        let c = count_str("hello world\n", all_flags());
        assert_eq!((c.lines, c.words, c.bytes, c.chars, c.max_line_len), (1, 2, 12, 12, 11));
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
        let c = count_str("cafe\u{0301}\n", all_flags());
        assert_eq!((c.lines, c.words, c.bytes, c.chars), (1, 1, 7, 6));
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

    #[test]
    fn chars_only_via_reader() {
        let c = count_str("hello\n", chars_only());
        assert_eq!(c.chars, 6);
    }

    #[test]
    fn combining_mark_display_width() {
        let c = count_str("a\u{0301}\n", all_flags());
        assert_eq!(c.max_line_len, 1); // combining accent is width 0
    }

    #[test]
    fn wide_char_display_width() {
        let c = count_str("\u{754C}\n", all_flags()); // 界 is width 2
        assert_eq!(c.max_line_len, 2);
    }
}
