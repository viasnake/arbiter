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
                "Usage: arbiter serve --config <path> | arbiter audit-verify [--path <path>] [--mirror-path <path>]"
            );
            std::process::exit(2);
        }
    }
}
