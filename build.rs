use std::path::PathBuf;
use std::process::Command;

fn main() {
    println!("cargo:rerun-if-changed=resource.rc");
    println!("cargo:rerun-if-changed=icon.ico");
    
    let out_dir = PathBuf::from(std::env::var("OUT_DIR").unwrap());
    let res_path = out_dir.join("resource.o");
    
    let status = Command::new("windres")
        .arg("resource.rc")
        .arg("-o")
        .arg(&res_path)
        .status();
        
    if let Ok(s) = status {
        if s.success() {
            println!("cargo:rustc-link-arg={}", res_path.display());
        }
    }
}
