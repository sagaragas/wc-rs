use std::process::Command;
use tempfile::NamedTempFile;
use std::io::Write;

const WC_RS: &str = env!("CARGO_BIN_EXE_wc-rs");

fn gnu_wc(args: &[&str]) -> String {
    let out = Command::new("wc")
        .args(args)
        .output()
        .expect("failed to run GNU wc");
    String::from_utf8_lossy(&out.stdout).to_string()
}

fn our_wc(args: &[&str]) -> String {
    let out = Command::new(WC_RS)
        .args(args)
        .output()
        .expect("failed to run wc-rs");
    String::from_utf8_lossy(&out.stdout).to_string()
}

fn make_file(content: &[u8]) -> NamedTempFile {
    let mut f = NamedTempFile::new().unwrap();
    f.write_all(content).unwrap();
    f.flush().unwrap();
    f
}

fn assert_parity(content: &[u8], flag_sets: &[&[&str]]) {
    let f = make_file(content);
    let path = f.path().to_str().unwrap();

    for flags in flag_sets {
        let mut gnu_args: Vec<&str> = flags.to_vec();
        gnu_args.push(path);
        let mut our_args: Vec<&str> = flags.to_vec();
        our_args.push(path);

        let gnu = gnu_wc(&gnu_args);
        let ours = our_wc(&our_args);

        assert_eq!(
            gnu, ours,
            "Mismatch with flags {:?}\nGNU wc: {:?}\nwc-rs:  {:?}",
            flags, gnu, ours
        );
    }
}

#[test]
fn parity_empty_file() {
    assert_parity(b"", &[&[], &["-l"], &["-w"], &["-c"], &["-m"], &["-L"], &["-lwc"]]);
}

#[test]
fn parity_single_line() {
    assert_parity(b"hello world\n", &[&[], &["-l"], &["-w"], &["-c"], &["-m"], &["-L"]]);
}

#[test]
fn parity_no_trailing_newline() {
    assert_parity(b"hello world", &[&[], &["-l"], &["-w"], &["-c"], &["-m"], &["-L"]]);
}

#[test]
fn parity_multiple_lines() {
    assert_parity(
        b"one\ntwo three\nfour\n",
        &[&[], &["-l"], &["-w"], &["-c"], &["-lwc"]],
    );
}

#[test]
fn parity_whitespace_heavy() {
    assert_parity(
        b"  hello   world  \n\n  foo\tbar  \n",
        &[&[], &["-l"], &["-w"], &["-c"], &["-L"]],
    );
}

#[test]
fn parity_tabs() {
    assert_parity(
        b"\thello\t\tworld\n",
        &[&[], &["-L"]],
    );
}

#[test]
fn parity_only_newlines() {
    assert_parity(b"\n\n\n\n\n", &[&[], &["-l"], &["-w"], &["-c"]]);
}

#[test]
fn parity_only_spaces() {
    assert_parity(b"     ", &[&[], &["-l"], &["-w"], &["-c"], &["-L"]]);
}

#[test]
fn parity_binary_ish() {
    assert_parity(
        &[0, 1, 2, 10, 255, 128, 10, 65, 66, 67],
        &[&[], &["-l"], &["-w"], &["-c"]],
    );
}

#[test]
fn parity_large_line() {
    let mut data = vec![b'x'; 10000];
    data.push(b'\n');
    assert_parity(&data, &[&[], &["-L"]]);
}

#[test]
fn parity_multi_file() {
    let f1 = make_file(b"hello\n");
    let f2 = make_file(b"world foo\n");
    let p1 = f1.path().to_str().unwrap();
    let p2 = f2.path().to_str().unwrap();

    let gnu = gnu_wc(&[p1, p2]);
    let ours = our_wc(&[p1, p2]);
    assert_eq!(gnu, ours, "Multi-file mismatch\nGNU: {:?}\nOurs: {:?}", gnu, ours);
}

#[test]
fn parity_utf8() {
    assert_parity(
        "caf\u{e9} na\u{ef}ve\n".as_bytes(),
        &[&[], &["-m"], &["-c"], &["-L"]],
    );
}

#[test]
fn parity_stdin() {
    let input = b"hello world\nfoo bar baz\n";

    let gnu = Command::new("wc")
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .spawn()
        .and_then(|mut child| {
            child.stdin.take().unwrap().write_all(input)?;
            child.wait_with_output()
        })
        .unwrap();

    let ours = Command::new(WC_RS)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .spawn()
        .and_then(|mut child| {
            child.stdin.take().unwrap().write_all(input)?;
            child.wait_with_output()
        })
        .unwrap();

    assert_eq!(
        String::from_utf8_lossy(&gnu.stdout),
        String::from_utf8_lossy(&ours.stdout),
        "stdin parity mismatch"
    );
}

fn stdin_parity(input: &[u8], flags: &[&str]) {
    let gnu = Command::new("wc")
        .args(flags)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .spawn()
        .and_then(|mut child| {
            child.stdin.take().unwrap().write_all(input)?;
            child.wait_with_output()
        })
        .unwrap();

    let ours = Command::new(WC_RS)
        .args(flags)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .spawn()
        .and_then(|mut child| {
            child.stdin.take().unwrap().write_all(input)?;
            child.wait_with_output()
        })
        .unwrap();

    assert_eq!(
        String::from_utf8_lossy(&gnu.stdout),
        String::from_utf8_lossy(&ours.stdout),
        "stdin parity mismatch with flags {:?}",
        flags
    );
}

#[test]
fn parity_stdin_chars() {
    let input = "hello world\n".as_bytes();
    stdin_parity(input, &["-m"]);
    stdin_parity(input, &["-m", "-c"]);
    stdin_parity(input, &["-m", "-l"]);
}

#[test]
fn parity_stdin_single_flags() {
    let input = "hello world\n".as_bytes();
    stdin_parity(input, &["-l"]);
    stdin_parity(input, &["-w"]);
    stdin_parity(input, &["-c"]);
    stdin_parity(input, &["-L"]);
}
