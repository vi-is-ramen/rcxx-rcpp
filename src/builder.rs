use anyhow::{Result, Context};
use std::collections::HashMap;
use std::path::Path;
use std::process::Command;
use crate::codegen::{ModuleRegistry, register_module, generate_module_files};

pub fn build_project(
    project_dir: &str,
    output_dir: &str,
    root_module: Option<&str>,
    cc_path: Option<&str>,
    cc_args: &[String],
    build_cfg: &HashMap<String, String>,
    cli_externs: &HashMap<String, String>,
    on_progress: &mut dyn FnMut(usize, &str),
) -> Result<()> {
    let mut registry = ModuleRegistry::new();
    let mut all_rcx_files = Vec::new();

    for (name, ext_path) in cli_externs {
        let src_dir = Path::new(ext_path).join("src");
        if !src_dir.exists() {
            anyhow::bail!("Extern module '{}' source directory not found: {:?}", name, src_dir);
        }
        for entry in walkdir::WalkDir::new(&src_dir) {
            let entry = entry?;
            if entry.file_name().to_str().unwrap_or("").ends_with(".rcx") {
                all_rcx_files.push((entry.path().to_str().unwrap().to_string(), Some(name.clone())));
            }
        }
    }

    let main_src_dir = Path::new(project_dir).join("src");
    if !main_src_dir.exists() {
        anyhow::bail!("Main source directory not found: {:?}", main_src_dir);
    }

    for entry in walkdir::WalkDir::new(&main_src_dir) {
        let entry = entry?;
        if entry.file_name().to_str().unwrap_or("").ends_with(".rcx") {
            all_rcx_files.push((entry.path().to_str().unwrap().to_string(), None));
        }
    }

    if all_rcx_files.is_empty() {
        anyhow::bail!("No .rcx files found");
    }

    on_progress(5, &format!("Found {} RC++ modules", all_rcx_files.len()));

    for (i, (file, extern_root)) in all_rcx_files.iter().enumerate() {
        let pct = 5 + (i * 20) / all_rcx_files.len();
        let file_name = Path::new(file).file_name().unwrap().to_str().unwrap();
        on_progress(pct, &format!("Registering {}", file_name));
        
        let current_root = if let Some(er) = extern_root {
            Some(er.as_str())
        } else {
            root_module
        };

        register_module(file, current_root, &mut registry)?;
    }

    on_progress(25, "All modules registered");

    // Канонизируем output_dir для абсолютных путей
    let abs_output_dir = if Path::new(output_dir).is_absolute() {
        output_dir.to_string()
    } else {
        std::env::current_dir()?.join(output_dir).to_string_lossy().to_string()
    };

    for (i, (file, extern_root)) in all_rcx_files.iter().enumerate() {
        let pct = 25 + (i * 25) / all_rcx_files.len();
        let file_name = Path::new(file).file_name().unwrap().to_str().unwrap();
        on_progress(pct, &format!("Generating {}", file_name));
        
        let current_root = if let Some(er) = extern_root {
            Some(er.as_str())
        } else {
            root_module
        };

        generate_module_files(file, &abs_output_dir, current_root, build_cfg, &registry, cli_externs)?;
    }

    on_progress(50, "Preprocessing complete");

    if let Some(cc) = cc_path {
        let cpp_files: Vec<_> = std::fs::read_dir(&abs_output_dir)?
            .filter_map(|e| e.ok())
            .filter(|e| e.path().extension().map_or(false, |ext| ext == "cpp"))
            .map(|e| e.path())
            .collect();
        
        let total = cpp_files.len();
        on_progress(60, &format!("Compiling {} files to .o", total));

        for (i, cpp_path) in cpp_files.iter().enumerate() {
            let pct = 60 + (i * 35) / total;
            let file_name = cpp_path.file_name().unwrap().to_str().unwrap();
            on_progress(pct, &format!("Compiling {}", file_name));
            
            let obj_path = cpp_path.with_extension("o");
            
            let mut cmd = Command::new(cc);
            cmd.arg("-c");
            cmd.arg(cpp_path);
            cmd.arg("-o").arg(&obj_path);
            
            for arg in cc_args {
                cmd.arg(arg);
            }

            let output = cmd.output().context(format!("Failed to compile {:?}", cpp_path))?;
            
            if !output.status.success() {
                let stderr = String::from_utf8_lossy(&output.stderr);
                let stdout = String::from_utf8_lossy(&output.stdout);
                anyhow::bail!("Compilation failed for {:?}:\n{}\n{}", cpp_path, stdout, stderr);
            }
        }
        
        on_progress(100, "Compilation successful (object files generated)");
    } else {
        on_progress(100, "Preprocessing successful (no compiler invoked)");
    }

    Ok(())
}
