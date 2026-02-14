use std::env;

#[tokio::main]
async fn main() {
    let mut args = env::args().skip(1);
    let cmd = args.next().unwrap_or_default();
    if cmd == "audit-verify" {
        let mut audit_path = String::from("./arbiter-audit.jsonl");
        let mut mirror_path: Option<String> = None;
        while let Some(arg) = args.next() {
            if arg == "--path" {
                if let Some(v) = args.next() {
                    audit_path = v;
                }
            }
            if arg == "--mirror-path" {
                if let Some(v) = args.next() {
                    mirror_path = Some(v);
                }
            }
        }
        match arbiter_server::verify_audit_chain_with_mirror(&audit_path, mirror_path.as_deref()) {
            Ok(message) => {
                println!("{message}");
                return;
            }
            Err(e) => {
                eprintln!("audit verification failed: {e}");
                std::process::exit(1);
            }
        }
    }

    if cmd != "serve" {
        eprintln!(
            "Usage: arbiter serve --config <path> | arbiter audit-verify --path <path> [--mirror-path <path>]"
        );
        std::process::exit(2);
    }

    let mut config_path = String::from("./config/example-config.yaml");
    while let Some(arg) = args.next() {
        if arg == "--config" {
            if let Some(v) = args.next() {
                config_path = v;
            }
        }
    }

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
