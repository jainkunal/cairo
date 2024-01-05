//! Compiles and runs a Cairo program.

use std::fs::File;
use std::io::Write;
use std::path::{Path, PathBuf};

use anyhow::{Context, Ok};
use cairo_lang_compiler::db::RootDatabase;
use cairo_lang_compiler::diagnostics::DiagnosticsReporter;
use cairo_lang_compiler::project::{check_compiler_path, setup_project};
use cairo_lang_diagnostics::ToOption;
use cairo_lang_runner::short_string::as_cairo_short_string;
use cairo_lang_runner::{SierraCasmRunner, StarknetState};
use cairo_lang_sierra_generator::db::SierraGenGroup;
use cairo_lang_sierra_generator::replace_ids::{DebugReplacer, SierraIdReplacer};
use cairo_lang_starknet::contract::get_contracts_info;
use clap::Parser;

/// Command line args parser.
/// Exits with 0/1 if the input is formatted correctly/incorrectly.
#[derive(Parser, Debug)]
#[clap(version, verbatim_doc_comment)]
struct Args {
    /// The file to compile and run.
    path: PathBuf,
    /// Whether path is a single file.
    #[arg(short, long)]
    single_file: bool,
    /// In cases where gas is available, the amount of provided gas.
    #[arg(long)]
    available_gas: Option<usize>,
    /// Whether to print the memory.
    #[arg(long, default_value_t = false)]
    print_full_memory: bool,
}

fn main() -> anyhow::Result<()> {
    let args = Args::parse();

    // Check if args.path is a file or a directory.
    check_compiler_path(args.single_file, &args.path)?;

    let db = &mut RootDatabase::builder().detect_corelib().build()?;

    let main_crate_ids = setup_project(db, Path::new(&args.path))?;

    if DiagnosticsReporter::stderr().check(db) {
        anyhow::bail!("failed to compile: {}", args.path.display());
    }

    let sierra_program = db
        .get_sierra_program(main_crate_ids.clone())
        .to_option()
        .with_context(|| "Compilation failed without any diagnostics.")?;
    let replacer = DebugReplacer { db };
    if args.available_gas.is_none() && sierra_program.requires_gas_counter() {
        anyhow::bail!("Program requires gas counter, please provide `--available-gas` argument.");
    }

    let contracts_info = get_contracts_info(db, main_crate_ids, &replacer)?;

    let runner = SierraCasmRunner::new(
        replacer.apply(&sierra_program),
        if args.available_gas.is_some() { Some(Default::default()) } else { None },
        contracts_info,
    )
    .with_context(|| "Failed setting up runner.")?;

    dbg!(runner.metadata.clone());
    dbg!(sierra_program.as_ref().type_declarations.len());
    dbg!(sierra_program.as_ref().libfunc_declarations.len());

    // Write string to file sierra_program to file
    let mut file = File::create("/Users/kunaljain/Code/cairo/sierra_program.txt").unwrap();
    file.write_all(replacer.apply(&sierra_program).to_string().as_bytes()).unwrap();
    file.flush().unwrap();

    let mut file = File::create("/Users/kunaljain/Code/cairo/sierra_program_no_debug.txt").unwrap();
    file.write_all(sierra_program.to_string().as_bytes()).unwrap();
    file.flush().unwrap();

    let mut file = File::create("/Users/kunaljain/Code/cairo/casm_program.txt").unwrap();
    file.write_all(runner.get_casm_program().to_string().as_bytes()).unwrap();
    file.flush().unwrap();

    // dbg!(&runner.get_casm_program().instructions);

    let mut file = File::create("/Users/kunaljain/Code/cairo/casm_bytecode.txt").unwrap();
    let x: Vec<String> = runner
        .get_casm_program()
        .instructions
        .iter()
        .map(|i| i.assemble().encode())
        .flatten()
        .map(|x| x.to_string())
        .collect();
    file.write_all(x.join("\n").as_bytes()).unwrap();
    file.flush().unwrap();
    // dbg!(&runner.get_casm_program().debug_info);
    dbg!(runner.get_casm_program().instructions.len());
    // for (i, instruction) in runner.get_casm_program().instructions.iter().enumerate() {
    //     dbg!(i);
    //     dbg!(instruction);
    //     // dbg!(instruction.assemble());
    //     // dbg!(instruction.assemble().encode());
    //     if i == 10 {
    //         break;
    //     }
    // }
    dbg!(runner.get_casm_program().debug_info.sierra_statement_info.len());
    let result = runner
        .run_function_with_starknet_context(
            runner.find_function("::main")?,
            &[],
            args.available_gas,
            StarknetState::default(),
        )
        .with_context(|| "Failed to run the function.")?;
    match result.value {
        cairo_lang_runner::RunResultValue::Success(values) => {
            println!("Run completed successfully, returning {values:?}")
        }
        cairo_lang_runner::RunResultValue::Panic(values) => {
            print!("Run panicked with [");
            for value in &values {
                match as_cairo_short_string(value) {
                    Some(as_string) => print!("{value} ('{as_string}'), "),
                    None => print!("{value}, "),
                }
            }
            println!("].")
        }
    }
    if let Some(gas) = result.gas_counter {
        println!("Remaining gas: {gas}");
    }
    if args.print_full_memory {
        print!("Full memory: [");
        for cell in &result.memory {
            match cell {
                None => print!("_, "),
                Some(value) => print!("{value}, "),
            }
        }
        println!("]");
    }
    Ok(())
}
