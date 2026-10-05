use std::path::PathBuf;
use std::process::exit;

use clap::{Parser, ValueEnum};
use dbc_codegen::{Config, InputEncoding, RoundingPolicy, decode_input};

#[derive(Clone, Copy, Debug, ValueEnum)]
enum Encoding {
    Utf8,
    Windows1252,
    Cp949,
}

#[derive(Clone, Copy, Debug, ValueEnum)]
enum Rounding {
    Truncate,
    NearestAway,
    Exact,
}

/// Generate Rust `struct`s from a `dbc` file.
#[derive(Debug, Parser)]
#[command(version)]
struct Cli {
    /// Path to a `.dbc` file
    dbc_path: PathBuf,

    /// Target directory to write Rust source file(s) to
    out_path: PathBuf,

    /// Enable debug printing
    #[arg(long)]
    debug: bool,

    /// Explicit encoding (no guessing or replacement characters)
    #[arg(long, value_enum, default_value = "utf8")]
    encoding: Encoding,

    /// Generate transmit and receive messages for this node (repeatable)
    #[arg(long = "node")]
    nodes: Vec<String>,

    /// Use f64 physical values for integer wire signals
    #[arg(long)]
    physical_f64: bool,

    /// Integer wire quantization policy
    #[arg(long, value_enum, default_value = "truncate")]
    rounding: Rounding,
}

fn main() {
    let args = Cli::parse();
    let bytes = std::fs::read(&args.dbc_path).unwrap_or_else(|e| {
        eprintln!("could not read `{}`: {e}", args.dbc_path.display());
        exit(exitcode::NOINPUT);
    });
    let encoding = match args.encoding {
        Encoding::Utf8 => InputEncoding::Utf8,
        Encoding::Windows1252 => InputEncoding::Windows1252,
        Encoding::Cp949 => InputEncoding::Cp949,
    };
    let dbc_file = decode_input(&bytes, encoding).unwrap_or_else(|e| {
        eprintln!("could not decode `{}`: {e:#}", args.dbc_path.display());
        exit(exitcode::NOINPUT);
    });
    let dbc_file_name = args
        .dbc_path
        .file_name()
        .unwrap_or_else(|| args.dbc_path.as_ref())
        .to_string_lossy();

    if !args.out_path.is_dir() {
        eprintln!(
            "Output path needs to point to a directory (checked {})",
            args.out_path.display()
        );
        exit(exitcode::CANTCREAT);
    }

    let source_path = args.dbc_path.canonicalize().unwrap_or_else(|e| {
        eprintln!("could not resolve input path: {e}");
        exit(exitcode::NOINPUT);
    });
    for name in ["messages.rs", "manifest.json"] {
        if args
            .out_path
            .join(name)
            .canonicalize()
            .is_ok_and(|p| p == source_path)
        {
            eprintln!("output `{name}` would replace the source file");
            exit(exitcode::CANTCREAT);
        }
    }

    let nodes: Vec<_> = args.nodes.iter().map(String::as_str).collect();

    if let Err(e) = Config::builder()
        .dbc_name(&dbc_file_name)
        .dbc_content(&dbc_file)
        .debug_prints(args.debug)
        .selected_nodes(&nodes)
        .physical_f64(args.physical_f64)
        .rounding(match args.rounding {
            Rounding::Truncate => RoundingPolicy::Truncate,
            Rounding::NearestAway => RoundingPolicy::NearestAway,
            Rounding::Exact => RoundingPolicy::Exact,
        })
        .build()
        .generate_artifacts(&bytes, encoding)
        .and_then(|artifacts| artifacts.write_to_directory(&args.out_path))
    {
        eprintln!("could not convert `{}`: {e:#}", args.dbc_path.display());
        if args.debug {
            eprintln!("details: {e:?}");
        }
        exit(exitcode::NOINPUT)
    }
}
