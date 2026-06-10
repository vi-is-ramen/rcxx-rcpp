use anyhow::Result;
use std::collections::HashMap;
use std::fs;
use std::path::Path;
use crate::parser::{Attribute, InterfaceBlock, RestCode, UseStatement};

#[derive(Debug, Clone)]
pub struct ModuleInfo {
    pub _namespace: String,
    pub hash: String,
}

pub struct ModuleRegistry {
    pub modules: HashMap<String, ModuleInfo>,
}

impl ModuleRegistry {
    pub fn new() -> Self { Self { modules: HashMap::new() } }
    pub fn register(&mut self, name: String, namespace: String, hash: String) {
        self.modules.insert(name, ModuleInfo { _namespace: namespace, hash });
    }
    pub fn get(&self, name: &str) -> Option<&ModuleInfo> {
        self.modules.get(name)
    }
}

pub struct SourceMap {
    pub entries: Vec<(String, usize, String, usize)>,
}

impl SourceMap {
    pub fn new() -> Self { Self { entries: Vec::new() } }
    pub fn add(&mut self, src_file: &str, src_line: usize, dst_file: &str, dst_line: usize) {
        self.entries.push((src_file.to_string(), src_line, dst_file.to_string(), dst_line));
    }
    pub fn write_csv(&self, path: &Path) -> Result<()> {
        let mut csv = String::from("src_file,src_line,dst_file,dst_line\n");
        for (sf, sl, df, dl) in &self.entries {
            csv.push_str(&format!("{},{},{},{}\n", sf, sl, df, dl));
        }
        fs::write(path, csv)?;
        Ok(())
    }
}

fn hash_module_path(path: &str) -> String {
    const FNV_PRIME: u64 = 0x100000001b3;
    const FNV_OFFSET: u64 = 0xcbf29ce484222325;
    let mut hash = FNV_OFFSET;
    for byte in path.bytes() {
        hash ^= byte as u64;
        hash = hash.wrapping_mul(FNV_PRIME);
    }
    format!("{:016x}", hash)
}

fn evaluate_cfg(attrs: &[Attribute], build_cfg: &HashMap<String, String>) -> bool {
    for attr in attrs {
        if attr.name == "cfg" {
            for (k, v) in &attr.args {
                if build_cfg.get(k) != Some(v) {
                    return false;
                }
            }
        }
    }
    true
}

fn translate_attrs(attrs: &[Attribute], build_cfg: &HashMap<String, String>) -> (String, bool) {
    if !evaluate_cfg(attrs, build_cfg) {
        return (String::new(), false);
    }
    let mut cpp_attrs = Vec::new();
    for attr in attrs {
        match attr.name.as_str() {
            "no_mangle" => cpp_attrs.push("extern \"C\"".to_string()),
            "inline" => cpp_attrs.push("inline".to_string()),
            "export_name" => {
                if let Some(name) = attr.args.iter().find(|(k, _)| k == "name").map(|(_, v)| v) {
                    cpp_attrs.push(format!("[[gnu::alias(\"{}\")]]", name));
                }
            }
            "section" => {
                if let Some(sec) = attr.args.iter().find(|(k, _)| k == "name").map(|(_, v)| v) {
                    cpp_attrs.push(format!("[[gnu::section(\"{}\")]]", sec));
                }
            }
            _ => {}
        }
    }
    if cpp_attrs.is_empty() {
        (String::new(), true)
    } else {
        (cpp_attrs.join(" ") + " ", true)
    }
}

fn mask_literals(code: &str) -> (String, Vec<String>) {
    let mut masked = String::with_capacity(code.len());
    let mut literals = Vec::new();
    let mut chars = code.chars().peekable();

    while let Some(c) = chars.next() {
        if c == '"' {
            let mut lit = String::from('"');
            while let Some(&nc) = chars.peek() {
                lit.push(chars.next().unwrap());
                if nc == '"' && !lit.ends_with("\\\"") {
                    break;
                }
            }
            literals.push(lit.clone());
            masked.push_str(&format!("\x00LIT_{}\x00", literals.len() - 1));
        } else if c == '\'' {
            let mut lit = String::from('\'');
            while let Some(&nc) = chars.peek() {
                lit.push(chars.next().unwrap());
                if nc == '\'' && !lit.ends_with("\\\'") {
                    break;
                }
            }
            literals.push(lit.clone());
            masked.push_str(&format!("\x00LIT_{}\x00", literals.len() - 1));
        } else if c == 'R' && chars.peek() == Some(&'"') {
            let mut lit = String::from("R\"");
            chars.next();
            while let Some(&nc) = chars.peek() {
                lit.push(chars.next().unwrap());
                if nc == '"' && lit.len() > 3 {
                    break;
                }
            }
            literals.push(lit.clone());
            masked.push_str(&format!("\x00LIT_{}\x00", literals.len() - 1));
        } else {
            masked.push(c);
        }
    }
    (masked, literals)
}

fn apply_rcpp_syntax(content: &str, module_name: &str) -> String {
    let (masked, literals) = mask_literals(content);
    let mut res = masked;

    res = regex::Regex::new(r"\blet(\s+[a-zA-Z_][a-zA-Z0-9_]*)")
        .unwrap()
        .replace_all(&res, "const auto$1")
        .to_string();

    res = regex::Regex::new(r"\bmut(\s+[a-zA-Z_][a-zA-Z0-9_]*)")
        .unwrap()
        .replace_all(&res, "auto$1")
        .to_string();

    // fn name(args) -> Type;  ->  Type name(args);
    res = regex::Regex::new(r"\bfn\s+(\w+)\s*\(([^)]*)\)\s*->\s*(\w+)\s*;")
        .unwrap()
        .replace_all(&res, "${3} ${1}(${2});")
        .to_string();

    // fn name(args) -> Type {  ->  Type name(args) {
    res = regex::Regex::new(r"\bfn\s+([a-zA-Z_][a-zA-Z0-9_:]*)\s*\(([^)]*)\)\s*->\s*([a-zA-Z_][a-zA-Z0-9_:]*)\s*(\{)")
        .unwrap()
        .replace_all(&res, "${3} ${1}(${2}) ${4}")
        .to_string();

    // НОВОЕ: fn name(args) {  ->  auto name(args) { (без явного типа возврата)
    res = regex::Regex::new(r"\bfn\s+([a-zA-Z_][a-zA-Z0-9_:]*)\s*\(([^)]*)\)\s*(\{)")
        .unwrap()
        .replace_all(&res, "auto ${1}(${2}) ${3}")
        .to_string();

    res = replace_panic(&res, module_name);

    for (i, lit) in literals.into_iter().enumerate() {
        res = res.replace(&format!("\x00LIT_{}\x00", i), &lit);
    }

    res
}

fn replace_panic(code: &str, module_name: &str) -> String {
    let mut result = String::with_capacity(code.len() + 64);
    let mut chars = code.chars().peekable();
    
    while let Some(c) = chars.next() {
        if code.chars().skip(result.len()).collect::<String>().starts_with("panic!") {
            let prev_char = result.chars().last();
            let is_word_bound = prev_char.map_or(true, |pc| !pc.is_alphanumeric() && pc != '_');
            
            if is_word_bound {
                for _ in 0..5 { chars.next(); } 
                while let Some(&' ') = chars.peek() { chars.next(); }
                
                if chars.peek() == Some(&'(') {
                    result.push_str(&format!("__panic_here(__FILE__, __LINE__, \"{}\", ", module_name));
                    chars.next();
                    
                    let mut depth = 1;
                    let mut args = String::new();
                    while let Some(nc) = chars.next() {
                        if nc == '(' { depth += 1; }
                        else if nc == ')' { depth -= 1; }
                        if depth == 0 { break; }
                        args.push(nc);
                    }
                    result.push_str(&args);
                    result.push(')');
                    continue;
                }
            }
        }
        result.push(c);
    }
    result
}

fn resolve_namespace_path(path: &str, root_module: Option<&str>) -> String {
    if let Some(root) = root_module {
        if !path.starts_with("::") && !path.starts_with(&format!("{}::", root)) && path != root {
            return format!("{}::{}", root, path);
        }
    }
    path.to_string()
}

fn resolve_include_hash(path: &str, root_module: Option<&str>, registry: &ModuleRegistry, output_dir: &str) -> String {
    let ns = resolve_namespace_path(path, root_module);
    let base_name = path.split("::").last().unwrap_or(path);
    
    let hash = if let Some(info) = registry.get(base_name) {
        info.hash.clone()
    } else {
        hash_module_path(&ns)
    };
    
    // Формируем абсолютный путь
    let abs_path = Path::new(output_dir).join(format!("{}.hpp", hash));
    abs_path.to_string_lossy().to_string()
}

pub fn generate_hpp(
    namespace_path: &str,
    hashed_name: &str,
    interface: &InterfaceBlock,
    uses: &[UseStatement],
    root_module: Option<&str>,
    registry: &ModuleRegistry,
    output_dir: &str,
    sm: &mut SourceMap,
    build_cfg: &HashMap<String, String>,
) -> Result<String> {
    let mut out = Vec::new();
    out.push(format!("// Generated from {}", interface.file));
    out.push("#pragma once\n".to_string());
    
    // Все use-выражения (и сверху, и внутри interface) добавят свои #include
    for u in uses {
        let inc_path = resolve_include_hash(&u.path, root_module, registry, output_dir);
        out.push(format!("#include \"{}\"", inc_path));
    }
    out.push("".to_string());

    let (attr_str, include) = translate_attrs(&interface.attrs, build_cfg);
    if !include { return Ok(out.join("\n")); }

    out.push(format!("namespace {} {{\n", namespace_path));
    
    if let Some(first) = interface.lines.first() {
        out.push(format!("#line {} \"{}\"", first.line_num, first.file));
        sm.add(&first.file, first.line_num, &format!("{}.hpp", hashed_name), out.len());
    }

    for line in &interface.lines {
        let (l_attr, l_inc) = translate_attrs(&line.attrs, build_cfg);
        if !l_inc { continue; }
        
        // apply_rcpp_syntax корректно обработает fn, let, mut и panic! внутри интерфейса
        let content = apply_rcpp_syntax(&line.content, namespace_path);
        out.push(format!("{}{}", l_attr, content));
    }

    out.push(format!("\n}} // namespace {}", namespace_path));
    Ok(out.join("\n"))
}

pub fn generate_cpp(
    namespace_path: &str,
    hashed_name: &str,
    rest: &RestCode,
    uses: &[UseStatement],
    root_module: Option<&str>,
    registry: &ModuleRegistry,
    output_dir: &str,
    sm: &mut SourceMap,
    build_cfg: &HashMap<String, String>,
) -> Result<String> {
    let mut out = Vec::new();
    out.push(format!("// Generated from {}.rcx", namespace_path));
    
    // Абсолютный путь к своему .hpp
    let own_hpp = Path::new(output_dir).join(format!("{}.hpp", hashed_name));
    out.push(format!("#include \"{}\"\n", own_hpp.to_string_lossy()));

    for u in uses {
        let inc_path = resolve_include_hash(&u.path, root_module, registry, output_dir);
        out.push(format!("#include \"{}\"", inc_path));
        
        let resolved_ns = resolve_namespace_path(&u.path, root_module);
        
        // ПРАВИЛЬНАЯ ТРАНСЛЯЦИЯ use
        if u.items == vec!["*"] {
            // use person::*;  ->  using namespace example::person;
            out.push(format!("using namespace {};", resolved_ns));
        } else if u.items.is_empty() {
            // use person;  ->  (ничего, только include)
        } else {
            // use person::Person;  ->  using example::person::Person;
            for item in &u.items {
                out.push(format!("using {}::{};", resolved_ns, item));
            }
        }
    }
    out.push("".to_string());
    out.push(format!("namespace {} {{\n", namespace_path));

    let mut first_line_added = false;
    for line in &rest.lines {
        let (l_attr, l_inc) = translate_attrs(&line.attrs, build_cfg);
        if !l_inc { continue; }

        if !first_line_added && !line.content.trim().is_empty() {
            out.push(format!("#line {} \"{}\"", line.line_num, line.file));
            sm.add(&line.file, line.line_num, &format!("{}.cpp", hashed_name), out.len());
            first_line_added = true;
        }

        let content = apply_rcpp_syntax(&line.content, namespace_path);
        out.push(format!("{}{}", l_attr, content));
    }

    out.push(format!("\n}} // namespace {}", namespace_path));
    Ok(out.join("\n"))
}

pub fn register_module(
    rcx_path: &str, 
    root_module: Option<&str>,
    registry: &mut ModuleRegistry,
) -> Result<()> {
    let path = std::path::Path::new(rcx_path);
    
    let rcx_path_normalized = rcx_path.replace('\\', "/");
    let relative_path = if let Some(idx) = rcx_path_normalized.find("/src/") {
        let rel = &rcx_path_normalized[idx + 5..];
        rel.trim_end_matches(".rcx").to_string()
    } else {
        path.file_stem().unwrap().to_str().unwrap().to_string()
    };

    let namespace_path = if let Some(root) = root_module {
        if relative_path == "_" || relative_path.is_empty() {
            root.to_string()
        } else {
            format!("{}::{}", root, relative_path.replace('/', "::"))
        }
    } else {
        relative_path.replace('/', "::")
    };

    let hashed_name = hash_module_path(&namespace_path);
    
    let module_name_for_registry = if let Some(root) = root_module {
        if relative_path == "_" || relative_path.is_empty() {
            root.to_string()
        } else {
            relative_path.split('/').last().unwrap_or(root).to_string()
        }
    } else {
        relative_path.split('/').last().unwrap_or(&relative_path).to_string()
    };

    registry.register(module_name_for_registry, namespace_path, hashed_name);
    Ok(())
}

pub fn generate_module_files(
    rcx_path: &str, 
    output_dir: &str, 
    root_module: Option<&str>, 
    build_cfg: &HashMap<String, String>,
    registry: &ModuleRegistry,
    cli_externs: &HashMap<String, String>,
) -> Result<()> {
    let code = fs::read_to_string(rcx_path)?;
    let path = std::path::Path::new(rcx_path);
    
    let rcx_path_normalized = rcx_path.replace('\\', "/");
    let relative_path = if let Some(idx) = rcx_path_normalized.find("/src/") {
        let rel = &rcx_path_normalized[idx + 5..];
        rel.trim_end_matches(".rcx").to_string()
    } else {
        path.file_stem().unwrap().to_str().unwrap().to_string()
    };

    let namespace_path = if let Some(root) = root_module {
        if relative_path == "_" || relative_path.is_empty() {
            root.to_string()
        } else {
            format!("{}::{}", root, relative_path.replace('/', "::"))
        }
    } else {
        relative_path.replace('/', "::")
    };

    let hashed_name = hash_module_path(&namespace_path);
    let out_dir_path = Path::new(output_dir);
    fs::create_dir_all(out_dir_path)?;

    let (uses, externs, interface, rest) = crate::parser::parse_file(&code, rcx_path)?;
    let mut sm = SourceMap::new();
    let mut dependencies = Vec::new();

    for ext in &externs {
        let resolved_path = if let Some(cli_path) = cli_externs.get(&ext.name) {
            cli_path.clone()
        } else if let Some(inline) = &ext.inline_path {
            inline.clone()
        } else {
            anyhow::bail!("Extern module '{}' has no path. Provide via inline \"path\" or CLI -e {},<path>", ext.name, ext.name);
        };
        dependencies.push(ext.name.clone());
        
        let target_check = if std::path::Path::new(&resolved_path).is_dir() {
            format!("{}/src", resolved_path)
        } else {
            resolved_path
        };
        if !std::path::Path::new(&target_check).exists() {
            anyhow::bail!("Extern module path does not exist: {}", target_check);
        }
    }

    for u in &uses {
        let base_name = u.path.split("::").last().unwrap_or(&u.path).to_string();
        dependencies.push(base_name);
    }

    // ВСЕГДА генерируем .hpp, даже если interface пустой
    let hpp = if let Some(ref iface) = interface {
        generate_hpp(&namespace_path, &hashed_name, iface, &uses, root_module, registry, output_dir, &mut sm, build_cfg)?
    } else {
        // Создаем пустой заголовок с namespace
        let mut out = Vec::new();
        out.push(format!("// Generated from {}.rcx", namespace_path));
        out.push("#pragma once\n".to_string());
        
        for u in &uses {
            let inc_path = resolve_include_hash(&u.path, root_module, registry, output_dir);
            out.push(format!("#include \"{}\"", inc_path));
            // Убираем using из заголовка, он должен быть только в .cpp
        }
        out.push("".to_string());
        out.push(format!("namespace {} {{", namespace_path));
        out.push(format!("}} // namespace {}", namespace_path));
        
        out.join("\n")
    };
    
    fs::write(out_dir_path.join(format!("{}.hpp", hashed_name)), hpp)?;

    let cpp = generate_cpp(&namespace_path, &hashed_name, &rest, &uses, root_module, registry, output_dir, &mut sm, build_cfg)?;
    fs::write(out_dir_path.join(format!("{}.cpp", hashed_name)), cpp)?;

    sm.write_csv(&out_dir_path.join(format!("{}.csv", hashed_name)))?;

    let mut lst_content = String::new();
    for dep_name in dependencies {
        if let Some(dep_info) = registry.get(&dep_name) {
            lst_content.push_str(&format!("{}\n", dep_info.hash));
        }
    }
    fs::write(out_dir_path.join(format!("{}.lst", hashed_name)), lst_content)?;

    println!("✅ {} -> {}/{}.cpp", rcx_path, output_dir, hashed_name);
    Ok(())
}
