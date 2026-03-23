use std::env;

mod cli;

use crate::cli::{parse_args, Command};

#[tokio::main]
async fn main() {
    match parse_args(env::args().skip(1)) {
        Command::AuditVerify {
            audit_path,
            mirror_path,
        } => match arbiter_server::verify_audit_chain_with_mirror(
            &audit_path,
            mirror_path.as_deref(),
        ) {
            Ok(message) => {
                println!("{message}");
            }
            Err(e) => {
                eprintln!("audit verification failed: {e}");
                std::process::exit(1);
            }
        },
        Command::ConfigValidate { config_path } => {
            match arbiter_config::load_and_validate(&config_path) {
                Ok(_) => println!("config valid: {config_path}"),
                Err(e) => {
                    eprintln!("config invalid: {e}");
                    std::process::exit(1);
                }
            }
        }
        Command::StoreDoctor { config_path } => {
            let cfg = match arbiter_config::load_and_validate(&config_path) {
                Ok(v) => v,
                Err(e) => {
                    eprintln!("failed to load config: {e}");
                    std::process::exit(1);
                }
            };
            match arbiter_server::doctor(cfg).await {
                Ok(lines) => {
                    for line in lines {
                        println!("{line}");
                    }
                }
                Err(e) => {
                    eprintln!("store doctor failed: {e}");
                    std::process::exit(1);
                }
            }
        }
        Command::Serve { config_path } => {
            let cfg = match arbiter_config::load_and_validate(&config_path) {
                Ok(v) => v,
                Err(e) => {
                    eprintln!("failed to load config: {e}");
                    std::process::exit(1);
                }
            };

            if let Err(e) = arbiter_server::serve(cfg).await {
                eprintln!("server exited with error: {e}");
                std::process::exit(1);
            }
        }
        Command::Invalid => {
            eprintln!(
                "Usage: arbiter serve --config <path> | arbiter config-validate [--config <path>] | arbiter audit-verify [--path <path>] [--mirror-path <path>] | arbiter store-doctor [--config <path>]"
            );
            std::process::exit(2);
        }
    }
}
