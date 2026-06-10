use anyhow::Result;

#[derive(Debug, Clone)]
pub struct Attribute {
    pub name: String,
    pub args: Vec<(String, String)>,
}

#[derive(Debug, Clone)]
pub struct Line {
    pub content: String,
    pub line_num: usize,
    pub file: String,
    pub attrs: Vec<Attribute>,
}

#[derive(Debug, Clone)]
pub struct UseStatement {
    pub path: String,
    pub items: Vec<String>,
    pub _line: Line,
}

#[derive(Debug, Clone)]
pub struct ExternMod {
    pub name: String,
    pub inline_path: Option<String>,
    pub _line: Line,
}

#[derive(Debug, Clone)]
pub struct InterfaceBlock {
    pub lines: Vec<Line>,
    pub _start_line: usize,
    pub file: String,
    pub attrs: Vec<Attribute>,
}

#[derive(Debug, Clone)]
pub struct RestCode {
    pub lines: Vec<Line>,
}

pub fn parse_file(code: &str, file: &str) -> Result<(Vec<UseStatement>, Vec<ExternMod>, Option<InterfaceBlock>, RestCode)> {
    let mut uses = Vec::new();
    let mut externs = Vec::new();
    let mut interface_lines = Vec::new();
    let mut rest_lines = Vec::new();
    
    let mut in_interface = false;
    let mut brace_depth = 0;
    let mut interface_start_line = 0;
    let mut interface_attrs = Vec::new();
    let mut pending_attrs = Vec::new();

    let extern_re = regex::Regex::new(r#"extern\s+mod\s+([a-zA-Z_][a-zA-Z0-9_]*)(?:\s+"([^"]+)")?\s*;"#).unwrap();

    for (idx, line_str) in code.lines().enumerate() {
        let line_num = idx + 1;
        let content = line_str.trim().to_string();
        
        // НОВОЕ: Удаляем однострочные комментарии для парсинга
        let content_without_comment = if let Some(comment_idx) = content.find("//") {
            content[..comment_idx].trim().to_string()
        } else {
            content.clone()
        };
        
        let mut line = Line {
            content: line_str.to_string(),
            line_num,
            file: file.to_string(),
            attrs: Vec::new(),
        };

        if content_without_comment.starts_with("#[") && content_without_comment.ends_with("]") {
            pending_attrs.push(parse_attribute(&content_without_comment));
            continue;
        }

        if !in_interface && (content_without_comment.is_empty() || content_without_comment.starts_with("//") || content_without_comment.starts_with("/*")) {
            line.attrs = pending_attrs.clone();
            pending_attrs.clear();
            rest_lines.push(line);
            continue;
        }

        if !in_interface && content_without_comment.starts_with("use ") {
            if let Some((path, items)) = parse_use_statement(&content_without_comment) {
                line.attrs = pending_attrs.clone();
                pending_attrs.clear();
                uses.push(UseStatement { path, items, _line: line });
                continue;
            }
        }

        if !in_interface && content_without_comment.starts_with("extern mod ") {
            if let Some(caps) = extern_re.captures(&content_without_comment) {
                line.attrs = pending_attrs.clone();
                pending_attrs.clear();
                externs.push(ExternMod {
                    name: caps[1].to_string(),
                    inline_path: caps.get(2).map(|m| m.as_str().to_string()),
                    _line: line,
                });
                continue;
            }
        }

        if !in_interface && content_without_comment.starts_with("interface") {
            in_interface = true;
            interface_start_line = line_num;
            interface_attrs = pending_attrs.clone();
            pending_attrs.clear();

            let brace_count_open = content_without_comment.matches('{').count();
            let brace_count_close = content_without_comment.matches('}').count();
            brace_depth = brace_count_open.saturating_sub(brace_count_close);

            if brace_depth == 0 {
                let inner = content_without_comment.replace("interface", "").replace('{', "").replace('}', "").trim().to_string();
                if !inner.is_empty() {
                    interface_lines.push(Line { content: inner, line_num, file: file.to_string(), attrs: vec![] });
                }
                in_interface = false;
            } else {
                let inner = content_without_comment.replace("interface", "").replace('{', "").trim().to_string();
                if !inner.is_empty() {
                    interface_lines.push(Line { content: inner, line_num, file: file.to_string(), attrs: vec![] });
                }
            }
            continue;
        }

        if in_interface {
            line.attrs = pending_attrs.clone();
            pending_attrs.clear();
            brace_depth = brace_depth + content_without_comment.matches('{').count() - content_without_comment.matches('}').count();
            
            if brace_depth <= 0 {
                let inner = content_without_comment.replace('}', "").trim().to_string();
                if !inner.is_empty() {
                    interface_lines.push(Line { content: inner, line_num, file: file.to_string(), attrs: line.attrs.clone() });
                }
                in_interface = false;
            } else {
                interface_lines.push(line);
            }
            continue;
        }

        line.attrs = pending_attrs.clone();
        pending_attrs.clear();
        rest_lines.push(line);
    }

    let interface_block = if interface_lines.is_empty() {
        None
    } else {
        Some(InterfaceBlock {
            lines: interface_lines,
            _start_line: interface_start_line,
            file: file.to_string(),
            attrs: interface_attrs,
        })
    };

    Ok((uses, externs, interface_block, RestCode { lines: rest_lines }))
}

fn parse_attribute(s: &str) -> Attribute {
    let inner = s.trim_start_matches("#[").trim_end_matches("]");
    if let Some(paren_idx) = inner.find('(') {
        let name = inner[..paren_idx].trim().to_string();
        let args_str = inner[paren_idx + 1 .. inner.len() - 1].trim();
        let mut args = Vec::new();
        for part in args_str.split(',') {
            let part = part.trim();
            if let Some(eq_idx) = part.find('=') {
                let k = part[..eq_idx].trim().to_string();
                let v = part[eq_idx + 1..].trim().trim_matches('"').trim_matches('\'').to_string();
                args.push((k, v));
            } else if !part.is_empty() {
                args.push((part.to_string(), "true".to_string()));
            }
        }
        Attribute { name, args }
    } else {
        Attribute { name: inner.to_string(), args: vec![] }
    }
}

fn parse_use_statement(s: &str) -> Option<(String, Vec<String>)> {
    let s = s.trim_start_matches("use ").trim_end_matches(';').trim();
    if s.ends_with("::*") {
        Some((s.trim_end_matches("::*").to_string(), vec!["*".to_string()]))
    } else if s.contains("::") {
        let parts: Vec<&str> = s.rsplitn(2, "::").collect();
        Some((parts[1].to_string(), vec![parts[0].to_string()]))
    } else {
        Some((s.to_string(), vec![]))
    }
}
