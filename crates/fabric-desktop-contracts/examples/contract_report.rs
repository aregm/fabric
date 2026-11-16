//! The Rust side of the contract suite (README section 13).
//!
//! ```text
//! contract_report report <contract dir>                 print the report; exit 1 on a disagreement
//! contract_report mutants <contract dir> <out.jsonl>    write the mutants of the accepted messages
//! contract_report judge <contract dir> <mutants.jsonl>  print the judgment of every mutant
//! ```

use std::io::Write;
use std::path::Path;
use std::process::ExitCode;

use fabric_desktop_contracts::suite;

fn print_lines(mut lines: Vec<String>) -> Result<(), String> {
    lines.sort();
    let mut out = String::new();
    for line in lines {
        out.push_str(&line);
        out.push('\n');
    }
    std::io::stdout()
        .write_all(out.as_bytes())
        .map_err(|e| e.to_string())
}

fn run(args: &[String]) -> Result<bool, String> {
    let usage = "usage: contract_report report|mutants|judge <contract dir> [file]";
    let (mode, dir) = match args {
        [mode, dir, ..] => (mode.as_str(), Path::new(dir)),
        _ => return Err(usage.into()),
    };
    match (mode, args.get(2)) {
        ("report", None) => {
            let (lines, problems) = suite::report(dir)?;
            print_lines(lines)?;
            for p in &problems {
                eprintln!("PROBLEM: {p}");
            }
            Ok(problems.is_empty())
        }
        ("mutants", Some(out)) => {
            let mut text = String::new();
            for m in suite::mutants(dir)? {
                text.push_str(&m.to_line());
                text.push('\n');
            }
            std::fs::write(out, text).map_err(|e| format!("{out}: {e}"))?;
            Ok(true)
        }
        ("judge", Some(file)) => {
            let lines = suite::read_mutants(Path::new(file))?
                .iter()
                .map(suite::mutant_line)
                .collect::<Result<Vec<_>, _>>()?;
            print_lines(lines)?;
            Ok(true)
        }
        _ => Err(usage.into()),
    }
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match run(&args) {
        Ok(true) => ExitCode::SUCCESS,
        Ok(false) => ExitCode::FAILURE,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}
