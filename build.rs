use std::path::{Path, PathBuf};
use std::process::Command;

fn find_sdk_tool(name: &str, env_override: &str) -> Option<PathBuf> {
    if let Ok(p) = std::env::var(env_override) {
        let p = PathBuf::from(p);
        if p.is_file() {
            return Some(p);
        }
    }

    let roots = [
        r"C:\Program Files (x86)\Windows Kits\10\bin",
        r"C:\Program Files\Windows Kits\10\bin",
    ];

    let mut best: Option<(String, PathBuf)> = None;
    for root in roots {
        let Ok(entries) = std::fs::read_dir(root) else { continue };
        for entry in entries.flatten() {
            let version = entry.file_name().to_string_lossy().into_owned();
            if !version.starts_with("10.") {
                continue;
            }
            let candidate = entry.path().join("x64").join(name);
            if candidate.is_file() && best.as_ref().is_none_or(|(b, _)| version > *b) {
                best = Some((version, candidate));
            }
        }
    }

    best.map(|(_, path)| path)
}

fn compile_shaders(out_dir: &Path) {
    println!("cargo:rerun-if-changed=src/shaders/wallpaper.hlsl");
    println!("cargo:rerun-if-changed=src/shaders/transition.hlsl");
    println!("cargo:rerun-if-env-changed=FXC");

    let fxc = find_sdk_tool("fxc.exe", "FXC").expect(
        "fxc.exe nao encontrado. Instale o Windows SDK ou aponte a variavel FXC para o executavel.",
    );

    for (file, entry, profile, output) in [
        ("wallpaper", "vs_main", "vs_5_0", "wallpaper_vs.cso"),
        ("wallpaper", "ps_main", "ps_5_0", "wallpaper_ps.cso"),
        ("wallpaper", "ps_image", "ps_5_0", "image_ps.cso"),
        ("transition", "vs_main", "vs_5_0", "transition_vs.cso"),
        ("transition", "ps_main", "ps_5_0", "transition_ps.cso"),
    ] {
        let source = PathBuf::from(format!("src/shaders/{file}.hlsl"));
        let status = Command::new(&fxc)
            .arg("/nologo")
            .arg("/T")
            .arg(profile)
            .arg("/E")
            .arg(entry)
            .arg("/O3")
            .arg("/Qstrip_debug")
            .arg("/Qstrip_reflect")
            .arg("/Fo")
            .arg(out_dir.join(output))
            .arg(&source)
            .status()
            .unwrap_or_else(|e| panic!("falha ao executar fxc: {e}"));

        if !status.success() {
            panic!("fxc falhou em {entry}/{profile}");
        }
    }
}

fn compile_resources(out_dir: &Path) {
    println!("cargo:rerun-if-changed=assets/walllit.rc");
    println!("cargo:rerun-if-changed=assets/walllit.ico");
    println!("cargo:rerun-if-env-changed=RC");

    let Some(rc) = find_sdk_tool("rc.exe", "RC") else {
        println!("cargo:warning=rc.exe nao encontrado, os executaveis ficarao sem icone");
        return;
    };

    let res = out_dir.join("walllit.res");
    let status = Command::new(&rc)
        .arg("/nologo")
        .arg("/fo")
        .arg(&res)
        .arg("assets/walllit.rc")
        .status()
        .unwrap_or_else(|e| panic!("falha ao executar rc: {e}"));

    if !status.success() {
        panic!("rc.exe falhou ao compilar assets/walllit.rc");
    }

    println!("cargo:rustc-link-arg-bins={}", res.display());
}

fn main() {
    let out_dir = PathBuf::from(std::env::var("OUT_DIR").unwrap());
    compile_shaders(&out_dir);
    compile_resources(&out_dir);
}
