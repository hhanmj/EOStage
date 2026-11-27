use std::fs;
use std::path::{Path, PathBuf};

fn collect_skill_files(base: &Path, cur: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(cur) else {
        return;
    };
    let mut paths: Vec<PathBuf> = entries.flatten().map(|e| e.path()).collect();
    paths.sort();
    for p in paths {
        let Some(name) = p.file_name().and_then(|s| s.to_str()) else {
            continue;
        };
        if name.starts_with('.') {
            continue;
        }
        if p.is_dir() {
            println!("cargo:rerun-if-changed={}", p.display());
            collect_skill_files(base, &p, out);
        } else if p.is_file() {
            println!("cargo:rerun-if-changed={}", p.display());
            if let Ok(rel) = p.strip_prefix(base) {
                out.push(rel.to_path_buf());
            }
        }
    }
}

fn generate_embedded_builtin_skills() {
    let manifest_dir =
        PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR"));
    let skills_root = manifest_dir.join("../ui/public/skills");
    println!("cargo:rerun-if-changed={}", skills_root.display());

    let mut entries: Vec<(String, String)> = Vec::new();
    if let Ok(read_dir) = fs::read_dir(&skills_root) {
        let mut skill_dirs: Vec<PathBuf> = read_dir
            .flatten()
            .map(|e| e.path())
            .filter(|p| {
                p.is_dir()
                    && p.file_name()
                        .and_then(|s| s.to_str())
                        .is_some_and(|n| !n.starts_with('.'))
            })
            .collect();
        skill_dirs.sort();
        for skill_dir in skill_dirs {
            let Some(skill_id) = skill_dir.file_name().and_then(|s| s.to_str()) else {
                continue;
            };
            println!("cargo:rerun-if-changed={}", skill_dir.display());
            let mut rel_files = Vec::new();
            collect_skill_files(&skill_dir, &skill_dir, &mut rel_files);
            for rel in rel_files {
                let rel_str = rel.to_string_lossy().replace('\\', "/");
                if rel_str == "manifest.json" {
                    continue;
                }
                entries.push((skill_id.to_string(), rel_str));
            }
        }
    }

    let out_dir = PathBuf::from(std::env::var("OUT_DIR").expect("OUT_DIR"));
    let mut code = String::from(
        "/// Compile-time embedded built-in skill files for standalone portable packages.\n\
         pub const EMBEDDED_BUILTIN_SKILLS: &[(&str, &str, &str)] = &[\n",
    );
    for (skill_id, rel_path) in entries {
        code.push_str(&format!(
            "    ({:?}, {:?}, include_str!(concat!(env!(\"CARGO_MANIFEST_DIR\"), {:?}))),\n",
            skill_id,
            rel_path,
            format!("/../ui/public/skills/{skill_id}/{rel_path}")
        ));
    }
    code.push_str("];\n");
    fs::write(out_dir.join("embedded_builtin_skills.rs"), code)
        .expect("write embedded_builtin_skills.rs");
}

fn main() {
    tauri_build::build();
    println!("cargo:rerun-if-changed=../contracts/compat-worker-component.v1.json");
    println!("cargo:rerun-if-changed=../ui/dist-tauri/index.html");
    println!("cargo:rerun-if-changed=../ui/dist-tauri/assets");
    generate_embedded_builtin_skills();
}
