mod parser;
mod codegen;
mod builder;

use anyhow::Result;
use clap::Parser;
use std::collections::HashMap;
use std::io::{self, BufRead, Write};

#[derive(Parser, Debug)]
#[command(name = "rcpp")]
#[command(about = "RC++ Preprocessor and Build Driver")]
struct Args {
    /// Project directory
    project_dir: String,

    /// Output directory (default: <project>/build)
    #[arg(short, long)]
    output: Option<String>,

    /// Root module name (for _.rcx)
    #[arg(short = 'r', long)]
    root_module: Option<String>,

    /// External module: name,path or name=path (can be used multiple times)
    #[arg(short = 'e', long = "extern", value_parser = parse_extern_arg)]
    externs: Vec<(String, String)>,

    /// Path to C++ compiler (e.g., clang++, g++)
    #[arg(long)]
    invoke_cc: Option<String>,

    /// Build configuration (e.g., arch="x86_64")
    #[arg(long)]
    cfg: Vec<String>,

    /// Run as JSON-RPC 2.0 server via stdin/stdout
    #[arg(long)]
    json_rpc: bool,

    /// Arguments passed directly to the C++ compiler (must be after --)
    #[arg(last = true)]
    cc_args: Vec<String>,
}

fn parse_extern_arg(s: &str) -> Result<(String, String), String> {
    let parts: Vec<&str> = if s.contains('=') {
        s.splitn(2, '=').collect()
    } else {
        s.splitn(2, ',').collect()
    };
    if parts.len() == 2 {
        Ok((parts[0].trim().to_string(), parts[1].trim().to_string()))
    } else {
        Err("Extern format must be 'name,path' or 'name=path'".to_string())
    }
}

fn main() -> Result<()> {
    let args = Args::parse();
    
    if args.json_rpc {
        run_json_rpc(args)
    } else {
        run_cli(args)
    }
}

fn run_cli(args: Args) -> Result<()> {
    let out_dir = args.output.unwrap_or_else(|| format!("{}/build", args.project_dir));
    
    let mut build_cfg = HashMap::new();
    for cfg_item in args.cfg {
        if let Some(eq) = cfg_item.find('=') {
            build_cfg.insert(cfg_item[..eq].to_string(), cfg_item[eq+1..].trim_matches('"').to_string());
        } else {
            build_cfg.insert(cfg_item.to_string(), "true".to_string());
        }
    }

    let cli_externs: HashMap<String, String> = args.externs.into_iter().collect();

    let mut progress = |pct: usize, msg: &str| println!("[{:3}%] {}", pct, msg);
    
    builder::build_project(
        &args.project_dir, 
        &out_dir, 
        args.root_module.as_deref(), 
        args.invoke_cc.as_deref(), 
        &args.cc_args, 
        &build_cfg, 
        &cli_externs,
        &mut progress
    )?;
    
    println!("✅ Build complete!");
    Ok(())
}

fn run_json_rpc(args: Args) -> Result<()> {
    let stdin = io::stdin();
    let out_dir = args.output.unwrap_or_else(|| "./build".to_string());
    
    let mut build_cfg = HashMap::new();
    for cfg_item in args.cfg {
        if let Some(eq) = cfg_item.find('=') {
            build_cfg.insert(cfg_item[..eq].to_string(), cfg_item[eq+1..].trim_matches('"').to_string());
        } else {
            build_cfg.insert(cfg_item.to_string(), "true".to_string());
        }
    }
    let cli_externs: HashMap<String, String> = args.externs.into_iter().collect();

    for line in stdin.lock().lines() {
        let line = line?;
        if line.trim().is_empty() { continue; }
        
        if let Ok(req) = serde_json::from_str::<serde_json::Value>(&line) {
            let id = req.get("id").cloned();
            let method = req.get("method").and_then(|v| v.as_str()).unwrap_or("");
            
            if method == "build" {
                let mut last_pct = 0;
                let mut progress = |pct: usize, msg: &str| {
                    if pct != last_pct {
                        let notification = serde_json::json!({
                            "jsonrpc": "2.0",
                            "method": "$/progress",
                            "params": {"percent": pct, "message": msg}
                        });
                        println!("{}", notification);
                        io::stdout().flush().unwrap();
                        last_pct = pct;
                    }
                };

                let res = builder::build_project(
                    &args.project_dir, &out_dir, args.root_module.as_deref(),
                    args.invoke_cc.as_deref(), &args.cc_args, &build_cfg, &cli_externs, &mut progress
                );

                let response = match res {
                    Ok(_) => serde_json::json!({"jsonrpc": "2.0", "id": id, "result": {"success": true}}),
                    Err(e) => serde_json::json!({"jsonrpc": "2.0", "id": id, "error": {"code": -32000, "message": e.to_string()}})
                };
                println!("{}", response);
                io::stdout().flush().unwrap();
            }
        }
    }
    Ok(())
}
