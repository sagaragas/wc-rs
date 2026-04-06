mod count;

use clap::Parser;
use count::{CountFlags, Counts, count_bytes, count_reader};
use memmap2::Mmap;
use rayon::prelude::*;
use std::fs;
use std::fs::File;
use std::io::{self, BufReader};
use std::process;

#[derive(Parser)]
#[command(
    name = "wc",
    about = "word, line, character, and byte count",
    version,
    after_help = "With no FILE, or when FILE is -, read standard input."
)]
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
        return 7;
    }
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

    if cli.files.is_empty() {
        // Stdin: single-threaded
        match count_reader(io::stdin().lock(), flags) {
            Ok(c) => print_counts(&c, &display_flags, width, None),
            Err(e) => {
                eprintln!("wc: standard input: {}", e);
                process::exit(1);
            }
        }
        return;
    }

    // Check if any file is stdin ("-") -- can't parallelize stdin
    let has_stdin = cli.files.iter().any(|f| f == "-");

    if has_stdin || cli.files.len() == 1 {
        run_sequential(&cli.files, flags, &display_flags, width);
    } else {
        run_parallel(&cli.files, flags, &display_flags, width);
    }
}

fn run_sequential(files: &[String], flags: CountFlags, display_flags: &CountFlags, width: usize) {
    let mut total = Counts::default();
    let mut had_error = false;

    for path in files {
        let result = if path == "-" {
            count_reader(io::stdin().lock(), flags)
        } else {
            count_file(path, flags)
        };

        match result {
            Ok(c) => {
                print_counts(&c, display_flags, width, Some(path));
                total.add(&c);
            }
            Err(e) => {
                eprintln!("wc: {}: {}", path, e);
                had_error = true;
            }
        }
    }

    if files.len() > 1 {
        print_counts(&total, display_flags, width, Some("total"));
    }

    if had_error {
        process::exit(1);
    }
}

fn run_parallel(files: &[String], flags: CountFlags, display_flags: &CountFlags, width: usize) {
    // Count all files in parallel, preserving order
    let results: Vec<(String, Result<Counts, String>)> = files
        .par_iter()
        .map(|path| {
            let result = count_file(path, flags).map_err(|e| format!("{}", e));
            (path.clone(), result)
        })
        .collect();

    // Print in original order (sequential, to match GNU wc output)
    let mut total = Counts::default();
    let mut had_error = false;

    for (path, result) in &results {
        match result {
            Ok(c) => {
                print_counts(c, display_flags, width, Some(path));
                total.add(c);
            }
            Err(e) => {
                eprintln!("wc: {}: {}", path, e);
                had_error = true;
            }
        }
    }

    if files.len() > 1 {
        print_counts(&total, display_flags, width, Some("total"));
    }

    if had_error {
        process::exit(1);
    }
}

fn count_file(path: &str, flags: CountFlags) -> io::Result<Counts> {
    let f = File::open(path)?;
    let meta = f.metadata()?;

    let effective = flags.default_if_none();
    if effective.bytes && !effective.lines && !effective.words
        && !effective.chars && !effective.max_line_len
    {
        return Ok(Counts {
            bytes: meta.len(),
            ..Counts::default()
        });
    }

    if meta.is_file() && meta.len() > 0 {
        let mmap = unsafe { Mmap::map(&f)? };
        return Ok(count_bytes(&mmap, flags));
    }

    let reader = BufReader::with_capacity(256 * 1024, f);
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
