use std::env;
use std::path::PathBuf;
use std::process::ExitCode;

use sf_conformance::inventory;

#[derive(Clone, Copy)]
enum Mode {
    Check,
    Generate,
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("rdb2rdf-inventory: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<(), String> {
    let suite_default = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/w3c/rdb2rdf");
    let mut suite = suite_default.clone();
    let mut inventory_path = None;
    let mut mode = None;
    let mut args = env::args().skip(1);
    while let Some(argument) = args.next() {
        match argument.as_str() {
            "--check" => set_mode(&mut mode, Mode::Check)?,
            "--generate" => set_mode(&mut mode, Mode::Generate)?,
            "--suite" => {
                suite = PathBuf::from(
                    args.next()
                        .ok_or_else(|| "--suite requires a path".to_owned())?,
                );
            }
            "--inventory" => {
                inventory_path = Some(PathBuf::from(
                    args.next()
                        .ok_or_else(|| "--inventory requires a path".to_owned())?,
                ));
            }
            "--help" | "-h" => {
                println!(
                    "Usage: rdb2rdf-inventory (--check | --generate) \
                     [--suite PATH] [--inventory PATH]"
                );
                return Ok(());
            }
            _ => return Err(format!("unknown argument {argument:?}")),
        }
    }
    let mode = mode.ok_or_else(|| "choose exactly one of --check or --generate".to_owned())?;
    let inventory_path = inventory_path.unwrap_or_else(|| suite.join("inventory.tsv"));
    match mode {
        Mode::Check => {
            let sealed = inventory::check(&suite, &inventory_path)?;
            println!(
                "verified 1 suite manifest, {} scenarios, {} cases, {} case-tree files",
                sealed.scenarios.len(),
                sealed.cases.len(),
                sealed.files.len()
            );
        }
        Mode::Generate => {
            inventory::write_generated(&suite, &inventory_path)?;
            println!("generated {}", inventory_path.display());
        }
    }
    Ok(())
}

fn set_mode(target: &mut Option<Mode>, candidate: Mode) -> Result<(), String> {
    if target.is_some() {
        return Err("choose exactly one of --check or --generate".to_owned());
    }
    *target = Some(candidate);
    Ok(())
}
