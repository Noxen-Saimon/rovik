use std::path::Path;
use std::process::Command;

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=rovik.rc");
    println!("cargo:rerun-if-changed=assets/app.ico");

    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let out_dir = std::env::var("OUT_DIR").unwrap();
        let res_path = format!("{out_dir}/rovik.res");

        let mut rc_cmd = None;
        if Command::new("rc.exe").arg("/?").output().is_ok() {
            rc_cmd = Some("rc.exe".to_string());
        } else {
            let possible_paths = [
                r"C:\Program Files (x86)\Windows Kits\10\bin\10.0.26100.0\x64\rc.exe",
                r"C:\Program Files (x86)\Windows Kits\10\bin\10.0.22621.0\x64\rc.exe",
                r"C:\Program Files (x86)\Windows Kits\10\bin\10.0.19041.0\x64\rc.exe",
            ];
            for p in &possible_paths {
                if Path::new(p).exists() {
                    rc_cmd = Some(p.to_string());
                    break;
                }
            }
        }

        if let Some(rc) = rc_cmd {
            let status = Command::new(rc)
                .arg("/fo")
                .arg(&res_path)
                .arg("rovik.rc")
                .status();

            if let Ok(s) = status {
                if s.success() {
                    println!("cargo:rustc-link-arg={res_path}");
                }
            }
        }
    }
}
