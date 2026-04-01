use std::path::Path;
use std::process::Command;
use std::{env, fs};

fn main() {
    println!("cargo:rerun-if-changed=hoon/app/app.hoon");
    println!("cargo:rerun-if-changed=hoon/lib/vesl-graft.hoon");
    println!("cargo:rerun-if-changed=hoon/lib/vesl-logic.hoon");
    println!("cargo:rerun-if-changed=hoon/sur/vesl.hoon");

    let out_dir = env::var("OUT_DIR").unwrap();
    let hoon_app_file = "hoon/app/app.hoon";

    if Path::new(hoon_app_file).exists() {
        let nock_home = env::var("NOCK_HOME").ok();
        let mut args = vec![
            hoon_app_file.to_string(),
            "--output".to_string(),
            format!("{}/app.nock", out_dir),
        ];
        if let Some(ref home) = nock_home {
            args.push(format!("{}/hoon/", home));
        }

        let output = Command::new("hoonc")
            .args(&args)
            .output();

        match output {
            Ok(result) => {
                if !result.status.success() {
                    panic!(
                        "Failed to compile Hoon: {}",
                        String::from_utf8_lossy(&result.stderr)
                    );
                }
                println!("cargo:rustc-env=COMPILED_HOON_PATH={}/app.nock", out_dir);
            }
            Err(e) => {
                println!(
                    "cargo:warning=Could not run hoonc: {}. Using pre-compiled out.jam.",
                    e
                );
            }
        }
    }
}
