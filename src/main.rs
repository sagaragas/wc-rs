mod count;

use clap::Parser;
use count::{CountFlags, Counts, count_reader};
use std::fs;
use std::fs::File;
use std::io::{self, BufReader};
use std::process;

#[derive(Parser)]
#[command(name = "wc", about = "word, line, character, and byte count")]
struct Cli {
    /// Print the newline counts
    #[arg(short = 'l', long = "lines")]
    lines: bool,

    /// Print the word counts
    #[arg(short = 'w', long = "words")]
    words: bool,

    /// Print the byte counts
    #[arg(short = 'c', long = "bytes")]
    bytes: bool,

    /// Print the character counts
    #[arg(short = 'm', long = "chars")]
    chars: bool,

    /// Print the maximum display width
    #[arg(short = 'L', long = "max-line-length")]
    max_line_length: bool,

    /// Files to process
    files: Vec<String>,
}

fn number_width(n: u64) -> usize {
    if n == 0 {
        return 1;
    }
    let mut w = 0;
    let mut v = n;
    while v > 0 {
        w += 1;
        v /= 10;
    }
    w
}

fn compute_field_width(files: &[String], flags: &CountFlags) -> usize {
    if files.is_empty() {
        return 7; // GNU wc default for stdin
    }
    // GNU wc uses file-size-based width only when bytes or chars are displayed
    if !flags.bytes && !flags.chars {
        return 1;
    }
    let mut total_size: u64 = 0;
    for path in files {
        if path == "-" {
            return 7;
        }
        if let Ok(meta) = fs::metadata(path) {
            total_size += meta.len();
        }
    }
    let w = number_width(total_size);
    if w < 1 { 1 } else { w }
}

fn main() {
    let cli = Cli::parse();

    let flags = CountFlags {
        lines: cli.lines,
        words: cli.words,
        bytes: cli.bytes,
        chars: cli.chars,
        max_line_len: cli.max_line_length,
    };
    let display_flags = flags.default_if_none();
    let width = compute_field_width(&cli.files, &display_flags);

    let mut total = Counts::default();
    let mut had_error = false;

    if cli.files.is_empty() {
        match count_reader(io::stdin().lock(), flags) {
            Ok(c) => {
                print_counts(&c, &display_flags, width, None);
                total.add(&c);
            }
            Err(e) => {
                eprintln!("wc: standard input: {}", e);
                had_error = true;
            }
        }
    } else {
        for path in &cli.files {
            let result = if path == "-" {
                count_reader(io::stdin().lock(), flags)
            } else {
                count_file(path, flags)
            };

            match result {
                Ok(c) => {
                    print_counts(&c, &display_flags, width, Some(path));
                    total.add(&c);
                }
                Err(e) => {
                    eprintln!("wc: {}: {}", path, e);
                    had_error = true;
                }
            }
        }

        if cli.files.len() > 1 {
            print_counts(&total, &display_flags, width, Some("total"));
        }
    }

    if had_error {
        process::exit(1);
    }
}

fn count_file(path: &str, flags: CountFlags) -> io::Result<Counts> {
    let f = File::open(path)?;

    let effective = flags.default_if_none();
    if effective.bytes && !effective.lines && !effective.words
        && !effective.chars && !effective.max_line_len
    {
        let meta = f.metadata()?;
        return Ok(Counts {
            bytes: meta.len(),
            ..Counts::default()
        });
    }

    let reader = BufReader::with_capacity(64 * 1024, f);
    count_reader(reader, flags)
}

fn print_counts(counts: &Counts, flags: &CountFlags, width: usize, filename: Option<&str>) {
    let mut first = true;

    if flags.lines {
        print!("{:>w$}", counts.lines, w = width);
        first = false;
    }
    if flags.words {
        if !first { print!(" "); }
        print!("{:>w$}", counts.words, w = width);
        first = false;
    }
    if flags.chars {
        if !first { print!(" "); }
        print!("{:>w$}", counts.chars, w = width);
        first = false;
    }
    if flags.bytes {
        if !first { print!(" "); }
        print!("{:>w$}", counts.bytes, w = width);
        first = false;
    }
    if flags.max_line_len {
        if !first { print!(" "); }
        print!("{:>w$}", counts.max_line_len, w = width);
    }

    match filename {
        Some(name) => println!(" {}", name),
        None => println!(),
    }
}
